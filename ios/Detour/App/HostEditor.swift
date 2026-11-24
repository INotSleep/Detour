import SwiftUI

/// Adds a host to a vault (the personal one or a team's), or changes one. Fields the phone doesn't show (proxy,
/// tags, ssh-agent forwarding...) are kept as they are.
struct HostEditor: View {
    /// The host to change; nil adds one.
    let hostId: String?
    /// The vault it is in, or goes to.
    var vault: String = VaultData.personal
    let saved: () async -> Void
    @Environment(\.dismiss) private var dismiss

    @State private var original: PersonalVault.JSON?
    @State private var label = ""
    @State private var address = ""
    @State private var port = "22"
    @State private var user = ""
    @State private var group = ""
    @State private var auth = "password"
    @State private var keyId = ""
    @State private var password = ""
    @State private var hasPassword = false
    @State private var keys: [SshKey] = []
    @State private var groups: [String] = []
    /// The way to the host: "" direct, "p:<id>" a saved proxy, "j:<host id>" through a host,
    /// "legacy" as an older version wrote it.
    @State private var route = ""
    @State private var routeBefore = ""
    @State private var routes: [Choice] = []
    @State private var loaded = false
    @State private var busy = false
    @State private var error: String?
    @FocusState private var focus: Field?

    private enum Field { case address, label, port, user, group, password }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Address", text: $address)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .focused($focus, equals: .address)
                        .submitLabel(.next)
                        .onSubmit { focus = .user }
                    TextField("User", text: $user)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .focused($focus, equals: .user)
                    TextField("Port", text: $port)
                        .keyboardType(.numberPad)
                        .focused($focus, equals: .port)
                    TextField(labelPlaceholder, text: $label)
                        .autocorrectionDisabled()
                        .focused($focus, equals: .label)
                }
                Section {
                    Picker("Login", selection: $auth) {
                        Text("Password").tag("password")
                        if !keys.isEmpty { Text("Key").tag("key") }
                        if auth == "agent" { Text("ssh-agent").tag("agent") }
                    }
                    .pickerStyle(.segmented)
                    if auth == "password" {
                        SecureField(hasPassword ? String(localized: "Saved; type to change") : String(localized: "Ask when connecting"), text: $password)
                            .focused($focus, equals: .password)
                    } else if auth == "key" {
                        Picker("Key", selection: $keyId) {
                            ForEach(keys) { k in Text(k.name).tag(k.id) }
                        }
                    }
                }
                Section {
                    TextField("Group", text: $group)
                        .autocorrectionDisabled()
                        .focused($focus, equals: .group)
                    if focus == .group {
                        let matching = groups.filter { !$0.isEmpty && $0 != group && (group.isEmpty || $0.localizedCaseInsensitiveContains(group)) }
                        ForEach(matching.prefix(5), id: \.self) { g in
                            Button(g) { group = g }
                        }
                    }
                }
                Section {
                    Picker("Proxy", selection: $route) {
                        ForEach(routes) { r in Text(verbatim: r.name).tag(r.id) }
                    }
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.footnote)
                }
            }
            .navigationTitle(hostId == nil ? "New host" : "Edit host")
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

    private var labelPlaceholder: String {
        let a = address.trimmingCharacters(in: .whitespaces)
        return a.isEmpty ? String(localized: "Name") : a
    }

    private var portNumber: Int? { Int(port.trimmingCharacters(in: .whitespaces)).flatMap { (1...65535).contains($0) ? $0 : nil } }

    private var ready: Bool {
        loaded && !address.trimmingCharacters(in: .whitespaces).isEmpty && !user.trimmingCharacters(in: .whitespaces).isEmpty && portNumber != nil && (auth != "key" || !keyId.isEmpty)
    }

    private func load() async {
        guard !loaded else { return }
        let v = (try? await VaultData.get(vault)) ?? [:]
        // The vault's own keys: a team host logs in with the team's.
        if let data = try? JSONSerialization.data(withJSONObject: v["keys"] ?? []), let k = try? JSONDecoder().decode([SshKey].self, from: data) { keys = k }
        let hosts = v["hosts"] as? [PersonalVault.JSON] ?? []
        groups = Array(Set(hosts.compactMap { $0["group"] as? String })).sorted()
        if let id = hostId, let h = hosts.first(where: { $0["id"] as? String == id }) {
            original = h
            label = h["label"] as? String ?? ""
            address = h["address"] as? String ?? ""
            port = String(h["port"] as? Int ?? 22)
            user = h["user"] as? String ?? ""
            group = h["group"] as? String ?? ""
            hasPassword = h["hasPassword"] as? Bool ?? false
            let a = h["auth"] as? PersonalVault.JSON ?? [:]
            auth = a["type"] as? String ?? "password"
            keyId = a["keyId"] as? String ?? ""
        } else {
            keyId = keys.first?.id ?? ""
            focus = .address
        }
        if keyId.isEmpty { keyId = keys.first?.id ?? "" }
        // The way to it: a saved proxy of the same vault, or another of its hosts.
        let this = hostId.flatMap { id in hosts.first { $0["id"] as? String == id } }
        let wasProxy = this?["proxy"] as? PersonalVault.JSON
        if let id = this?["proxyId"] as? String {
            route = "p:\(id)"
        } else if wasProxy?["type"] as? String == "jump", let id = wasProxy?["hostId"] as? String {
            route = "j:\(id)"
        } else if let p = wasProxy, !p.isEmpty {
            route = "legacy"
        }
        routeBefore = route
        var list = [Choice(id: "", name: String(localized: "Not set"))]
        if route == "legacy", let p = wasProxy {
            list.append(Choice(id: "legacy", name: "\((p["type"] as? String ?? "").uppercased()) \(p["address"] as? String ?? ""):\(p["port"] as? Int ?? 0)"))
        }
        for p in v["proxies"] as? [PersonalVault.JSON] ?? [] {
            if let id = p["id"] as? String { list.append(Choice(id: "p:\(id)", name: p["name"] as? String ?? "")) }
        }
        for h in hosts where h["id"] as? String != hostId {
            if let id = h["id"] as? String {
                let name = h["label"] as? String ?? h["address"] as? String ?? ""
                list.append(Choice(id: "j:\(id)", name: String(localized: "Through \(name)")))
            }
        }
        routes = list
        loaded = true
    }

    private func save() async {
        guard ready, !busy, let port = portNumber else { return }
        busy = true
        error = nil
        let a = address.trimmingCharacters(in: .whitespaces)
        var h = original ?? ["id": PersonalVault.newId(), "tags": [String]()]
        h["address"] = a
        h["label"] = label.trimmingCharacters(in: .whitespaces).isEmpty ? a : label.trimmingCharacters(in: .whitespaces)
        h["port"] = port
        h["user"] = user.trimmingCharacters(in: .whitespaces)
        h["group"] = group.trimmingCharacters(in: .whitespaces)
        // Unchanged, the host keeps what older versions on other devices know how to reach it by.
        if route != routeBefore {
            if route.hasPrefix("p:") {
                h["proxyId"] = String(route.dropFirst(2))
                h["proxy"] = nil
            } else if route.hasPrefix("j:") {
                h["proxy"] = ["type": "jump", "hostId": String(route.dropFirst(2))]
                h["proxyId"] = nil
            } else if route.isEmpty {
                h["proxy"] = nil
                h["proxyId"] = nil
            }
        }
        switch auth {
        case "key": h["auth"] = ["type": "key", "keyId": keyId]
        case "agent": h["auth"] = ["type": "agent"]
        default:
            h["auth"] = ["type": "password"]
            if !password.isEmpty { h["newPassword"] = password }
        }
        do {
            try await VaultData.save(host: h, in: vault)
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
