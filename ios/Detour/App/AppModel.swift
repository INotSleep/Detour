import Foundation
import Network
import SwiftUI

/// An open session: a terminal or a host's files over SFTP.
enum OpenSession: Identifiable {
    case terminal(TerminalSession)
    case files(FilesSession)

    var id: UUID {
        switch self {
        case .terminal(let t): t.id
        case .files(let f): f.uid
        }
    }

    var host: Host {
        switch self {
        case .terminal(let t): t.host
        case .files(let f): f.host
        }
    }

    var terminal: TerminalSession? {
        if case .terminal(let t) = self { return t }
        return nil
    }
}

/// A team being opened from a drive folder: how far the download is, or why it failed.
struct OpeningTeam: Equatable {
    var done = 0
    var total = 0
    var error: String?
}

/// The app's tabs, in the order of the tab bar.
enum MainTab: String, CaseIterable, Identifiable {
    case hosts, sessions, keys, snippets, proxies, teams, settings
    var id: String { rawValue }
}

/// What the app shows and holds: the vault's state, its hosts, the open sessions.
@MainActor
final class AppModel: ObservableObject {
    enum Phase { case loading, setup, locked, ready }

    @Published var phase: Phase = .loading
    @Published var sections: [HostSection] = []
    @Published var sync: SyncStatus?
    /// Drives to sign in to again: the vault's sync or a team is kept there.
    @Published var driveSignIns: [DriveSignInNeed] = []
    /// Teams on this person's other devices kept in a folder: opened here through a drive.
    @Published var folderTeams: [PendingTeam] = []
    /// The personal vault and the teams, with their hosts, groups and this person's own grouping.
    @Published var vaults: [VaultHosts] = []
    /// The vault the hosts screen shows: new hosts go there.
    @Published private(set) var hostsVault = UserDefaults.standard.string(forKey: "hostsVault") ?? VaultData.personal
    /// Teams whose hosts show in this person's own groups, not the shared ones.
    @Published private(set) var mineViews = Set(UserDefaults.standard.stringArray(forKey: "mineViews") ?? [])
    /// Teams being opened from a drive folder, by team id: shown with their progress in the lists.
    @Published var openingTeams: [String: OpeningTeam] = [:]
    /// Bumped when a team joined here: the teams screen reads its list again.
    @Published var teamsVersion = 0
    @Published var sessions: [OpenSession] = []
    @Published var current: OpenSession.ID?
    @Published var showTerminals = false
    @Published var tab: MainTab = .hosts

