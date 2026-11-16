import Foundation
import SwiftUI

/// What the app shows and holds: the vault's state, its hosts, the open terminals.
@MainActor
final class AppModel: ObservableObject {
    enum Phase { case loading, setup, locked, ready }

    @Published var phase: Phase = .loading
    @Published var sections: [HostSection] = []
    @Published var sync: SyncStatus?
    @Published var terminals: [TerminalSession] = []
    @Published var current: TerminalSession.ID?
    @Published var showTerminals = false
    /// Offer Face ID once, after the first unlock with the password.
    @Published var offerBiometry = false
    private var lastPassword: String?

    init() {
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
        if let teams = try? await Core.shared.call("teams_list", as: [TeamEntry].self) {
            for t in teams {
                guard let v = t.view else { continue }
                out += Self.grouped(v.data.hosts, vault: v.id, prefix: v.name)
            }
        }
        sections = out
        sync = try? await Core.shared.call("sync_status", as: SyncStatus.self)
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

    /// Back in front: sessions that dropped while the app slept connect again.
    func resumed() {
        guard phase == .ready else { return }
        for t in terminals { t.reconnectIfDropped() }
    }
}
