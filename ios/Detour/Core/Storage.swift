import SwiftUI

/// Where a vault or a team is kept, as the phone reaches it: a repository over HTTPS (signed
/// in with GitHub, or a token) or over SSH (with a key of the personal vault), or a cloud
/// drive's folder.
@MainActor
final class StorageModel: ObservableObject {
    enum Kind: String { case git, drive }
    enum Login: String { case github, token }

    @Published var kind: Kind = .git
    @Published var url = ""
    @Published var login: Login = .github
    @Published var token = ""
    /// The GitHub account this device is signed in with.
    @Published var github: String?
    /// A GitHub sign-in under way: the code to enter and where.
    @Published var signIn: GithubCode?
    @Published var signInError: String?
    @Published var provider: DriveProvider = .onedrive
    @Published var folder: DriveFolderRef?
    @Published var driveAccount: String?
    /// The personal key a repository over SSH is reached with.
    @Published var keyId: String?

    struct GithubCode: Equatable {
        let code: String
        let uri: String
    }

    var trimmedURL: String { url.trimmingCharacters(in: .whitespaces) }

    /// A repository reached over SSH (`git@host:path`, `ssh://...`): anything not HTTPS.
    var overSSH: Bool {
        let u = trimmedURL.lowercased()
        return !u.isEmpty && !u.hasPrefix("https://") && !u.hasPrefix("http://")
    }

    /// Everything needed is filled in.
    var ready: Bool {
        switch kind {
        case .drive: folder != nil && driveAccount != nil
        case .git: !trimmedURL.isEmpty && (overSSH ? keyId != nil : login == .github ? github != nil : !token.isEmpty)
        }
    }

    /// The repository as the core takes it (`url`, `auth`, `token`, `github`).
    var git: [String: Any?] {
        if overSSH { return ["url": trimmedURL, "auth": ["type": "key", "keyId": keyId ?? ""]] }
        return ["url": trimmedURL, "auth": ["type": "password"], "token": login == .token ? token : nil, "github": login == .github]
    }

    /// The drive folder as the core takes it, reached with the chosen account.
    var drive: [String: Any]? {
        guard var f = folder else { return nil }
        f.account = driveAccount
        return f.json
    }

    func loadGithub() async {
        github = try? await Core.shared.call("github_user") as? String
    }

    /// Signs this device in with GitHub: a code to enter on GitHub's page, then waits.
    func startGithub(open: (URL) -> Void) async {
        signInError = nil
        do {
            let d = try await Core.shared.call("github_start") as? [String: Any] ?? [:]
            guard let device = d["deviceCode"] as? String, let user = d["userCode"] as? String, let uri = d["verificationUri"] as? String else { return }
            signIn = GithubCode(code: user, uri: uri)
            UIPasteboard.general.string = user
            Haptics.success()
            if let u = URL(string: uri) { open(u) }
            var interval = (d["interval"] as? Double) ?? 5
            while signIn != nil {
                try await Task.sleep(for: .seconds(interval))
                let r = try await Core.shared.call("github_poll", ["deviceCode": device]) as? [String: Any] ?? [:]
                switch r["status"] as? String {
                case "done":
                    github = r["user"] as? String
                    signIn = nil
                case "slowDown":
                    interval = (r["interval"] as? Double) ?? interval + 5
                default:
                    break
                }
            }
        } catch {
            signIn = nil
            signInError = coreErrorText(error)
        }
    }
}

/// Choosing the storage: a repository or a cloud drive, and how to get in.
struct StorageSections: View {
    @ObservedObject var model: StorageModel

    var body: some View {
        Section {
            Picker("Storage", selection: $model.kind) {
                Text("Repository").tag(StorageModel.Kind.git)
                Text("Cloud drive").tag(StorageModel.Kind.drive)
            }
            .pickerStyle(.segmented)
        }
        if model.kind == .drive {
            DriveSections(provider: $model.provider, folder: $model.folder, account: $model.driveAccount)
        } else {
            Section {
                TextField("https://github.com/you/detour-vault", text: $model.url)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            } header: {
                Text("Repository")
            }
            if model.overSSH {
                RepoKeySection(model: model)
            } else {
                RepoLoginSection(model: model)
            }
        }
    }
}

/// Getting into a repository over SSH: a key of the personal vault (or a new one), whose public
/// half the repository has to know.
struct RepoKeySection: View {
    @ObservedObject var model: StorageModel
    @State private var keys: [SshKey] = []
    @State private var making = false
    @State private var copies = 0
    @State private var error: String?

    private var chosen: SshKey? { keys.first { $0.id == model.keyId } }

