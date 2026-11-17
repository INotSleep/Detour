import CoreImage.CIFilterBuiltins
import SwiftUI

/// Syncing the personal vault between this person's devices: where, how it goes, another
/// device, the recovery key, turning it off here.
struct SyncView: View {
    @EnvironmentObject var app: AppModel
    @State private var setup: SyncJoinView.Mode?
    @State private var addingDevice = false
    @State private var showingKey = false
    @State private var newLogin = false
    @State private var confirmOff = false
    @State private var replaceKey = false
    @State private var syncing = false
    @State private var error: String?

    var body: some View {
        List {
            if let s = app.sync, s.enabled {
                Section {
                    HStack(spacing: 12) {
                        Image(systemName: s.git ? "arrow.triangle.branch" : s.storage == "folder" ? "folder" : "icloud")
                            .foregroundStyle(Color.accentColor)
                            .frame(width: 24)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(s.location ?? "").font(.callout.monospaced()).lineLimit(2).truncationMode(.middle)
                            Text(status(s)).font(.footnote).foregroundStyle(s.error != nil ? Color.red : s.pending ? Color.orange : Color.secondary)
                        }
                    }
                    fix(s)
                }
                Section {
                    Button { addingDevice = true } label: { Label("Add another device", systemImage: "iphone.and.arrow.forward") }
                    Button { showingKey = true } label: { Label("Recovery key", systemImage: "key") }
                }
                Section {
                    Button(role: .destructive) { confirmOff = true } label: { Label("Turn off on this device", systemImage: "xmark.circle") }
                }
            } else {
                Section {
                    Button { setup = .connect } label: { Label("Connect to your vault", systemImage: "arrow.triangle.2.circlepath") }
                    Button { setup = .create } label: { Label("Turn on sync", systemImage: "plus.circle") }
                }
            }
            if let error {
                Text(error).foregroundStyle(.red).font(.footnote)
            }
        }
        .navigationTitle("Sync")
        .refreshable { await syncNow() }
        .sheet(item: $setup) { m in SyncJoinView(mode: m) }
        .sheet(isPresented: $addingDevice) { AddDeviceSheet(sync: app.sync) }
        .sheet(isPresented: $showingKey) {
            NavigationStack {
                RecoveryKeyView()
                    .navigationTitle("Recovery key")
                    .navigationBarTitleDisplayMode(.inline)
                    .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { showingKey = false } } }
            }
        }
        .sheet(isPresented: $newLogin) { RepoLoginSheet(url: app.sync?.location ?? "") }
        .confirmationDialog("Turn off sync on this device?", isPresented: $confirmOff, titleVisibility: .visible) {
            Button("Turn off", role: .destructive) { Task { await turnOff() } }
        }
        .confirmationDialog("Replace the server key?", isPresented: $replaceKey, titleVisibility: .visible) {
            Button("Replace key", role: .destructive) { Task { await replaceServerKey() } }
        }
    }

    /// The way to put right what stopped the last sync, or syncing now.
    @ViewBuilder private func fix(_ s: SyncStatus) -> some View {
        let e = s.error
        if e?.kind == "driveSignIn", let p = e?.drive {
            DriveSignInAgainRow(need: DriveSignInNeed(provider: p, account: e?.account)) { Task { await app.driveSignedIn() } }
        } else if e?.kind == "hostKeyChanged" {
            Button("Replace server key") { replaceKey = true }
        } else if e?.kind == "git", let m = e?.message, m.contains("token rejected") || m.contains("github sign-in needed") || m.contains("github app has no access") {
            Button("Sign in to the repository again") { newLogin = true }
        } else {
            Button { Task { await syncNow() } } label: {
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

    private func status(_ s: SyncStatus) -> String {
        if let e = s.error { return syncErrorText(e) }
        if syncing { return String(localized: "Syncing…") }
        if s.pending { return String(localized: "Not sent yet") }
        guard let at = s.syncedAt else { return String(localized: "Not synced yet") }
        let ago = RelativeDateTimeFormatter().localizedString(for: Date(timeIntervalSince1970: at / 1000), relativeTo: Date())
        return String(localized: "Synced \(ago)")
    }

    private func syncNow() async {
        syncing = true
        error = nil
        await app.syncNow()
        syncing = false
    }

    private func turnOff() async {
        do {
            _ = try await Core.shared.call("sync_disable")
            await app.reload()
        } catch {
            self.error = coreErrorText(error)
        }
    }

    private func replaceServerKey() async {
        guard let actual = app.sync?.error?.actual else { return }
        do {
            _ = try await Core.shared.call("sync_replace_server_key", ["fingerprint": actual])
            await app.reload()
        } catch {
            self.error = coreErrorText(error)
        }
    }
}

/// A sync error in words.
func syncErrorText(_ e: CoreErrorInfo) -> String {
    switch e.kind {
    case "driveSignIn":
        let name = e.drive?.name ?? ""
        if let a = e.account { return String(localized: "Sign in to \(name) as \(a) again") }
        return String(localized: "Sign in to \(name) again")
    case "driveNoAccess": return String(localized: "The \(e.drive?.name ?? "") account has no access to the folder")
    case "hostKeyChanged": return String(localized: "The server's key has changed")
    case "unreachable", "io": return String(localized: "No connection")
    default: return e.message ?? e.kind
    }
}

/// A one-time code (and its QR) another device of this person's joins sync with.
struct AddDeviceSheet: View {
    let sync: SyncStatus?
    @Environment(\.dismiss) private var dismiss
    @State private var pairing: Pairing?
    @State private var error: String?
    @State private var copies = 0

    struct Pairing: Decodable {
        let code: String
        let expiresAt: Double
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 20) {
                if let pairing {
                    if let qr = QRCode.image(joinLink(pairing.code)) {
                        Image(uiImage: qr)
                            .interpolation(.none)
                            .resizable()
                            .scaledToFit()
                            .frame(maxWidth: 260)
                            .padding(12)
                            .background(Color.white, in: RoundedRectangle(cornerRadius: 12))
                    }
                    Text(pairing.code).font(.title3.monospaced().bold()).textSelection(.enabled)
                    Text("Valid until \(Date(timeIntervalSince1970: pairing.expiresAt / 1000).formatted(date: .omitted, time: .shortened))")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    Button {
                        UIPasteboard.general.string = pairing.code
                        copies += 1
                    } label: {
                        Label(copies > 0 ? String(localized: "Copied") : String(localized: "Copy"), systemImage: copies > 0 ? "checkmark" : "doc.on.doc")
                    }
                    .buttonStyle(.bordered)
                    .sensoryFeedback(.success, trigger: copies)
                } else if let error {
                    Text(error).foregroundStyle(.red)
                } else {
                    ProgressView()
                }
            }
            .padding()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .navigationTitle("New device")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .task {
                do {
                    pairing = try await Core.shared.call("sync_add_device", as: Pairing.self)
                } catch {
                    self.error = coreErrorText(error)
                }
            }
        }
    }

    /// What the other device scans: the code, and where the vault is as it reaches it.
    private func joinLink(_ code: String) -> String {
        var c = URLComponents()
        c.scheme = "detour"
        c.host = "join"
        var items = [URLQueryItem(name: "code", value: code)]
        if let d = sync?.drive, let data = try? JSONEncoder().encode(d), let json = String(data: data, encoding: .utf8) {
            items.append(URLQueryItem(name: "drive", value: json))
        } else if let repo = sync?.location, repo.hasPrefix("https://") || repo.hasPrefix("http://") {
            items.append(URLQueryItem(name: "repo", value: repo))
        }
        c.queryItems = items
        return c.string ?? "detour://join?code=\(code)"
    }
}

/// A new way into the sync repository: GitHub again, or a new token.
struct RepoLoginSheet: View {
    let url: String
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @StateObject private var model = StorageModel()
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section { Text(url).font(.callout.monospaced()) }
                RepoLoginSection(model: model)
                if let error { Text(error).foregroundStyle(.red).font(.footnote) }
            }
            .navigationTitle("Repository login")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else { Button("Save") { Task { await save() } }.disabled(model.login == .github ? model.github == nil : model.token.isEmpty) }
                }
            }
        }
    }

    private func save() async {
        busy = true
        defer { busy = false }
        do {
            if model.login == .github {
                _ = try await Core.shared.call("sync_use_github")
            } else {
                _ = try await Core.shared.call("sync_set_token", ["token": model.token])
            }
            await app.reload()
            dismiss()
        } catch {
            self.error = coreErrorText(error)
        }
    }
}

/// QR codes drawn on the phone.
enum QRCode {
    static func image(_ text: String) -> UIImage? {
        let f = CIFilter.qrCodeGenerator()
        f.message = Data(text.utf8)
        f.correctionLevel = "L"
        guard let out = f.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
              let cg = CIContext().createCGImage(out, from: out.extent) else { return nil }
        return UIImage(cgImage: cg)
    }
}
