import SwiftUI

/// This person's own login for a team host: kept in their personal vault and used instead of
/// the team's shared one while it is on.
struct OwnLoginEditor: View {
    let host: Host
    let saved: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var exists = false
    @State private var active = true
    @State private var user = ""
    @State private var auth = "password"
    @State private var keyId = ""
    @State private var password = ""
    @State private var hasPassword = false
    @State private var keys: [SshKey] = []
    @State private var loaded = false
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                if exists {
                    Section {
                        Toggle("Log in with my own login", isOn: $active)
                    } footer: {
                        if !active { Text("The team's login is used: \(host.user)") }
                    }
                }
                Section {
                    TextField("User", text: $user)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Picker("Login", selection: $auth) {
                        Text("Password").tag("password")
                        if !keys.isEmpty { Text("Key").tag("key") }
                    }
                    .pickerStyle(.segmented)
                    if auth == "password" {
                        SecureField(hasPassword ? String(localized: "Saved; type to change") : String(localized: "Ask when connecting"), text: $password)
                    } else {
                        Picker("Key", selection: $keyId) {
                            ForEach(keys) { k in Text(k.name).tag(k.id) }
                        }
                    }
                }
                if exists {
                    Section {
                        Button("Remove my login", role: .destructive) { Task { await set(nil) } }
                    }
                }
                if let error { Text(error).foregroundStyle(.red).font(.footnote) }
            }
            .navigationTitle(host.label)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else {
                        Button("Save") { Task { await save() } }
                            .disabled(!loaded || user.trimmingCharacters(in: .whitespaces).isEmpty || (auth == "key" && keyId.isEmpty))
                    }
                }
            }
            .task { await load() }
        }
    }

    private func load() async {
        guard !loaded else { return }
        keys = (try? await Core.shared.call("vault_get", as: VaultKeys.self).keys) ?? []
        let teams = (try? await Core.shared.call("teams_list") as? [[String: Any]]) ?? []
        let view = teams.compactMap { $0["view"] as? [String: Any] }.first { $0["id"] as? String == host.vaultId }
        let hosts = (view?["data"] as? [String: Any])?["hosts"] as? [[String: Any]] ?? []
        if let own = hosts.first(where: { $0["id"] as? String == host.id })?["own"] as? [String: Any] {
            exists = true
            active = own["active"] as? Bool ?? true
            user = own["user"] as? String ?? ""
            hasPassword = own["hasPassword"] as? Bool ?? false
            let a = own["auth"] as? [String: Any] ?? [:]
            auth = a["type"] as? String == "key" ? "key" : "password"
            keyId = a["keyId"] as? String ?? ""
        }
        if keyId.isEmpty { keyId = keys.first?.id ?? "" }
        loaded = true
    }

    private func save() async {
        var login: [String: Any] = [
            "user": user.trimmingCharacters(in: .whitespaces),
            "auth": auth == "key" ? ["type": "key", "keyId": keyId] : ["type": "password"],
            "active": exists ? active : true,
        ]
        if auth == "password", !password.isEmpty { login["newPassword"] = password }
        await set(login)
    }

    private func set(_ login: [String: Any]?) async {
        busy = true
        error = nil
        do {
            _ = try await Core.shared.call("team_own_login_set", ["id": host.vaultId, "hostId": host.id, "login": login])
            await saved()
            dismiss()
        } catch let f as CoreFailure {
            error = f["message"] ?? f.kind
        } catch {
            self.error = error.localizedDescription
        }
        busy = false
    }
}
