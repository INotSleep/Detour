import SwiftUI

/// Where a vault or a team is kept, as the phone reaches it: a repository over HTTPS (signed
/// in with GitHub, or a token) or a cloud drive's folder.
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

    struct GithubCode: Equatable {
        let code: String
        let uri: String
    }

    var trimmedURL: String { url.trimmingCharacters(in: .whitespaces) }

    /// Everything needed is filled in.
    var ready: Bool {
        switch kind {
        case .drive: folder != nil && driveAccount != nil
        case .git: !trimmedURL.isEmpty && (login == .github ? github != nil : !token.isEmpty)
        }
    }

    /// The repository as the core takes it (`url`, `auth`, `token`, `github`).
    var git: [String: Any?] {
        ["url": trimmedURL, "auth": ["type": "password"], "token": login == .token ? token : nil, "github": login == .github]
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
            RepoLoginSection(model: model)
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