    /// The open terminals (not the files sessions).
    var terminals: [TerminalSession] { sessions.compactMap(\.terminal) }
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
        Core.shared.onEvent { [weak self] name, data in
            guard let self else { return }
            switch name {
            case "locked": self.phase = .locked
            case "data.changed", "unlocked": if self.phase == .ready { Task { await self.reload() } }
            case "drive.progress":
                let done = (data["done"] as? NSNumber)?.intValue ?? 0
                let total = (data["total"] as? NSNumber)?.intValue ?? 0
                for (id, o) in self.openingTeams where o.error == nil {
                    self.openingTeams[id] = OpeningTeam(done: done, total: total)
                }
            default: break
            }
        }
    }

    func start() async {
        #if DEBUG
        await Shots.prepare()
        #endif
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

    /// Changes shown before the core has them: a reload begun before one is saved reads what was there before.
    private var saving = 0
    private var shownAt = 0

    /// Shows a vault changed here at once, while `save` keeps it.
    func showWhileSaving(_ next: VaultHosts, save: () async throws -> Void) async throws {
        withAnimation { vaults = vaults.map { $0.id == next.id ? next : $0 } }
        shownAt += 1
        saving += 1
        defer { saving -= 1 }
        try await save()
    }

    func reload() async {
        let began = shownAt
        var all: [VaultHosts] = []
        if let personal = try? await Core.shared.call("vault_get", as: VaultView.self) {
            all.append(VaultHosts(id: VaultData.personal, name: nil, role: nil, hosts: personal.hosts.map { h in var h = h; h.vaultId = VaultData.personal; return h }, groups: personal.groups ?? [], layout: nil))
        }
        var signIns: [CoreErrorInfo?] = []
        if let teams = try? await Core.shared.call("teams_list", as: [TeamEntry].self) {
            for t in teams {
                guard let v = t.view else { continue }
                let me = v.members.first { $0.isYou }
                var levels: [String: String] = [:]
                for a in v.access ?? [] where a.member == me?.id { levels[a.record] = a.level }
                let hosts = v.data.hosts.map { h in
                    var h = h
                    h.vaultId = v.id
                    h.access = levels[h.id] ?? "none"
                    return h
                }
                all.append(VaultHosts(id: v.id, name: v.name, role: me?.role, hosts: hosts, groups: v.data.groups ?? [], layout: v.layout?.groups))
                signIns.append(v.remote?.error)
            }
        }
        // The user connecting goes with, as the core picks it: this person's settings of the host's
        // group of theirs, then of its shared group.
        let prefs = (try? await Core.shared.call("group_prefs") as? [[String: Any]]) ?? []
        var byKey: [String: [String: Any]] = [:]
        for p in prefs { if let k = p["key"] as? String { byKey[k] = p } }
        func loginPrefs(_ key: String) -> [String: Any]? {
            guard let p = byKey[key], p["user"] is String || p["auth"] is [String: Any] else { return nil }
            return p
        }
        // Read before a change shown here was saved: the reload after saving it shows it.
        if saving == 0 && began == shownAt {
            vaults = all.map { v in
                var next = v
                next.hosts = v.hosts.map { h in
                    var h = h
                    let mine = v.layout?.first { $0.hosts.contains(h.id) }.flatMap { loginPrefs("\(v.id)/mine:\($0.id)") }
                    if let user = (mine ?? loginPrefs("\(v.id)/team:\(h.group)"))?["user"] as? String, !user.isEmpty { h.groupUser = user }
                    return h
                }
                return next
            }
            sections = vaults.flatMap { v in
                Groups.list(v.hosts, v.groups, withEmpty: false).map { g in
                    HostSection(id: "\(v.id)/\(g.name)", title: [v.name, g.name.isEmpty ? nil : g.name].compactMap { $0 }.joined(separator: " / "), hosts: g.hosts)
                }
            }
        }
        sync = try? await Core.shared.call("sync_status", as: SyncStatus.self)
        signIns.append(sync?.error)
        var needs: [DriveSignInNeed] = signIns.compactMap { e in e?.kind == "driveSignIn" ? e?.drive.map { DriveSignInNeed(provider: $0, account: e?.account) } : nil }
        var folders: [PendingTeam] = []
        if sync?.enabled == true, let pending = try? await Core.shared.call("teams_pending", as: [PendingTeam].self), !pending.isEmpty {
            let accounts = await Drives.signedIn()
            for t in pending {
                if let d = t.drive {
                    let here = accounts.contains { $0.provider == d.provider && (d.account == nil || $0.account == d.account) }
                    if !here { needs.append(DriveSignInNeed(provider: d.provider, account: d.account)) }
                } else if t.url == nil {
                    folders.append(t)
                }
            }
        }
        var seen = Set<DriveSignInNeed>()
        driveSignIns = needs.filter { seen.insert($0).inserted }
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

    /// Opens here a team kept in a drive folder. It runs on its own: the list of teams shows how
    /// far it is, and the sheet that started it is gone.
    func openTeamFromDrive(_ team: PendingTeam, folder: DriveFolderRef) {
        if let o = openingTeams[team.teamId], o.error == nil { return }
        openingTeams[team.teamId] = OpeningTeam()
        tab = .teams
        // A look at another app meanwhile doesn't stop it.
        let id = team.teamId
        opening[id] = UIApplication.shared.beginBackgroundTask(withName: "team") { [weak self] in
            MainActor.assumeIsolated { self?.endOpening(id) }
        }
        Task {
            defer { endOpening(id) }
            do {
                _ = try await Core.shared.call("team_join_linked_drive", ["teamId": team.teamId, "drive": folder.json])
                await reload()
                openingTeams[team.teamId] = nil
                teamsVersion += 1
            } catch {
                openingTeams[team.teamId]?.error = Drives.errorText(error)
            }
        }
    }

    /// iOS's time in the background for the teams being opened.
    private var opening: [String: UIBackgroundTaskIdentifier] = [:]

    private func endOpening(_ id: String) {
        guard let t = opening.removeValue(forKey: id) else { return }
        UIApplication.shared.endBackgroundTask(t)
    }

    /// Pulls what changed on other devices.
    func syncNow() async {
        if sync?.enabled == true { _ = try? await Core.shared.call("sync_now") }
        await reload()
    }

    /// What a failed call says to the person.
    nonisolated static func describe(_ error: Error) -> String {
        if let f = error as? CoreFailure { return TerminalSession.describe(f) }
        return error.localizedDescription
    }

    func showVault(_ id: String) {
        hostsVault = id
        UserDefaults.standard.set(id, forKey: "hostsVault")
    }

    /// Shows a team's hosts in this person's own groups (made from the shared ones the first
    /// time) or the shared ones. Returns why making them failed.
    func setMineView(_ team: VaultHosts, _ on: Bool) async -> String? {
        if on { mineViews.insert(team.id) } else { mineViews.remove(team.id) }
        UserDefaults.standard.set(Array(mineViews), forKey: "mineViews")
        guard on, team.layout == nil else { return nil }
        defer { Task { await reload() } }
        do {
            try await Groups.saveLayout(team.id, Groups.layoutFrom(team.hosts, team.groups))
            return nil
        } catch {
            return Self.describe(error)
        }
    }

    // MARK: terminals

    func open(_ host: Host) {
        let t = TerminalSession(host: host)
        sessions.append(.terminal(t))
        current = t.id
        showTerminals = true
        t.connect()
    }

    /// A host's files, as a session of its own beside the terminals.
    func openFiles(_ host: Host) {
        let f = FilesSession(host: host)
        sessions.append(.files(f))
        current = f.uid
        showTerminals = true
        Task { await f.open() }
    }

    /// Brings an open session forward.
    func show(_ s: OpenSession) {
        current = s.id
        showTerminals = true
    }

    func close(_ s: OpenSession) {
        switch s {
        case .terminal(let t): t.close()
        case .files(let f): f.close()
        }
        sessions.removeAll { $0.id == s.id }
        if current == s.id { current = sessions.last?.id }
        if sessions.isEmpty { showTerminals = false }
    }

    /// Back in front: sessions whose connection went while the app slept connect again.
    func resumed() {
        endBackground()
        // A drive's sign-in waiting for the browser: iOS may have taken its listening socket while
        // the app was away (the person off finding a password), so it listens again.
        Task { _ = try? await Core.shared.call("drive_sign_in_resume") }
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
