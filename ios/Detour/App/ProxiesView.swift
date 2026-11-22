import SwiftUI

/// A saved proxy, as the core shows it (`ProxyView`): its kind stays JSON, there are several.
struct ProxyItem: Identifiable {
    let vaultId: String
    let json: [String: Any]

    var id: String { "\(vaultId)/\(json["id"] as? String ?? "")" }
    var name: String { json["name"] as? String ?? "" }
    private var kind: [String: Any] { json["kind"] as? [String: Any] ?? [:] }

    /// "VLESS · example.com:443"
    var summary: String {
        let type = (kind["type"] as? String ?? "").uppercased()
        let shown = type == "SOCKS5" ? "SOCKS5" : type == "WIREGUARD" ? "WireGuard" : type
        let place: String
        if let endpoint = kind["endpoint"] as? String {
            place = endpoint
        } else if let address = kind["address"] as? String {
            place = "\(address):\(kind["port"] as? Int ?? 0)"
        } else {
            place = ""
        }
        return [shown, place].filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

/// The proxies of one vault: the personal one (no name) or a team's.
struct ProxyGroup: Identifiable {
    let id: String
    let name: String?
    let proxies: [ProxyItem]
}

/// The proxies of the vault and its teams: checked with a tap, added from a link or a WireGuard
/// config, the personal vault's deleted with a swipe.
struct ProxiesView: View {
    @State private var groups: [ProxyGroup]?
    @State private var checks: [String: String] = [:]
    @State private var checking: Set<String> = []
    @State private var adding = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            List {
                ForEach(groups ?? []) { g in
                    Section {
                        ForEach(g.proxies) { p in
                            Button { Task { await check(p) } } label: { row(p) }
                                .tint(.primary)
                                .swipeActions(edge: .trailing) {
                                    if p.vaultId == "personal" {
                                        Button(role: .destructive) { Task { await delete(p) } } label: { Label("Delete", systemImage: "trash") }
                                    }
                                }
                        }
                    } header: {
                        if let name = g.name { Text(name) } else if (groups?.count ?? 0) > 1 { Text("My proxies") }
                    }
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.footnote)
                }
            }
            .overlay {
                if groups == nil {
                    ProgressView()
                } else if groups?.isEmpty == true {
                    ContentUnavailableView {
                        Label("No proxies", systemImage: "shield")
                    } actions: {
                        Button("Add proxy") { adding = true }.buttonStyle(.borderedProminent)
                    }
                }
            }
            .navigationTitle("Proxies")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button { adding = true } label: { Image(systemName: "plus") }
                        .accessibilityLabel("Add proxy")
                }
            }
            .sheet(isPresented: $adding) { AddProxyView { await load() } }
            .task { await load() }
            .refreshable { await load() }
        }
    }

    private func row(_ p: ProxyItem) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(p.name).font(.body.weight(.medium))
                Text(p.summary).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            if checking.contains(p.id) {
                ProgressView()
            } else if let c = checks[p.id] {
                Text(c).font(.footnote).foregroundStyle(c.hasSuffix("ms") ? Color.green : Color.red).lineLimit(2).multilineTextAlignment(.trailing)
            }
        }
    }

    private func load() async {
        var out: [ProxyGroup] = []
        if let v = try? await Core.shared.call("vault_get") as? [String: Any], let list = v["proxies"] as? [[String: Any]], !list.isEmpty {
            out.append(ProxyGroup(id: "personal", name: nil, proxies: list.map { ProxyItem(vaultId: "personal", json: $0) }))
        }
        if let teams = try? await Core.shared.call("teams_list") as? [[String: Any]] {
            for t in teams {
                guard let view = t["view"] as? [String: Any], let id = view["id"] as? String,
                      let data = view["data"] as? [String: Any], let list = data["proxies"] as? [[String: Any]], !list.isEmpty else { continue }
                out.append(ProxyGroup(id: id, name: view["name"] as? String, proxies: list.map { ProxyItem(vaultId: id, json: $0) }))
            }
        }
        groups = out
    }

    /// Connects through the proxy once: how long it took, or what went wrong.
    private func check(_ p: ProxyItem) async {
        checking.insert(p.id)
        defer { checking.remove(p.id) }
        do {
            let r = try await Core.shared.call("proxy_check", ["vaultId": p.vaultId, "proxy": p.json]) as? [String: Any]
            checks[p.id] = "\(r?["ms"] as? Int ?? 0) ms"
        } catch let f as CoreFailure {
            checks[p.id] = TerminalSession.describe(f)
        } catch {
            checks[p.id] = error.localizedDescription
        }
    }

    private func delete(_ p: ProxyItem) async {
        let id = p.json["id"] as? String
        do {
            try await PersonalVault.edit { v in
                v["proxies"] = (v["proxies"] as? [PersonalVault.JSON] ?? []).filter { $0["id"] as? String != id }
            }
            error = nil
        } catch {
            self.error = coreErrorText(error)
        }
        await load()
    }
}

/// A proxy from a link (`vless://`, `socks5://`...) or a WireGuard config, pasted in.
private struct AddProxyView: View {
    let added: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextEditor(text: $text)
                        .font(.footnote.monospaced())
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .frame(minHeight: 140)
                    Button {
                        if let s = UIPasteboard.general.string { text = s }
                    } label: {
                        Label("Paste", systemImage: "doc.on.clipboard")
                    }
                } header: {
                    Text("Link or WireGuard config")
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.footnote)
                }
            }
            .navigationTitle("Add proxy")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else { Button("Add") { Task { await add() } }.disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) }
                }
            }
        }
    }

    private func add() async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            let parsed = try await Core.shared.call("proxy_parse", ["text": text.trimmingCharacters(in: .whitespacesAndNewlines)]) as? [String: Any] ?? [:]
            let proxy: PersonalVault.JSON = [
                "id": PersonalVault.newId(),
                "name": parsed["name"] as? String ?? "proxy",
                "kind": parsed["kind"] ?? [String: Any](),
                "newSecrets": parsed["secrets"] ?? [String: Any](),
            ]
            try await PersonalVault.edit { v in
                v["proxies"] = (v["proxies"] as? [PersonalVault.JSON] ?? []) + [proxy]
            }
            await added()
            dismiss()
        } catch let f as CoreFailure {
            error = f.kind == "proxy" ? String(localized: "Detour can't read this proxy") : TerminalSession.describe(f)
        } catch {
            self.error = error.localizedDescription
        }
    }
}
