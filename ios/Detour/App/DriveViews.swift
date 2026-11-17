import AuthenticationServices
import SwiftUI

/// Signing in to a drive: the drive's page in the system's sign-in sheet, back in the app once
/// the drive has answered (or a code from the page to paste, when the drive's port was busy).
@MainActor
final class DriveSignInFlow: ObservableObject {
    @Published var busy = false
    @Published var paste = false
    @Published var code = ""
    @Published var error: String?

    /// The account signed in to, or nil (cancelled, refused, or a code to paste first).
    func signIn(_ p: DriveProvider, with webAuth: WebAuthenticationSession) async -> String? {
        busy = true
        error = nil
        defer { busy = false }
        do {
            let start = try await Drives.start(p)
            do {
                let back = try await webAuth.authenticate(using: start.url, callbackURLScheme: "detour")
                let ok = URLComponents(url: back, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "ok" }?.value
                if ok == "0" {
                    error = String(localized: "The sign-in did not go through")
                    return nil
                }
            } catch let e where ASWebAuthenticationSessionError.isCancel(e) {
                if start.paste { paste = true }
                return nil
            }
            return done(try await Drives.finish(p))
        } catch {
            self.error = Drives.errorText(error)
            return nil
        }
    }

    func finishCode(_ p: DriveProvider) async -> String? {
        let c = code.trimmingCharacters(in: .whitespaces)
        guard !c.isEmpty else { return nil }
        do {
            return done(try await Drives.finish(p, code: c))
        } catch {
            self.error = Drives.errorText(error)
            return nil
        }
    }

    private func done(_ who: String?) -> String? {
        if who == nil { error = String(localized: "The sign-in did not go through") }
        paste = false
        code = ""
        return who
    }
}

/// The field for a code the drive's page showed.
private struct DriveCodeField: View {
    @ObservedObject var flow: DriveSignInFlow
    let finish: () -> Void

    var body: some View {
        HStack {
            TextField("Code from the page", text: $flow.code)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.body.monospaced())
                .onSubmit(finish)
            Button("Done", action: finish)
                .buttonStyle(.borderless)
                .disabled(flow.code.trimmingCharacters(in: .whitespaces).isEmpty)
        }
    }
}

/// The account of a drive a vault is reached with: one this device is signed in to, another
/// one added, or signed out of. `account` is the chosen one while it is signed in.
struct DriveAccountRow: View {
    let provider: DriveProvider
    @Binding var account: String?
    @Environment(\.webAuthenticationSession) private var webAuth
    @StateObject private var flow = DriveSignInFlow()
    @State private var accounts: [DriveAccount] = []
    @State private var chosen: String?
    @State private var loaded = false

    private var current: DriveAccount? { accounts.first { $0.account == chosen } }

    var body: some View {
        Group {
            if flow.paste {
                DriveCodeField(flow: flow) { Task { await added(flow.finishCode(provider)) } }
            } else if let current {
                Menu {
                    ForEach(accounts, id: \.account) { a in
                        Button { chosen = a.account } label: {
                            if a.account == current.account { Label(a.account, systemImage: "checkmark") } else { Text(a.account) }
                        }
                    }
                    Button { Task { await added(flow.signIn(provider, with: webAuth)) } } label: { Label("Add account", systemImage: "plus") }
                    Divider()
                    Button(role: .destructive) { Task { await signOut(current.account) } } label: { Label("Sign out", systemImage: "rectangle.portrait.and.arrow.right") }
                } label: {
                    LabeledContent("Account") {
                        HStack(spacing: 6) {
                            if flow.busy { ProgressView() }
                            Text(current.account).lineLimit(1).truncationMode(.middle)
                            Image(systemName: "chevron.up.chevron.down").font(.caption)
                        }
                    }
                }
                .foregroundStyle(.primary)
                if !current.signedIn {
                    Button("Sign in to \(provider.name) again") { Task { await added(flow.signIn(provider, with: webAuth)) } }
                        .disabled(flow.busy)
                }
            } else if loaded {
                Button { Task { await added(flow.signIn(provider, with: webAuth)) } } label: {
                    HStack {
                        Text("Sign in to \(provider.name)")
                        if flow.busy {
                            Spacer()
                            ProgressView()
                        }
                    }
                }
                .disabled(flow.busy)
            }
            if let error = flow.error {
                Text(error).foregroundStyle(.red).font(.footnote)
            }
        }
        .task(id: provider) {
            flow.paste = false
            flow.error = nil
            await load(pick: account)
            loaded = true
        }
        .onChange(of: chosen) { publish() }
        .onChange(of: accounts.map(\.signedIn)) { publish() }
    }

