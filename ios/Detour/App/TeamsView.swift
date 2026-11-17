import SwiftUI

/// This person's teams on this phone, their Detour ID for an admin to add, and joining one.
struct TeamsView: View {
    @EnvironmentObject var app: AppModel
    @State private var teams: [TeamEntry] = []
    @State private var me: MyId?
    @State private var joining = false
    @State private var copies = 0
    @State private var folderTeam: PendingTeam?

    var body: some View {
        List {
            if let me {
                Section {
                    Button {
                        UIPasteboard.general.string = me.detourId
                        copies += 1
                    } label: {
                        HStack {
                            Text(me.detourId).font(.footnote.monospaced()).foregroundStyle(.primary).lineLimit(2).truncationMode(.middle)
                            Spacer()
                            Image(systemName: copies > 0 ? "checkmark" : "doc.on.doc").foregroundStyle(copies > 0 ? Color.green : Color.accentColor)
                        }
                    }
                    .sensoryFeedback(.success, trigger: copies)
                    LabeledContent("Fingerprint", value: me.fingerprint).font(.footnote.monospaced())
                    ShareLink(item: me.detourId) { Label("Send", systemImage: "square.and.arrow.up") }
                } header: {
                    Text("My Detour ID")
                }
            }
            if !teams.isEmpty || !app.folderTeams.isEmpty {
                Section("Teams") {
                    ForEach(teams, id: \.id) { t in
                        if let v = t.view {
                            NavigationLink { TeamDetailView(team: v, reload: load) } label: { TeamRow(team: v) }
                        } else {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(t.path).lineLimit(1).truncationMode(.middle)
                                if let e = t.error { Text(e).font(.footnote).foregroundStyle(.red) }
                            }
                        }
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
        }
        .navigationTitle("Teams")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { joining = true } label: { Image(systemName: "plus") }
                    .accessibilityLabel("Join a team")
            }
        }
        .overlay {
            if teams.isEmpty && app.folderTeams.isEmpty && me != nil {
                ContentUnavailableView {
                    Label("No teams", systemImage: "person.3")
                } actions: {
                    Button("Join a team") { joining = true }.buttonStyle(.borderedProminent)
                }
                .padding(.top, 220)
            }
        }
        .refreshable {
            await app.syncNow()
            await load()
        }
        .task { await load() }
        .sheet(isPresented: $joining, onDismiss: { Task { await load() } }) { JoinTeamView() }
        .sheet(item: $folderTeam, onDismiss: { Task { await load() } }) { t in TeamFromDriveView(team: t) }
    }

    private func load() async {
        me = try? await Core.shared.call("my_id", as: MyId.self)
        teams = (try? await Core.shared.call("teams_list", as: [TeamEntry].self)) ?? []
    }
}

private struct TeamRow: View {
    let team: TeamEntry.TeamView

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(team.name).font(.body.weight(.medium))
            if team.joined {
                Text("\(team.members.count) members · \(team.data.hosts.count) hosts").font(.footnote).foregroundStyle(.secondary)
            } else {
                Text("Waiting for an admin to add your Detour ID").font(.footnote).foregroundStyle(.orange)
            }
        }
    }
}

/// A team: where it is kept, how syncing it goes, its members, removing it from the devices.
struct TeamDetailView: View {
    let team: TeamEntry.TeamView
    let reload: () async -> Void
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var current: TeamEntry.TeamView?
    @State private var syncing = false
    @State private var confirmRemove = false
    @State private var error: String?

    private var t: TeamEntry.TeamView { current ?? team }

