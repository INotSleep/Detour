import Foundation
import Network
import SwiftUI

/// What the app shows and holds: the vault's state, its hosts, the open terminals.
@MainActor
final class AppModel: ObservableObject {
    enum Phase { case loading, setup, locked, ready }

    @Published var phase: Phase = .loading
    @Published var sections: [HostSection] = []
    @Published var sync: SyncStatus?
    /// Drives to sign in to again: the vault's sync or a team is kept there.
    @Published var driveSignIns: [DriveProvider] = []
    /// Teams on this person's other devices kept in a folder: opened here through a drive.
    @Published var folderTeams: [PendingTeam] = []
    @Published var terminals: [TerminalSession] = []
    @Published var current: TerminalSession.ID?
    @Published var showTerminals = false
    /// Offer Face ID once, after the first unlock with the password.
    @Published var offerBiometry = false
    /// A `detour://join` link (the other device's QR, scanned with the camera) waiting for the
    /// join form.
    @Published var pendingJoin: JoinLink?
    private var lastPassword: String?
    private let network = NWPathMonitor()
    private var lastNetwork: [Bool]?
    private var background: UIBackgroundTaskIdentifier = .invalid

    init() {
        network.pathUpdateHandler = { [weak self] path in
            let now = [path.status == .satisfied, path.usesInterfaceType(.wifi), path.usesInterfaceType(.cellular), path.usesInterfaceType(.wiredEthernet)]
            DispatchQueue.main.async { MainActor.assumeIsolated { self?.networkChanged(now) } }
        }
        network.start(queue: .global(qos: .utility))
        Core.shared.onEvent { [weak self] name, _ in
            guard let self else { return }
            switch name {
            case "locked": self.phase = .locked
            case "data.changed", "unlocked": if self.phase == .ready { Task { await self.reload() } }
            default: break
            }
        }
    }

    func start() async {
        do {
            let st = try await Core.shared.call("vault_status", as: VaultStatus.self)
            if !st.initialized { phase = .setup } else if !st.unlocked { phase = .locked } else { await ready() }
        } catch {
            phase = .locked
        }
    }

    func create(_ password: String) async throws {
        _ = try await Core.shared.call("vault_create", ["masterPassword": password])
        lastPassword = password
        await ready()
        offerBiometry = Keychain.biometryAvailable
    }

    func unlock(_ password: String, typed: Bool) async throws {
        _ = try await Core.shared.call("vault_unlock", ["masterPassword": password])
        if typed {
            lastPassword = password
            // A password changed on another device: the saved one is replaced on its next use.
            if Keychain.isSaved { _ = Keychain.save(password) } else { offerBiometry = Keychain.biometryAvailable }
        }
        await ready()
    }

    func rememberWithBiometry(_ on: Bool) {
        if on, let pw = lastPassword { _ = Keychain.save(pw) } else if !on { Keychain.delete() }
        lastPassword = nil
    }

    func lock() async {
        _ = try? await Core.shared.call("vault_lock")
        phase = .locked
    }

    private func ready() async {
        phase = .ready
        await reload()
    }

    func reload() async {
        var out: [HostSection] = []
        if let personal = try? await Core.shared.call("vault_get", as: VaultView.self) {
            out += Self.grouped(personal.hosts, vault: "personal", prefix: nil)
        }
        var signIns: [CoreErrorInfo?] = []
        if let teams = try? await Core.shared.call("teams_list", as: [TeamEntry].self) {
            for t in teams {
                guard let v = t.view else { continue }
                out += Self.grouped(v.data.hosts, vault: v.id, prefix: v.name)
                signIns.append(v.remote?.error)
            }
        }
        sections = out
        sync = try? await Core.shared.call("sync_status", as: SyncStatus.self)
        signIns.append(sync?.error)
        var drives = signIns.compactMap { $0?.kind == "driveSignIn" ? $0?.drive : nil }
        var folders: [PendingTeam] = []
        if sync?.enabled == true, let pending = try? await Core.shared.call("teams_pending", as: [PendingTeam].self), !pending.isEmpty {
            let accounts = await Drives.accounts()
            for t in pending {
                if let d = t.drive {
                    if accounts[d.provider] == nil { drives.append(d.provider) }
                } else if t.url == nil {
                    folders.append(t)
                }
            }
        }
        driveSignIns = DriveProvider.allCases.filter { drives.contains($0) }
        folderTeams = folders
    }

    /// Signed in to a drive again: what is kept there syncs now, and teams in it join here.
    func driveSignedIn() async {
        if sync?.enabled == true { _ = try? await Core.shared.call("sync_now") }
        if let teams = try? await Core.shared.call("teams_list", as: [TeamEntry].self) {
            for t in teams where t.view?.remote?.error?.kind == "driveSignIn" {
                _ = try? await Core.shared.call("team_sync", ["id": t.id])
            }
        }
        await reload()
    }

    /// Pulls what changed on other devices.
    func syncNow() async {
        if sync?.enabled == true { _ = try? await Core.shared.call("sync_now") }
        await reload()
    }

    private static func grouped(_ hosts: [Host], vault: String, prefix: String?) -> [HostSection] {
        let tagged = hosts.map { h -> Host in
            var h = h
            h.vaultId = vault
            return h
        }
        let groups = Dictionary(grouping: tagged, by: \.group)
        return groups.keys.sorted { a, b in a.isEmpty != b.isEmpty ? !a.isEmpty : a.localizedStandardCompare(b) == .orderedAscending }.map { g in
            let title = [prefix, g.isEmpty ? nil : g].compactMap { $0 }.joined(separator: " / ")
            return HostSection(id: "\(vault)/\(g)", title: title, hosts: groups[g]!.sorted { $0.label.localizedStandardCompare($1.label) == .orderedAscending })
        }
    }

    // MARK: terminals

    func open(_ host: Host) {
        let t = TerminalSession(host: host)
        terminals.append(t)
        current = t.id
        showTerminals = true
        t.connect()
    }

    func close(_ t: TerminalSession) {
        t.close()
        terminals.removeAll { $0.id == t.id }
        if current == t.id { current = terminals.last?.id }
        if terminals.isEmpty { showTerminals = false }
    }

    /// Back in front: sessions whose connection went while the app slept connect again.
    func resumed() {
        endBackground()
        guard phase == .ready else { return }
        for t in terminals { t.checkConnection() }
    }

    /// To the background: iOS gives the app some time before suspending it, so a quick look at
    /// another app doesn't cost the sessions.
    func backgrounded() {
        guard background == .invalid, !terminals.isEmpty else { return }
        background = UIApplication.shared.beginBackgroundTask(withName: "sessions") { [weak self] in
            MainActor.assumeIsolated { self?.endBackground() }
        }
    }

    private func endBackground() {
        guard background != .invalid else { return }
        UIApplication.shared.endBackgroundTask(background)
        background = .invalid
    }

    /// Another network (Wi-Fi to cellular, say): connections over the old one are likely gone.
    private func networkChanged(_ now: [Bool]) {
        defer { lastNetwork = now }
        guard let last = lastNetwork, last != now, now[0], phase == .ready else { return }
        for t in terminals { t.checkConnection() }
    }

    func open(link url: URL) {
        if let link = JoinLink(url.absoluteString) { pendingJoin = link }
    }
}
