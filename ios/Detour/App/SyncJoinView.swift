import SwiftUI

/// Connects this phone to the vault the person syncs through a repository, with a code from
/// another device (or the recovery key).
struct SyncJoinView: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL

    enum Login: String, CaseIterable { case github, token }

    @State private var url = ""
    @State private var login: Login = .github
    @State private var token = ""
    @State private var code = ""
    @State private var github: String?
    @State private var signIn: (code: String, uri: String)?
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("https://github.com/you/detour-vault", text: $url)
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                } header: {
                    Text("Repository")
                }
                Section {
                    Picker("Login", selection: $login) {
                        Text("GitHub").tag(Login.github)
                        Text("Token").tag(Login.token)
                    }
                    .pickerStyle(.segmented)
                    if login == .github {
                        if let github {
                            Label(github, systemImage: "checkmark.circle.fill").foregroundStyle(.green)
                        } else if let signIn {
                            Text(signIn.code).font(.title2.monospaced().bold()).textSelection(.enabled)
                            Button("Open GitHub") { openURL(URL(string: signIn.uri)!) }
                        } else {
                            Button("Sign in with GitHub") { Task { await startGithub() } }
                        }
                    } else {
                        SecureField("Access token", text: $token)
                    }
                }
                Section {
                    TextField("XXXX-XXXX-XXXX-XXXX", text: $code)
                        .textInputAutocapitalization(.characters)
                        .autocorrectionDisabled()
                        .font(.body.monospaced())
                } header: {
                    Text("Code from another device or recovery key")
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.footnote)
                }
            }
            .navigationTitle("Connect to your vault")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else { Button("Connect") { Task { await connect() } }.disabled(!ready) }
                }
            }
            .task { github = try? await Core.shared.call("github_user") as? String }
        }
    }

    private var ready: Bool {
        !url.trimmingCharacters(in: .whitespaces).isEmpty && !code.trimmingCharacters(in: .whitespaces).isEmpty && (login == .github ? github != nil : !token.isEmpty)
    }

    private func startGithub() async {
        error = nil
        do {
            let d = try await Core.shared.call("github_start") as? [String: Any] ?? [:]
            guard let device = d["deviceCode"] as? String, let user = d["userCode"] as? String, let uri = d["verificationUri"] as? String else { return }
            signIn = (user, uri)
            UIPasteboard.general.string = user
            openURL(URL(string: uri)!)
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
        } catch let f as CoreFailure {
            signIn = nil
            error = f["message"] ?? f.kind
        } catch {
            signIn = nil
        }
    }

    private func connect() async {
        busy = true
        error = nil
        do {
            _ = try await Core.shared.call("sync_join_git", [
                "url": url.trimmingCharacters(in: .whitespaces),
                "auth": ["type": "password"],
                "token": login == .token ? token : nil,
                "github": login == .github,
                "code": code.trimmingCharacters(in: .whitespaces),
            ])
            await app.reload()
            dismiss()
        } catch let f as CoreFailure {
            switch f.kind {
            case "invalidCode": error = String(localized: "Wrong code")
            case "codeExpired": error = String(localized: "The code has expired: make a new one on the other device")
            case "notASyncVault": error = String(localized: "This repository has no synced vault")
            default: error = f["message"] ?? f.kind
            }
        } catch {
            self.error = error.localizedDescription
        }
        busy = false
    }
}
