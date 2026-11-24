import SwiftUI
import UniformTypeIdentifiers

/// The hosts of one vault: the personal one or a team's (chips at the top switch, and new hosts go
/// to the one shown), in its groups or, in a team, this person's own. A tap connects; a hold opens
/// the menu, and moving the finger then drags the host to another place or group of the vault.
struct HostsView: View {
    @EnvironmentObject var app: AppModel
    @State private var query = ""
    @State private var joining = false
    /// The host editor: a host's id, or "" for a new one.
    @State private var editing: EditTarget?
    /// What was just deleted, for Undo.
    @State private var deleted: Deleted?
    @State private var deletedAt = Date.distantPast
    @State private var ownLogin: Host?
    @State private var folderTeam: PendingTeam?
    /// A group being named: a new one (`from` nil) or one renamed.
    @State private var naming: Naming?
    /// A group whose settings are open.
    @State private var settingsOf: ListedGroup?
    @State private var newName = ""
    /// Why the last change failed, shown for a moment.
    @State private var failure: String?
    /// The vault chip a dragged host is over: hosts don't go there.
    @State private var overVault: String?

    private struct EditTarget: Identifiable { let id: String }
    private struct Naming { let from: ListedGroup? }

    private enum Deleted {
        case host(vault: String, json: VaultData.JSON, label: String)
        case sharedGroup(vault: String, name: String, record: VaultData.JSON?, members: [String])
        case mineGroup(vault: String, layout: [MyGroup], name: String)

        var label: String {
            switch self {
            case .host(_, _, let l), .mineGroup(_, _, let l): l
            case .sharedGroup(_, let n, _, _): n
            }
        }
    }

    /// A row of a group: a host, or what an empty group shows.
    private enum Row: Identifiable {
        case host(Host)
        case empty(String)
        var id: String {
            switch self {
            case .host(let h): h.id
            case .empty(let key): "empty-\(key)"
            }
        }
    }

    private static let dragPrefix = "detour-host:"

    private var vault: VaultHosts? { app.vaults.first { $0.id == app.hostsVault } ?? app.vaults.first }
    private var mine: Bool {
        guard let v = vault, v.team, v.layout != nil else { return false }
        return app.mineViews.contains(v.id)
    }
    /// Everyone arranges their own grouping; the shared one is for those who manage it.
    private var canArrange: Bool { vault.map { mine || $0.canManage } ?? false }
    private var q: String { query.trimmingCharacters(in: .whitespaces).lowercased() }

    private var groups: [ListedGroup] {
        guard let v = vault else { return [] }
        let hosts = q.isEmpty ? v.hosts : v.hosts.filter { "\($0.label) \($0.address) \($0.login) \($0.group)".lowercased().contains(q) }
        if mine, let layout = v.layout { return Groups.listMine(hosts, layout, v.groups, withEmpty: q.isEmpty && canArrange) }
        return Groups.list(hosts, v.groups, withEmpty: q.isEmpty && canArrange)
    }

    private var noneKey: String { mine ? Groups.mineNone : Groups.sharedKey("") }

    private func canDrag(_ h: Host) -> Bool { canArrange && (mine || h.editable) && q.isEmpty }