    private func publish() {
        account = current?.signedIn == true ? current?.account : nil
    }

    private func load(pick: String?) async {
        accounts = await Drives.accounts(of: provider)
        let want = pick ?? chosen
        chosen = accounts.first { $0.account == want }?.account ?? accounts.first(where: \.signedIn)?.account ?? accounts.first?.account
        publish()
    }

    private func added(_ who: String?) async {
        guard let who else { return }
        await load(pick: who)
    }

    private func signOut(_ who: String) async {
        _ = try? await Core.shared.call("drive_sign_out", ["provider": provider.rawValue, "account": who])
        chosen = nil
        await load(pick: nil)
    }
}

/// Signing in to a drive again, where something kept there asks for it.
struct DriveSignInAgainRow: View {
    let need: DriveSignInNeed
    let done: () -> Void
    @Environment(\.webAuthenticationSession) private var webAuth
    @StateObject private var flow = DriveSignInFlow()

    var body: some View {
        if flow.paste {
            DriveCodeField(flow: flow) { Task { if await flow.finishCode(need.provider) != nil { done() } } }
        } else {
            Button {
                Task { if await flow.signIn(need.provider, with: webAuth) != nil { done() } }
            } label: {
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Sign in to \(need.provider.name) again")
                        if let a = need.account { Text(a).font(.caption).foregroundStyle(.secondary) }
                    }
                    if flow.busy {
                        Spacer()
                        ProgressView()
                    }
                }
            }
            .disabled(flow.busy)
        }
        if let error = flow.error {
            Text(error).foregroundStyle(.red).font(.footnote)
        }
    }
}

/// Which drive, the account there, and the folder: picked from its folders, or opened from a
/// share link. The folder carries the account it is reached with.
struct DriveSections: View {
    @Binding var provider: DriveProvider
    @Binding var folder: DriveFolderRef?
    @Binding var account: String?

    var body: some View {
        Section {
            Picker("Service", selection: $provider) {
                ForEach(DriveProvider.allCases) { Text($0.name).tag($0) }
            }
            DriveAccountRow(provider: provider, account: $account)
        }
        .onChange(of: provider) { _, p in
            if folder?.provider != p { folder = nil }
        }
        .onChange(of: account) { _, a in
            if let a, folder != nil, folder?.account != a { folder?.account = a }
        }
        if let account {
            Section {
                DriveFolderRows(provider: provider, account: account, folder: $folder)
            }
        }
    }
}

/// The folder: chosen in the drive, or opened from a link to a shared folder.
struct DriveFolderRows: View {
    let provider: DriveProvider
    let account: String
    @Binding var folder: DriveFolderRef?
    @State private var browsing = false
    @State private var link = ""
    @State private var opening = false
    @State private var error: String?

    var body: some View {
        Button { browsing = true } label: {
            LabeledContent("Folder") {
                Text(folder?.title ?? String(localized: "Choose"))
                    .foregroundStyle(folder == nil ? Color.accentColor : Color.secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
            }
        }
        .foregroundStyle(.primary)
        .sheet(isPresented: $browsing) {
            NavigationStack {
                DriveBrowser(provider: provider, account: account, parent: nil) { f in
                    folder = f
                    browsing = false
                }
            }
        }
        HStack {
            TextField("Or a link to a shared folder", text: $link)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .onSubmit { Task { await openLink() } }
            if opening {
                ProgressView()
            } else if !link.trimmingCharacters(in: .whitespaces).isEmpty {
                Button("Open") { Task { await openLink() } }.buttonStyle(.borderless)
            }
        }
        if let error {
            Text(error).foregroundStyle(.red).font(.footnote)
        }
    }

    private func openLink() async {
        let url = link.trimmingCharacters(in: .whitespaces)
        guard !url.isEmpty, !opening else { return }
        opening = true
        error = nil
        defer { opening = false }
        do {
            folder = try await Core.shared.call("drive_link", ["url": url, "provider": provider.rawValue, "account": account], as: DriveFolderRef.self)
            link = ""
        } catch {
            self.error = Drives.errorText(error)
        }
    }
}

/// A drive's folders, one level: walk into one, make one, or use this one.
struct DriveBrowser: View {
    let provider: DriveProvider
    let account: String
    let parent: DriveFolderRef?
    let pick: (DriveFolderRef) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var listing: DriveListing?
    @State private var error: String?
    @State private var opened: DriveFolderRef?
    @State private var naming = false
    @State private var newName = ""