    var body: some View {
        Section {
            if !keys.isEmpty {
                Picker("Key", selection: $model.keyId) {
                    ForEach(keys) { Text($0.name).tag(Optional($0.id)) }
                }
            }
            // The button is always there: the keys load with it.
            Button {
                Task { await make() }
            } label: {
                HStack {
                    Label("New key", systemImage: "plus")
                    if making {
                        Spacer()
                        ProgressView()
                    }
                }
            }
            .disabled(making)
            .task { await load() }
            if let chosen {
                Button {
                    UIPasteboard.general.string = chosen.publicKey
                    Haptics.success()
                    copies += 1
                } label: {
                    LabeledContent("Public key") {
                        Image(systemName: copies > 0 ? "checkmark" : "doc.on.doc")
                            .foregroundStyle(copies > 0 ? Color.green : Color.accentColor)
                    }
                }
                .foregroundStyle(.primary)
                .sensoryFeedback(.success, trigger: copies)
                ShareLink(item: chosen.publicKey + "\n") { Label("Share", systemImage: "square.and.arrow.up") }
            }
            if let error {
                Text(error).foregroundStyle(.red).font(.footnote)
            }
        } footer: {
            if chosen != nil { Text("Add it to the repository as a deploy key with write access") }
        }
    }

    private func load(pick: String? = nil) async {
        keys = (try? await Core.shared.call("vault_get", as: VaultKeys.self).keys) ?? []
        let want = pick ?? model.keyId
        model.keyId = keys.first { $0.id == want }?.id ?? keys.first?.id
    }

    /// A key for this repository, named after it.
    private func make() async {
        making = true
        error = nil
        defer { making = false }
        let repo = model.trimmedURL.split(whereSeparator: { "/:".contains($0) }).last.map { String($0).replacingOccurrences(of: ".git", with: "") }
        let name = (repo?.isEmpty == false ? repo! : "git") + "-iphone"
        do {
            _ = try await Core.shared.call("key_generate", ["vaultId": "personal", "name": name, "algorithm": "ed25519", "remember": true])
            let fresh = (try? await Core.shared.call("vault_get", as: VaultKeys.self).keys) ?? []
            await load(pick: fresh.last { $0.name == name }?.id)
        } catch {
            self.error = coreErrorText(error)
        }
    }
}

/// Getting into a repository: signed in with GitHub, or an access token.
struct RepoLoginSection: View {
    @ObservedObject var model: StorageModel
    @Environment(\.openURL) private var openURL
    @State private var copies = 0

    var body: some View {
        Section {
            Picker("Login", selection: $model.login) {
                Text("GitHub").tag(StorageModel.Login.github)
                Text("Token").tag(StorageModel.Login.token)
            }
            .pickerStyle(.segmented)
            if model.login == .github {
                if let github = model.github {
                    Label(github, systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                } else if let signIn = model.signIn {
                    Button {
                        UIPasteboard.general.string = signIn.code
                        Haptics.success()
                        copies += 1
                    } label: {
                        HStack {
                            Text(signIn.code).font(.title2.monospaced().bold())
                            Spacer()
                            Image(systemName: copies > 0 ? "checkmark" : "doc.on.doc")
                                .foregroundStyle(copies > 0 ? Color.green : Color.accentColor)
                        }
                    }
                    .foregroundStyle(.primary)
                    .accessibilityLabel("Copy")
                    .sensoryFeedback(.success, trigger: copies)
                    Button("Open GitHub") { if let u = URL(string: signIn.uri) { openURL(u) } }
                } else {
                    Button("Sign in with GitHub") { Task { await model.startGithub { openURL($0) } } }
                }
                if let e = model.signInError {
                    Text(e).foregroundStyle(.red).font(.footnote)
                }
            } else {
                SecureField("Access token", text: $model.token)
            }
        }
        .task { await model.loadGithub() }
    }
}

/// What a refusal of the core means, in words.
func coreErrorText(_ e: Error, folder: Bool = false) -> String {
    guard let f = e as? CoreFailure else { return e.localizedDescription }
    switch f.kind {
    case "invalidCode": return String(localized: "Wrong code")
    case "codeExpired": return String(localized: "The code has expired: make a new one on the other device")
    case "notASyncVault": return folder ? String(localized: "This folder has no synced vault") : String(localized: "This repository has no synced vault")
    case "folderNotEmpty": return folder ? String(localized: "This folder is not empty") : String(localized: "This repository is not empty")
    case "notATeamVault": return folder ? String(localized: "This folder has a different team") : String(localized: "This repository has no team vault")
    case "io", "unreachable": return String(localized: "No connection")
    default: return Drives.errorText(f)
    }
}
