import AuthenticationServices
import SwiftUI

/// This device's sign-in to a drive: the account, or a button that opens the drive's sign-in
/// (and a field for the code, when the drive's page shows one to paste).
struct DriveSignInRow: View {
    let provider: DriveProvider
    @Binding var account: String?
    var signedIn: (String) -> Void = { _ in }
    @Environment(\.webAuthenticationSession) private var webAuth
    @State private var busy = false
    @State private var paste = false
    @State private var code = ""
    @State private var error: String?

    var body: some View {
        Group {
            if let account {
                Label(account, systemImage: "checkmark.circle.fill").foregroundStyle(.green)
            } else if paste {
                HStack {
                    TextField("Code from the page", text: $code)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.body.monospaced())
                        .onSubmit { Task { await finishCode() } }
                    Button("Done") { Task { await finishCode() } }
                        .buttonStyle(.borderless)
                        .disabled(code.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            } else {
                Button { Task { await signIn() } } label: {
                    HStack {
                        Text("Sign in to \(provider.name)")
                        if busy {
                            Spacer()
                            ProgressView()
                        }
                    }
                }
                .disabled(busy)
            }
            if let error {
                Text(error).foregroundStyle(.red).font(.footnote)
            }
        }
        .task(id: provider) {
            paste = false
            error = nil
            account = await Drives.accounts()[provider]
        }
    }

    private func signIn() async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            let start = try await Drives.start(provider)
            do {
                let back = try await webAuth.authenticate(using: start.url, callbackURLScheme: "detour")
                let ok = URLComponents(url: back, resolvingAgainstBaseURL: false)?.queryItems?.first { $0.name == "ok" }?.value
                if ok == "0" {
                    error = String(localized: "The sign-in did not go through")
                    return
                }
            } catch let e where ASWebAuthenticationSessionError.isCancel(e) {
                // The page showed a code instead of coming back (the drive's port was busy).
                if start.paste { paste = true }
                return
            }
            done(try await Drives.finish(provider))
        } catch {
            self.error = Drives.errorText(error)
        }
    }

    private func finishCode() async {
        let c = code.trimmingCharacters(in: .whitespaces)
        guard !c.isEmpty else { return }
        do {
            done(try await Drives.finish(provider, code: c))
        } catch {
            self.error = Drives.errorText(error)
        }
    }

    private func done(_ who: String?) {
        guard let who else {
            error = String(localized: "The sign-in did not go through")
            return
        }
        paste = false
        code = ""
        account = who
        signedIn(who)
    }
}

/// Which drive, the sign-in to it, and the folder there: picked from its folders, or opened
/// from a share link.
struct DriveSections: View {
    @Binding var provider: DriveProvider
    @Binding var folder: DriveFolderRef?
    @Binding var account: String?

    var body: some View {
        Section {
            Picker("Service", selection: $provider) {
                ForEach(DriveProvider.allCases) { Text($0.name).tag($0) }
            }
            DriveSignInRow(provider: provider, account: $account)
        }
        .onChange(of: provider) { _, p in
            if folder?.provider != p { folder = nil }
        }
        if account != nil {
            Section {
                DriveFolderRows(provider: provider, folder: $folder)
            }
        }
    }
}

/// The folder: chosen in the drive, or opened from a link to a shared folder.
struct DriveFolderRows: View {
    let provider: DriveProvider
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
                DriveBrowser(provider: provider, parent: nil) { f in
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
            folder = try await Core.shared.call("drive_link", ["url": url, "provider": provider.rawValue], as: DriveFolderRef.self)
            link = ""
        } catch {
            self.error = Drives.errorText(error)
        }
    }
}

/// A drive's folders, one level: walk into one, make one, or use this one.
struct DriveBrowser: View {
    let provider: DriveProvider
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
        .navigationDestination(item: $opened) { f in DriveBrowser(provider: provider, parent: f, pick: pick) }
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
        return DriveFolderRef(provider: provider, id: f.id, drive: f.drive ?? listing?.folder.drive, label: label, link: nil)
    }

    private func load() async {
        error = nil
        do {
            listing = try await Core.shared.call("drive_browse", ["provider": provider.rawValue, "folder": parent?.json], as: DriveListing.self)
        } catch {
            self.error = Drives.errorText(error)
        }
    }

    private func create() async {
        let name = newName.trimmingCharacters(in: .whitespaces)
        newName = ""
        guard !name.isEmpty else { return }
        do {
            opened = try await Core.shared.call("drive_folder_create", ["provider": provider.rawValue, "parent": parent?.json, "name": name], as: DriveFolderRef.self)
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
        guard let folder else { return }
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
