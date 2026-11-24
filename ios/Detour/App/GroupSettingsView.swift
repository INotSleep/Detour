import SwiftUI

/// A group's settings: this person's own proxy and login for its hosts (kept in their personal
/// vault, used before the hosts' own), and for those who manage a team, the proxy of the whole team.
/// A choice of a picker: what it keeps, and what it shows.
struct Choice: Identifiable, Hashable {
    let id: String
    let name: String
}

struct GroupSettingsView: View {
    let vault: VaultHosts
    let group: ListedGroup
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var proxies: [Choice] = []
    @State private var teamProxies: [Choice] = []
    @State private var keys: [SshKey] = []
    @State private var proxy = ""
    @State private var teamProxy = ""
    @State private var teamProxyBefore = ""
    @State private var user = ""
    @State private var auth = "none"
    @State private var keyId = ""
    @State private var password = ""
    @State private var hasPassword = false
    @State private var loaded = false
    @State private var busy = false
    @State private var error: String?

    private var key: String { "\(vault.id)/\(group.mine.map { "mine:\($0.id)" } ?? "team:\(group.name)")" }
    /// The team's proxy of a shared group, for those who manage the team.
    private var teamProxyShown: Bool { vault.team && group.mine == nil && vault.canManage }
    private var ready: Bool { loaded && !(auth == "key" && keyId.isEmpty) }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("Proxy", selection: $proxy) {
                        Text("Not set").tag("")
                        ForEach(proxies) { p in Text(verbatim: p.name).tag(p.id) }
                    }
                    if teamProxyShown {
                        Picker("Proxy for the whole team", selection: $teamProxy) {
                            Text("Not set").tag("")
                            ForEach(teamProxies) { p in Text(verbatim: p.name).tag(p.id) }
                        }
                    }
                }
                Section {
                    TextField("User", text: $user)
                        .font(.body.monospaced())
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                Section("Login") {
                    Picker("Login", selection: $auth) {
                        Text("Not set").tag("none")
                        if !keys.isEmpty { Text("Key").tag("key") }
                        Text("Password").tag("password")
                        if auth == "agent" { Text(verbatim: "ssh-agent").tag("agent") }
                    }
                    .pickerStyle(.segmented)
                    if auth == "password" {
                        SecureField(hasPassword ? String(localized: "Saved; type to change") : String(localized: "Ask when connecting"), text: $password)
                    } else if auth == "key" {
                        Picker("Key", selection: $keyId) {
                            ForEach(keys) { k in Text(k.name).tag(k.id) }
                        }
                    }
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.footnote)
                }
            }
            .navigationTitle(group.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else { Button("Save") { Task { await save() } }.disabled(!ready) }
                }
            }
            .task { await load() }
        }
    }

    private static func proxiesOf(_ vault: String) async -> [Choice] {
        let list = (try? await VaultData.get(vault))?["proxies"] as? [VaultData.JSON] ?? []
        return list.compactMap { p in (p["id"] as? String).map { Choice(id: "\(vault)|\($0)", name: p["name"] as? String ?? "") } }
    }

    private func load() async {
        guard !loaded else { return }
        let personal = (try? await PersonalVault.get()) ?? [:]
        if let data = try? JSONSerialization.data(withJSONObject: personal["keys"] ?? []), let k = try? JSONDecoder().decode([SshKey].self, from: data) { keys = k }
        proxies = await Self.proxiesOf(VaultData.personal)
        if vault.team {
            let team = await Self.proxiesOf(vault.id)
            proxies += team
            // The team's proxy is kept by its id alone.
            teamProxies = team.map { Choice(id: String($0.id.split(separator: "|").last ?? ""), name: $0.name) }
        }
        let all = (try? await Core.shared.call("group_prefs") as? [VaultData.JSON]) ?? []
        if let saved = all.first(where: { $0["key"] as? String == key }) {
            if let p = saved["proxy"] as? VaultData.JSON, let v = p["vault"] as? String, let id = p["id"] as? String { proxy = "\(v)|\(id)" }
            user = saved["user"] as? String ?? ""
            let a = saved["auth"] as? VaultData.JSON
            auth = a?["type"] as? String ?? "none"
            keyId = a?["keyId"] as? String ?? ""
            hasPassword = saved["hasPassword"] as? Bool ?? false
        }
        if keyId.isEmpty { keyId = keys.first?.id ?? "" }
        if teamProxyShown {
            let groups = (try? await VaultData.get(vault.id))?["groups"] as? [VaultData.JSON] ?? []
            teamProxy = groups.first { $0["name"] as? String == group.name }?["proxyId"] as? String ?? ""
            teamProxyBefore = teamProxy
        }
        loaded = true
    }

    private func save() async {
        guard ready, !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        var prefs: VaultData.JSON = ["key": key]
        let parts = proxy.split(separator: "|").map(String.init)
        if parts.count == 2 { prefs["proxy"] = ["vault": parts[0], "id": parts[1]] }
        let u = user.trimmingCharacters(in: .whitespaces)
        if !u.isEmpty { prefs["user"] = u }
        switch auth {
        case "key": prefs["auth"] = ["type": "key", "keyId": keyId]
        case "none": break
        default: prefs["auth"] = ["type": auth]
        }
        prefs["hasPassword"] = auth == "password" && (!password.isEmpty || hasPassword)
        if auth == "password" && !password.isEmpty { prefs["newPassword"] = password }
        let empty = proxy.isEmpty && u.isEmpty && auth == "none"
        do {
            _ = try await Core.shared.call("group_prefs_set", ["prefs": prefs, "drop": empty])
            if teamProxyShown && teamProxy != teamProxyBefore {
                let name = group.name, chosen = teamProxy
                try await VaultData.edit(vault.id) { v in
                    var groups = v["groups"] as? [VaultData.JSON] ?? []
                    if !groups.contains(where: { $0["name"] as? String == name }) {
                        let last = groups.compactMap { ($0["order"] as? NSNumber)?.doubleValue }.max() ?? 0
                        groups.append(["id": PersonalVault.newId(), "name": name, "order": last + 1])
                    }
                    groups = groups.map { g in
                        guard g["name"] as? String == name else { return g }
                        var g = g
                        if chosen.isEmpty { g["proxyId"] = nil } else { g["proxyId"] = chosen }
                        return g
                    }
                    v["groups"] = groups
                }
            }
            await app.reload()
            Haptics.success()
            dismiss()
        } catch {
            self.error = AppModel.describe(error)
        }
    }
}