    var body: some View {
        let groups = self.groups
        let named = groups.contains { !$0.name.isEmpty }
        NavigationStack {
            List {
                if app.vaults.count > 1, let v = vault {
                    Section { picker(v) }
                }
                if query.isEmpty, !app.driveSignIns.isEmpty || !app.folderTeams.isEmpty {
                    Section {
                        ForEach(app.driveSignIns) { need in
                            DriveSignInAgainRow(need: need) { Task { await app.driveSignedIn() } }
                        }
                        ForEach(app.folderTeams) { t in
                            FolderTeamRow(team: t) { folderTeam = t }
                        }
                    }
                }
                ForEach(groups) { g in
                    let ids = g.hosts.map(\.id)
                    Section {
                        ForEach(g.hosts.isEmpty ? [Row.empty(g.key)] : g.hosts.map(Row.host)) { row in
                            switch row {
                            case .host(let h): hostRow(h, in: g, groups: groups)
                            case .empty: Text("Drag hosts here").foregroundStyle(.secondary)
                            }
                        }
                        .onInsert(of: [.plainText]) { i, providers in drop(providers, into: g, ids: ids, at: i, groups: groups) }
                    } header: {
                        header(g, named: named)
                    }
                }
            }
            .animation(.default, value: app.hostsVault)
            .overlay { empty(groups) }
            .safeAreaInset(edge: .bottom) { bottomBars }
            .searchable(text: $query)
            .refreshable { await app.syncNow() }
            .navigationTitle("Hosts")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    if canArrange {
                        Menu {
                            Button { editing = EditTarget(id: "") } label: { Label("New host", systemImage: "server.rack") }
                            Button { newName = ""; naming = Naming(from: nil) } label: { Label("New group", systemImage: "folder.badge.plus") }
                        } label: { Image(systemName: "plus") }
                            .accessibilityLabel("New host")
                    } else {
                        Button { editing = EditTarget(id: "") } label: { Image(systemName: "plus") }
                            .accessibilityLabel("New host")
                    }
                }
            }
            .alert(naming?.from == nil ? Text("New group") : Text("Rename"), isPresented: Binding(get: { naming != nil }, set: { if !$0 { naming = nil } })) {
                TextField("Name", text: $newName)
                Button { commitName() } label: { naming?.from == nil ? Text("Create") : Text("Save") }
                Button("Cancel", role: .cancel) { naming = nil }
            }
            .sheet(isPresented: $joining) { SyncJoinView() }
            .sheet(item: $editing) { t in HostEditor(hostId: t.id.isEmpty ? nil : t.id, vault: vault?.id ?? VaultData.personal, saved: { await app.reload() }) }
            .sheet(item: $ownLogin) { h in OwnLoginEditor(host: h, saved: { await app.reload() }) }
            .sheet(item: $folderTeam) { t in TeamFromDriveView(team: t) }
            .sheet(item: $settingsOf) { g in
                if let v = vault { GroupSettingsView(vault: v, group: g) }
            }
            .task { await shot() }
            .onAppear { joinIfLinked() }
            .onChange(of: app.pendingJoin) { joinIfLinked() }
        }
    }

    /// The vaults as chips, and for a team, its groups or this person's own. A host dragged onto
    /// another vault's chip doesn't go there: the chip locks and says so.
    @ViewBuilder private func picker(_ current: VaultHosts) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(app.vaults) { v in
                    let selected = v.id == current.id
                    Button {
                        Haptics.select()
                        withAnimation { app.showVault(v.id) }
                    } label: {
                        HStack(spacing: 5) {
                            if overVault == v.id { Image(systemName: "lock.fill").font(.caption) }
                            Text(v.name ?? String(localized: "Personal")).lineLimit(1)
                        }
                        .font(.subheadline.weight(selected ? .semibold : .medium))
                        .foregroundStyle(selected ? Color.white : Color.primary)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 7)
                        .background(selected ? Color.accentColor : Color(.tertiarySystemFill), in: Capsule())
                        .opacity(overVault == v.id ? 0.5 : 1)
                    }
                    .buttonStyle(.plain)
                    .dropDestination(for: String.self) { _, _ in
                        overVault = nil
                        return false
                    } isTargeted: { over in
                        guard !selected else { return }
                        withAnimation(.easeOut(duration: 0.15)) {
                            if over { overVault = v.id } else if overVault == v.id { overVault = nil }
                        }
                    }
                }
            }
            .padding(.horizontal, 20)
        }
        .listRowInsets(EdgeInsets(top: 4, leading: 0, bottom: 4, trailing: 0))
        .listRowBackground(Color.clear)
        if current.team {
            Picker("", selection: Binding(get: { app.mineViews.contains(current.id) }, set: { on in
                Task { if let e = await app.setMineView(current, on) { fail(e) } }
            })) {
                Text("Shared groups").tag(false)
                Text("My groups").tag(true)
            }
            .pickerStyle(.segmented)
            .listRowInsets(EdgeInsets(top: 4, leading: 20, bottom: 8, trailing: 20))
            .listRowBackground(Color.clear)
        }
    }

    /// A group's header: its menu behind "…" (and a hold), for a named group.
    @ViewBuilder private func header(_ g: ListedGroup, named: Bool) -> some View {
        let title = g.name.isEmpty ? (named ? String(localized: "No group") : nil) : g.name
        if let title {
            if g.name.isEmpty {
                Text(title)
            } else {
                HStack {
                    Text(title)
                    Spacer()
                    Menu { groupMenu(g) } label: {
                        Image(systemName: "ellipsis")
                            .font(.body)
                            .frame(width: 32, height: 22)
                            .contentShape(Rectangle())
                    }
                    .accessibilityLabel("Settings")
                }
                .contextMenu { groupMenu(g) }
            }
        }
    }

    /// A group's settings (this person's login and proxy for its hosts), and for those who may
    /// arrange it, renaming and deleting it.
    @ViewBuilder private func groupMenu(_ g: ListedGroup) -> some View {
        Button { settingsOf = g } label: { Label("Settings", systemImage: "slider.horizontal.3") }
        if canArrange {
            Button { newName = g.name; naming = Naming(from: g) } label: { Label("Rename", systemImage: "pencil") }
            Button(role: .destructive) { Task { await deleteGroup(g) } } label: { Label("Delete", systemImage: "trash") }
        }
    }

    @ViewBuilder private func hostRow(_ host: Host, in g: ListedGroup, groups: [ListedGroup]) -> some View {
        let row = Button { app.open(host) } label: { HostRow(host: host) }
            .tint(.primary)
            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                if host.editable {
                    Button(role: .destructive) { Task { await delete(host) } } label: { Label("Delete", systemImage: "trash") }
                        .tint(.red)
                    Button { editing = EditTarget(id: host.id) } label: { Label("Edit", systemImage: "pencil") }
                        .tint(.accentColor)
                }
            }
            .contextMenu {
                Button { app.open(host) } label: { Label("Connect", systemImage: "terminal") }
                Button { app.openFiles(host) } label: { Label("Files", systemImage: "folder") }
                Button { UIPasteboard.general.string = sshCommand(host); Haptics.success() } label: { Label("Copy ssh command", systemImage: "doc.on.doc") }
                if host.vaultId != VaultData.personal {
                    Button { ownLogin = host } label: { Label("My login", systemImage: "person.badge.key") }
                }
                if canDrag(host) {
                    // Also without dragging: into another group, last.
                    let targets = groups.filter { $0.key != g.key && !$0.name.isEmpty }
                    Menu {
                        ForEach(targets) { t in
                            Button(t.name) { move(host.id, to: t.key, name: t.name, before: nil, groups: groups) }
                        }
                        if !g.name.isEmpty {
                            Button("No group") { move(host.id, to: noneKey, name: "", before: nil, groups: groups) }
                        }
                    } label: { Label("Move to group", systemImage: "folder") }
                }
                if host.editable {
                    Button { editing = EditTarget(id: host.id) } label: { Label("Edit", systemImage: "pencil") }
                    Button(role: .destructive) { Task { await delete(host) } } label: { Label("Delete", systemImage: "trash") }
                }
            }
        if canDrag(host) {
            row.onDrag { NSItemProvider(object: (Self.dragPrefix + host.id) as NSString) }
        } else {
            row
        }
    }

    /// A host dropped into a group at row `i` (of `ids`, the group's hosts as shown).
    private func drop(_ providers: [NSItemProvider], into g: ListedGroup, ids: [String], at i: Int, groups: [ListedGroup]) {
        guard let p = providers.first, p.canLoadObject(ofClass: NSString.self) else { return }
        let key = g.key, name = g.name, prefix = Self.dragPrefix
        _ = p.loadObject(ofClass: NSString.self) { obj, _ in
            guard let s = obj as? String, s.hasPrefix(prefix) else { return }
            let id = String(s.dropFirst(prefix.count))
            Task { @MainActor in
                overVault = nil
                // Dropped where it was: no move.
                if let at = ids.firstIndex(of: id), i == at || i == at + 1 { return }
                var before = i < ids.count ? ids[i] : nil
                if before == id { before = i + 1 < ids.count ? ids[i + 1] : nil }
                move(id, to: key, name: name, before: before, groups: groups)
            }
        }
    }

    /// Puts a host of the vault shown into a group, before `before` (or last): shown at once, saved behind.
    private func move(_ id: String, to key: String, name: String, before: String?, groups: [ListedGroup]) {
        guard var v = vault, let h = v.hosts.first(where: { $0.id == id }), canDrag(h) else { return }
        Haptics.select()
        if mine, let layout = v.layout {
            let next = Groups.mineMoveHost(layout, groups, host: id, to: key, before: before)
            v.layout = next
            Task {
                do { try await app.showWhileSaving(v) { try await Groups.saveLayout(v.id, next) } } catch { fail(AppModel.describe(error)) }
                await app.reload()
            }
        } else {
            let changes = Groups.moved(v.hosts, host: id, to: name, before: before)
            v.hosts = Groups.applied(v.hosts, changes)
            Task {
                do { try await app.showWhileSaving(v) { try await Groups.moveHosts(v.id, changes) } } catch { fail(AppModel.describe(error)) }
                await app.reload()
            }
        }
    }

    private func fail(_ message: String) {
        Haptics.warning()
        withAnimation { failure = message }
        Task {
            try? await Task.sleep(for: .seconds(4))
            if failure == message { withAnimation { failure = nil } }
        }
    }

    private func commitName() {
        guard let n = naming, let v = vault else { return }
        naming = nil
        let name = newName.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty, name != n.from?.name else { return }
        Task {
            do {
                if mine, let layout = v.layout {
                    if let m = n.from?.mine { try await Groups.saveLayout(v.id, Groups.mineRename(layout, m.id, name)) }
                    else { try await Groups.saveLayout(v.id, Groups.mineAdd(layout, name)) }
                } else if let from = n.from {
                    try await Groups.renameGroup(v.id, from: from.name, to: name)
                } else {
                    try await Groups.addGroup(v.id, name)
                }
            } catch {
                fail(AppModel.describe(error))
            }
            await app.reload()
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
        case "history", "historyzsh":
            // As a person does it: open the host and go straight to the shell's history (no
            // keyboard swaps, which send the size anyway). A command that wraps at the phone's
            // width but not at 80 columns, then a short one, then the arrows.
            app.open(local)
            for _ in 0..<60 {
                if app.terminals.first?.state == .connected { break }
                try? await Task.sleep(for: .milliseconds(100))
            }
            let t = app.terminals.first
            if screen == "history" {
                t?.run("exec /bin/bash --norc")
                try? await Task.sleep(for: .milliseconds(500))
                t?.run("PS1='deploy@build-mac:~$ '")
            }
            try? await Task.sleep(for: .milliseconds(500))
            t?.run("stty size")
            try? await Task.sleep(for: .milliseconds(500))
            t?.run("echo a-command-that-wraps-at-55-not-at-80")
            try? await Task.sleep(for: .milliseconds(500))
            t?.run("ls")
            try? await Task.sleep(for: .milliseconds(500))
            for key in ["up", "up", "down", "up", "up"] {
                t?.input(KeyCodes.bytes(key, appCursor: t?.appCursor ?? false) ?? [])
                try? await Task.sleep(for: .milliseconds(400))
            }
            Shots.dump(t, "history arrows")
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
        do {
            guard let removed = try await VaultData.remove(host: host.id, in: host.vaultId) else { return }
            await app.reload()
            showDeleted(.host(vault: host.vaultId, json: removed, label: host.label))
        } catch {
            fail(AppModel.describe(error))
        }
    }

    private func deleteGroup(_ g: ListedGroup) async {
        guard let v = vault else { return }
        do {
            if let m = g.mine, let layout = v.layout {
                try await Groups.saveLayout(v.id, Groups.mineRemove(layout, m.id))
                showDeleted(.mineGroup(vault: v.id, layout: layout, name: g.name))
            } else {
                let (record, members) = try await Groups.removeGroup(v.id, g.name)
                showDeleted(.sharedGroup(vault: v.id, name: g.name, record: record, members: members))
            }
        } catch {
            fail(AppModel.describe(error))
        }
        await app.reload()
    }

    private func showDeleted(_ d: Deleted) {
        withAnimation { deleted = d }
        let at = Date()
        deletedAt = at
        Task {
            try? await Task.sleep(for: .seconds(5))
            if deletedAt == at { withAnimation { deleted = nil } }
        }
    }

    private func undo() async {
        guard let d = deleted else { return }
        withAnimation { deleted = nil }
        do {
            switch d {
            case .host(let vault, let json, _): try await VaultData.save(host: json, in: vault)
            case .sharedGroup(let vault, let name, let record, let members): try await Groups.restoreGroup(vault, name, record: record, members: members)
            case .mineGroup(let vault, let layout, _): try await Groups.saveLayout(vault, layout)
            }
        } catch {
            fail(AppModel.describe(error))
        }
        await app.reload()
    }

    @ViewBuilder private var bottomBars: some View {
        VStack(spacing: 0) {
            if overVault != nil {
                bar { Label("Can't move hosts between vaults", systemImage: "lock.fill").foregroundStyle(.primary) }
            }
            if let failure {
                bar { Label(failure, systemImage: "exclamationmark.circle").foregroundStyle(.red).lineLimit(3) }
            }
            if let d = deleted {
                bar {
                    Text("Deleted \(d.label)").lineLimit(1)
                    Spacer()
                    Button("Undo") { Task { await undo() } }.bold()
                }
            }
        }
    }

    private func bar(@ViewBuilder _ content: () -> some View) -> some View {
        HStack { content() }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
            .padding(.horizontal)
            .padding(.bottom, 8)
            .transition(.move(edge: .bottom).combined(with: .opacity))
    }

    private func sshCommand(_ h: Host) -> String {
        "ssh " + (h.port == 22 ? "" : "-p \(h.port) ") + "\(h.login)@\(h.address)"
    }

    /// A join link opened the app: the form, filled in from it.
    private func joinIfLinked() {
        guard app.pendingJoin != nil else { return }
        if app.sync?.enabled == true { app.pendingJoin = nil } else { joining = true }
    }

    @ViewBuilder private func empty(_ groups: [ListedGroup]) -> some View {
        if (vault?.hosts.isEmpty ?? true) && app.driveSignIns.isEmpty && app.folderTeams.isEmpty {
            ContentUnavailableView {
                Label("No hosts", systemImage: "server.rack")
            } actions: {
                if app.sync?.enabled != true && vault?.team != true {
                    Button("Connect to your vault") { joining = true }.buttonStyle(.borderedProminent)
                }
            }
        } else if !q.isEmpty && groups.isEmpty {
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
