import SwiftUI

/// The hosts of the vault and its teams; tapping one opens it in a terminal.
struct HostsView: View {
    @EnvironmentObject var app: AppModel
    @State private var query = ""
    @State private var joining = false
    /// The host editor: a host's id, or "" for a new one.
    @State private var editing: EditTarget?
    /// A host just deleted, for Undo.
    @State private var deleted: (host: PersonalVault.JSON, label: String)?
    @State private var deletedAt = Date.distantPast

    private struct EditTarget: Identifiable { let id: String }
    @State private var filesHost: Host?
    @State private var ownLogin: Host?
    @State private var snippets = false
    @State private var folderTeam: PendingTeam?

    var body: some View {
        NavigationStack {
            List {
                if query.isEmpty, !app.driveSignIns.isEmpty || !app.folderTeams.isEmpty {
                    Section {
                        ForEach(app.driveSignIns) { need in
                            DriveSignInAgainRow(need: need) { Task { await app.driveSignedIn() } }
                        }
                        ForEach(app.folderTeams) { t in
                            Button { folderTeam = t } label: {
                                LabeledContent {
                                    Text("Open from a cloud drive")
                                } label: {
                                    Label(t.name, systemImage: "person.3")
                                }
                            }
                            .foregroundStyle(.primary)
                        }
                    }
                }
                ForEach(filtered) { section in
                    Section(section.title) {
                        ForEach(section.hosts) { host in
                            Button { app.open(host) } label: { HostRow(host: host) }
                                .swipeActions(edge: .trailing) {
                                    if host.vaultId == "personal" {
                                        Button(role: .destructive) { Task { await delete(host) } } label: { Label("Delete", systemImage: "trash") }
                                        Button { editing = EditTarget(id: host.id) } label: { Label("Edit", systemImage: "pencil") }
                                            .tint(.accentColor)
                                    }
                                }
                                .contextMenu {
                                    Button { app.open(host) } label: { Label("Connect", systemImage: "terminal") }
                                    Button { filesHost = host } label: { Label("Files", systemImage: "folder") }
                                    Button { UIPasteboard.general.string = sshCommand(host) } label: { Label("Copy ssh command", systemImage: "doc.on.doc") }
                                    if host.vaultId != "personal" {
                                        Button { ownLogin = host } label: { Label("My login", systemImage: "person.badge.key") }
                                    }
                                    if host.vaultId == "personal" {
                                        Button { editing = EditTarget(id: host.id) } label: { Label("Edit", systemImage: "pencil") }
                                        Button(role: .destructive) { Task { await delete(host) } } label: { Label("Delete", systemImage: "trash") }
                                    }
                                }
                        }
                    }
                }
            }
            .overlay { empty }
            .overlay(alignment: .bottom) { undoBar }
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
                    Button { editing = EditTarget(id: "") } label: { Image(systemName: "plus") }
                        .accessibilityLabel("New host")
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        if app.sync?.enabled != true {
                            Button { joining = true } label: { Label("Connect to your vault", systemImage: "arrow.triangle.2.circlepath") }
                        }
                        NavigationLink { KeysView() } label: { Label("SSH keys", systemImage: "key") }
                        Button { snippets = true } label: { Label("Snippets", systemImage: "chevron.left.forwardslash.chevron.right") }
                        NavigationLink { KeyBarEditor() } label: { Label("Key bar", systemImage: "keyboard") }
                        NavigationLink { SettingsView() } label: { Label("Settings", systemImage: "gearshape") }
                        Button { Task { await app.lock() } } label: { Label("Lock", systemImage: "lock") }
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                }
            }
            .sheet(isPresented: $joining) { SyncJoinView() }
            .sheet(item: $editing) { t in HostEditor(hostId: t.id.isEmpty ? nil : t.id, saved: { await app.reload() }) }
            .sheet(item: $filesHost) { h in FilesView(host: h) }
            .sheet(item: $ownLogin) { h in OwnLoginEditor(host: h, saved: { await app.reload() }) }
            .sheet(isPresented: $snippets) { SnippetsView(session: nil) }
            .sheet(item: $folderTeam) { t in TeamFromDriveView(team: t) }
            .onAppear { joinIfLinked() }
            .onChange(of: app.pendingJoin) { joinIfLinked() }
        }
    }

    private func delete(_ host: Host) async {
        guard let removed = try? await PersonalVault.remove(host: host.id) else { return }
        await app.reload()
        withAnimation { deleted = (removed, host.label) }
        let at = Date()
        deletedAt = at
        try? await Task.sleep(for: .seconds(5))
        if deletedAt == at { withAnimation { deleted = nil } }
    }

    private func undo() async {
        guard let d = deleted else { return }
        withAnimation { deleted = nil }
        try? await PersonalVault.save(host: d.host)
        await app.reload()
    }

    @ViewBuilder private var undoBar: some View {
        if let d = deleted {
            HStack {
                Text("Deleted \(d.label)").lineLimit(1)
                Spacer()
                Button("Undo") { Task { await undo() } }.bold()
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
            .padding()
            .transition(.move(edge: .bottom).combined(with: .opacity))
        }
    }

    private func sshCommand(_ h: Host) -> String {
        "ssh " + (h.port == 22 ? "" : "-p \(h.port) ") + "\(h.login)@\(h.address)"
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
        if app.sections.isEmpty && app.driveSignIns.isEmpty && app.folderTeams.isEmpty {
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
    @ObservedObject private var terminal = TerminalSettings.shared
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
            Section("Terminal") {
                NavigationLink {
                    ThemePicker()
                } label: {
                    LabeledContent("Theme", value: terminal.themeId == TerminalSettings.system ? String(localized: "As the system") : TerminalTheme.named(terminal.themeId)?.name ?? "")
                }
                Stepper(value: $terminal.fontSize, in: 9...24, step: 1) {
                    LabeledContent("Font size", value: "\(Int(terminal.fontSize))")
                }
                Toggle("Highlight output", isOn: $terminal.highlight)
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

/// The terminal's themes, each shown in its own colours.
private struct ThemePicker: View {
    @ObservedObject private var settings = TerminalSettings.shared

    var body: some View {
        List {
            row(id: TerminalSettings.system, name: String(localized: "As the system"), theme: nil)
            ForEach(TerminalTheme.all) { t in row(id: t.id, name: t.name, theme: t) }
        }
        .navigationTitle("Theme")
    }

    private func row(id: String, name: String, theme: TerminalTheme?) -> some View {
        Button {
            settings.themeId = id
        } label: {
            HStack(spacing: 12) {
                swatch(theme)
                Text(name).foregroundStyle(.primary)
                Spacer()
                if settings.themeId == id { Image(systemName: "checkmark").foregroundStyle(Color.accentColor) }
            }
        }
    }

    /// The background with a prompt in the theme's own colours.
    @ViewBuilder private func swatch(_ theme: TerminalTheme?) -> some View {
        if let theme {
            HStack(spacing: 0) {
                Text("$ ").foregroundStyle(Color(uiColor: theme.foreground))
                Text("~").foregroundStyle(Color(uiColor: UIColor(hex: theme.c[8])))
                Text(" ok").foregroundStyle(Color(uiColor: UIColor(hex: theme.c[6])))
            }
            .font(.system(size: 13, design: .monospaced))
            .frame(width: 64, height: 32)
            .background(Color(uiColor: theme.background), in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.3)))
        } else {
            HStack(spacing: 0) {
                Color(uiColor: TerminalTheme.all[0].background)
                Color(uiColor: TerminalTheme.all[1].background)
            }
            .frame(width: 64, height: 32)
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.3)))
        }
    }
}