    var body: some View {
        List {
            if let r = t.remote {
                Section {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(r.url).font(.callout.monospaced()).lineLimit(2).truncationMode(.middle)
                        Text(status(r)).font(.footnote).foregroundStyle(r.error != nil ? Color.red : r.pending ? Color.orange : Color.secondary)
                    }
                    if r.error?.kind == "driveSignIn", let p = r.error?.drive {
                        DriveSignInAgainRow(need: DriveSignInNeed(provider: p, account: r.error?.account)) { Task { await sync() } }
                    } else {
                        Button { Task { await sync() } } label: {
                            HStack {
                                Text("Sync now")
                                if syncing {
                                    Spacer()
                                    ProgressView()
                                }
                            }
                        }
                        .disabled(syncing)
                    }
                }
            }
            if !t.joined {
                Section { Text("Waiting for an admin to add your Detour ID").foregroundStyle(.orange) }
            }
            Section("Members") {
                ForEach(t.members, id: \.id) { m in
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(m.isYou ? String(localized: "\(m.name) (you)") : m.name)
                            Text(m.fingerprint).font(.caption.monospaced()).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Text(roleName(m.role)).font(.footnote).foregroundStyle(.secondary)
                    }
                }
            }
            Section {
                Button(role: .destructive) { confirmRemove = true } label: { Label("Remove from all devices", systemImage: "trash") }
            }
            if let error {
                Text(error).foregroundStyle(.red).font(.footnote)
            }
        }
        .navigationTitle(t.name)
        .refreshable { await sync() }
        .confirmationDialog("Remove \"\(t.name)\" from all your devices?", isPresented: $confirmRemove, titleVisibility: .visible) {
            Button("Remove", role: .destructive) { Task { await remove() } }
        } message: {
            Text("The team vault and your membership stay, you can reconnect anytime.")
        }
    }

    private func status(_ r: TeamEntry.Remote) -> String {
        if let e = r.error { return syncErrorText(e) }
        if syncing { return String(localized: "Syncing…") }
        if r.pending { return String(localized: "Not sent yet") }
        guard let at = r.syncedAt else { return String(localized: "Not synced yet") }
        let ago = RelativeDateTimeFormatter().localizedString(for: Date(timeIntervalSince1970: at / 1000), relativeTo: Date())
        return String(localized: "Synced \(ago)")
    }

    private func roleName(_ role: String) -> String {
        switch role {
        case "owner": String(localized: "Owner")
        case "admin": String(localized: "Admin")
        default: String(localized: "Member")
        }
    }

    private func sync() async {
        syncing = true
        error = nil
        do {
            current = try await Core.shared.call("team_sync", ["id": t.id], as: TeamEntry.TeamView.self)
        } catch {
            self.error = coreErrorText(error)
        }
        syncing = false
        await app.reload()
        await reload()
    }

    private func remove() async {
        do {
            _ = try await Core.shared.call("team_leave", ["id": t.id])
            await app.reload()
            await reload()
            dismiss()
        } catch {
            self.error = coreErrorText(error)
        }
    }
}

/// Joining a team an admin added this person to: its repository or drive folder, the creator's
/// fingerprint to compare, then joined.
struct JoinTeamView: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @StateObject private var storage = StorageModel()
    @State private var found: Found?
    @State private var me: MyId?
    @State private var busy = false
    @State private var error: String?

    struct Found {
        let root: String
        let rootFingerprint: String
        let clone: String
    }

    private struct Inspected: Decodable {
        let info: Info
        let clone: String
        struct Info: Decodable {
            let root: String
            let rootFingerprint: String
        }
    }

    var body: some View {
        NavigationStack {
            Form {
                StorageSections(model: storage)
                if let found {
                    Section {
                        LabeledContent("Vault creator", value: found.rootFingerprint).font(.callout.monospaced())
                        if let me { LabeledContent("Your fingerprint", value: me.fingerprint).font(.footnote.monospaced()) }
                    } footer: {
                        Text("Compare the fingerprint with the team's creator")
                    }
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.footnote)
                }
            }
            .navigationTitle("Join team")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy {
                        ProgressView()
                    } else {
                        Button(found == nil ? String(localized: "Next") : String(localized: "Trust and join")) { Task { await submit() } }.disabled(!storage.ready)
                    }
                }
            }
            .onChange(of: storage.kind) { found = nil; error = nil }
            .onChange(of: storage.url) { found = nil }
            .onChange(of: storage.folder) { found = nil }
            .task { me = try? await Core.shared.call("my_id", as: MyId.self) }
        }
    }

    private func submit() async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            if let found {
                if storage.kind == .drive {
                    _ = try await Core.shared.call("team_join_drive", ["clone": found.clone, "root": found.root, "drive": storage.drive])
                } else {
                    var p = storage.git
                    p["clone"] = found.clone
                    p["root"] = found.root
                    _ = try await Core.shared.call("team_join_git", p)
                }
                await app.reload()
                dismiss()
            } else {
                let r: Inspected = storage.kind == .drive
                    ? try await Core.shared.call("team_inspect_drive", ["drive": storage.drive], as: Inspected.self)
                    : try await Core.shared.call("team_inspect_git", storage.git, as: Inspected.self)
                found = Found(root: r.info.root, rootFingerprint: r.info.rootFingerprint, clone: r.clone)
            }
        } catch {
            self.error = coreErrorText(error, folder: storage.kind == .drive)
        }
    }
}
