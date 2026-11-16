import SwiftUI

/// The hosts of the vault and its teams; tapping one opens it in a terminal.
struct HostsView: View {
    @EnvironmentObject var app: AppModel
    @State private var query = ""
    @State private var joining = false

    var body: some View {
        NavigationStack {
            List {
                ForEach(filtered) { section in
                    Section(section.title) {
                        ForEach(section.hosts) { host in
                            Button { app.open(host) } label: { HostRow(host: host) }
                        }
                    }
                }
            }
            .overlay { empty }
            .searchable(text: $query)
            .refreshable { await app.syncNow() }
            .navigationTitle("Hosts")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    if !app.terminals.isEmpty {
                        Button { app.showTerminals = true } label: {
                            Label("\(app.terminals.count)", systemImage: "terminal")
                                .labelStyle(.titleAndIcon)
                        }
                    }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        if app.sync?.enabled != true {
                            Button { joining = true } label: { Label("Connect to your vault", systemImage: "arrow.triangle.2.circlepath") }
                        }
                        NavigationLink { KeyBarEditor() } label: { Label("Key bar", systemImage: "keyboard") }
                        NavigationLink { SettingsView() } label: { Label("Settings", systemImage: "gearshape") }
                        Button { Task { await app.lock() } } label: { Label("Lock", systemImage: "lock") }
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                }
            }
            .sheet(isPresented: $joining) { SyncJoinView() }
            .onAppear { joinIfLinked() }
            .onChange(of: app.pendingJoin) { joinIfLinked() }
        }
    }

    /// A join link opened the app: the form, filled in from it.
    private func joinIfLinked() {
        guard app.pendingJoin != nil else { return }
        if app.sync?.enabled == true { app.pendingJoin = nil } else { joining = true }
    }

    private var filtered: [HostSection] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return app.sections }
        return app.sections.compactMap { s in
            let hosts = s.hosts.filter { "\($0.label) \($0.address) \($0.login) \(s.title)".lowercased().contains(q) }
            return hosts.isEmpty ? nil : HostSection(id: s.id, title: s.title, hosts: hosts)
        }
    }

    @ViewBuilder private var empty: some View {
        if app.sections.isEmpty {
            ContentUnavailableView {
                Label("No hosts", systemImage: "server.rack")
            } actions: {
                if app.sync?.enabled != true {
                    Button("Connect to your vault") { joining = true }.buttonStyle(.borderedProminent)
                }
            }
        } else if filtered.isEmpty {
            ContentUnavailableView.search(text: query)
        }
    }
}

private struct HostRow: View {
    let host: Host

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(host.label).font(.body.weight(.medium)).foregroundStyle(.primary)
            Text("\(host.login)@\(host.address)\(host.port == 22 ? "" : ":\(host.port)")")
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 2)
    }
}

/// Face ID, the key bar, the version.
struct SettingsView: View {
    @State private var biometry = Keychain.isSaved
    @State private var askPassword = false
    @State private var password = ""
    @State private var error: String?

    var body: some View {
        Form {
            if Keychain.biometryAvailable {
                Section {
                    Toggle("Unlock with \(Keychain.biometryName)", isOn: Binding(get: { biometry }, set: { on in
                        if on { askPassword = true } else { Keychain.delete(); biometry = false }
                    }))
                    if let error { Text(error).foregroundStyle(.red).font(.footnote) }
                }
            }
            Section {
                NavigationLink("Key bar") { KeyBarEditor() }
            }
            Section {
                LabeledContent("Version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "")
            }
        }
        .navigationTitle("Settings")
        .alert("Master password", isPresented: $askPassword) {
            SecureField("Master password", text: $password)
            Button("Cancel", role: .cancel) { password = "" }
            Button("Turn on") { Task { await turnOn() } }
        }
    }

    private func turnOn() async {
        let pw = password
        password = ""
        do {
            // Checked by the core before it is kept.
            _ = try await Core.shared.call("vault_unlock", ["masterPassword": pw])
            biometry = Keychain.save(pw)
            error = biometry ? nil : String(localized: "Couldn't save it on this device")
        } catch {
            self.error = String(localized: "Wrong master password")
        }
    }
}
