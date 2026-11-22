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
    @State private var ownLogin: Host?
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
                                .tint(.primary)
                                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                                    if host.vaultId == "personal" {
                                        Button(role: .destructive) { Task { await delete(host) } } label: { Label("Delete", systemImage: "trash") }
                                            .tint(.red)
                                        Button { editing = EditTarget(id: host.id) } label: { Label("Edit", systemImage: "pencil") }
                                            .tint(.accentColor)
                                    }
                                }
                                .contextMenu {
                                    Button { app.open(host) } label: { Label("Connect", systemImage: "terminal") }
                                    Button { app.openFiles(host) } label: { Label("Files", systemImage: "folder") }
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
            .safeAreaInset(edge: .bottom) { undoBar }
            .searchable(text: $query)
            .refreshable { await app.syncNow() }
            .navigationTitle("Hosts")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button { editing = EditTarget(id: "") } label: { Image(systemName: "plus") }
                        .accessibilityLabel("New host")
                }
            }
            .sheet(isPresented: $joining) { SyncJoinView() }
            .sheet(item: $editing) { t in HostEditor(hostId: t.id.isEmpty ? nil : t.id, saved: { await app.reload() }) }
            .sheet(item: $ownLogin) { h in OwnLoginEditor(host: h, saved: { await app.reload() }) }
            .sheet(item: $folderTeam) { t in TeamFromDriveView(team: t) }
            .task { await shot() }
            .onAppear { joinIfLinked() }
            .onChange(of: app.pendingJoin) { joinIfLinked() }
        }
    }

    /// Screenshots (debug builds): opens the screen asked for.
    private func shot() async {
        #if DEBUG
        guard let screen = Shots.screen else { return }
        var local: Host?
        for _ in 0..<40 {
            local = app.sections.flatMap(\.hosts).first { $0.label == Shots.localLabel }
            if local != nil { break }
            try? await Task.sleep(for: .milliseconds(250))
        }
        guard let local else { return }
        switch screen {
        case "terminal":
            app.open(local)
            for _ in 0..<60 {
                if app.terminals.first?.state == .connected { break }
                try? await Task.sleep(for: .milliseconds(250))
            }
            try? await Task.sleep(for: .seconds(1))
            app.terminals.first?.run(Shots.terminalCommands)
            // The terminal whole, without the keyboard.
            try? await Task.sleep(for: .seconds(2))
            app.terminals.first?.toggleKeyboard(shown: true)
        case "files": app.openFiles(local)
        case "snippets": app.tab = .snippets
        case "keys": app.tab = .keys
        case "proxies": app.tab = .proxies
        case "settings": app.tab = .settings
        case "keyskb", "editor":
            app.open(local)
            for _ in 0..<60 {
                if app.terminals.first?.state == .connected { break }
                try? await Task.sleep(for: .milliseconds(250))
            }
            try? await Task.sleep(for: .seconds(1))
            app.terminals.first?.run("ls -la")
            app.terminals.first?.showKeysKeyboard(true)
            if screen == "editor" {
                try? await Task.sleep(for: .seconds(1))
                app.terminals.first?.customizing = true
            }
        case "resize", "resizebash":
            // More than a screen of output, then the keyboard up and down: the terminal's rows change.
            app.open(local)
            for _ in 0..<60 {
                if app.terminals.first?.state == .connected { break }
                try? await Task.sleep(for: .milliseconds(250))
            }
            try? await Task.sleep(for: .seconds(1))
            let t = app.terminals.first
            if screen == "resizebash" {
                t?.run("exec /bin/bash --norc")
                try? await Task.sleep(for: .milliseconds(500))
                t?.run("PS1='deploy@build-mac:\\w$ '")
            }
            t?.run("seq 1 120")
            try? await Task.sleep(for: .seconds(1))
            Shots.dump(t, "output, keyboard up")
            // A command at each size, so there are prompts in the history to scroll back to.
            for (i, step) in ["keyboard down", "keyboard up", "keyboard down"].enumerated() {
                t?.toggleKeyboard(shown: step == "keyboard down")
                try? await Task.sleep(for: .milliseconds(1200))
                t?.run("echo step\(i + 1)")
                try? await Task.sleep(for: .milliseconds(800))
                Shots.dump(t, step)
            }
            // Back in the history, where the prompts showed twice.
            t?.view?.scrollUp(lines: 6)
            try? await Task.sleep(for: .milliseconds(800))
            Shots.dump(t, "scrolled up 6")
        case "sessions":
            app.open(local)
            app.openFiles(local)
            try? await Task.sleep(for: .seconds(3))
            app.showTerminals = false
            app.tab = .sessions
        default: break
        }
        #endif
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