    var body: some View {
        List {
            ForEach(listing?.folders ?? [], id: \.key) { f in
                Button { opened = child(f) } label: {
                    HStack {
                        Image(systemName: f.shared ? "person.2" : "folder").foregroundStyle(Color.accentColor)
                        Text(f.name).foregroundStyle(.primary).lineLimit(1)
                        Spacer()
                        if f.shared { Text("Shared").font(.footnote).foregroundStyle(.secondary) }
                        Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
                    }
                }
                .tint(.primary)
            }
        }
        .overlay {
            if let error {
                ContentUnavailableView {
                    Label(error, systemImage: "exclamationmark.triangle")
                } actions: {
                    Button("Try again") { Task { await load() } }
                }
            } else if listing == nil {
                ProgressView()
            } else if listing?.folders.isEmpty == true {
                ContentUnavailableView("No folders here", systemImage: "folder")
            }
        }
        .navigationTitle(parent.map { $0.label.split(separator: "/").last.map(String.init) ?? $0.label } ?? provider.name)
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(item: $opened) { f in DriveBrowser(provider: provider, account: account, parent: f, pick: pick) }
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                if parent == nil { Button("Cancel") { dismiss() } }
            }
            ToolbarItem(placement: .primaryAction) {
                Button { naming = true } label: { Image(systemName: "folder.badge.plus") }
                    .accessibilityLabel("New folder")
            }
            ToolbarItem(placement: .bottomBar) {
                if let parent {
                    Button("Use this folder") { pick(parent) }
                        .buttonStyle(.borderedProminent)
                        .frame(maxWidth: .infinity)
                }
            }
        }
        .alert("New folder", isPresented: $naming) {
            TextField("Name", text: $newName)
            Button("Cancel", role: .cancel) { newName = "" }
            Button("Create") { Task { await create() } }
        }
        .task { await load() }
    }

    private func child(_ f: DriveListing.Entry) -> DriveFolderRef {
        let label = f.shared || parent == nil ? f.name : "\(parent!.label)/\(f.name)"
        return DriveFolderRef(provider: provider, id: f.id, drive: f.drive ?? listing?.folder.drive, label: label, link: nil, account: account)
    }

    private func load() async {
        error = nil
        do {
            listing = try await Core.shared.call("drive_browse", ["provider": provider.rawValue, "folder": parent?.json, "account": account], as: DriveListing.self)
        } catch {
            self.error = Drives.errorText(error)
        }
    }

    private func create() async {
        let name = newName.trimmingCharacters(in: .whitespaces)
        newName = ""
        guard !name.isEmpty else { return }
        do {
            opened = try await Core.shared.call("drive_folder_create", ["provider": provider.rawValue, "parent": parent?.json, "name": name, "account": account], as: DriveFolderRef.self)
            await load()
        } catch {
            self.error = Drives.errorText(error)
        }
    }
}

/// A team joined on another device that keeps it in a folder its drive's own app syncs: opened
/// here through the drive.
struct TeamFromDriveView: View {
    let team: PendingTeam
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @State private var provider: DriveProvider = .onedrive
    @State private var folder: DriveFolderRef?
    @State private var account: String?
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                DriveSections(provider: $provider, folder: $folder, account: $account)
                if let error {
                    Text(error).foregroundStyle(.red).font(.footnote)
                }
            }
            .navigationTitle(team.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else { Button("Open") { Task { await open() } }.disabled(folder == nil || account == nil) }
                }
            }
        }
    }

    private func open() async {
        guard var folder else { return }
        folder.account = account
        busy = true
        error = nil
        defer { busy = false }
        do {
            _ = try await Core.shared.call("team_join_linked_drive", ["teamId": team.teamId, "drive": folder.json])
            await app.reload()
            dismiss()
        } catch {
            self.error = Drives.errorText(error)
        }
    }
}
