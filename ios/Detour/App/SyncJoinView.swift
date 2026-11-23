import SwiftUI

/// Sync for this phone: connects it to the vault the person already syncs (with a code from
/// another device, or the recovery key), or turns sync on with this phone's vault.
struct SyncJoinView: View {
    enum Mode: String, Identifiable {
        case connect, create
        var id: String { rawValue }
    }

    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @StateObject private var storage = StorageModel()
    @State var mode: Mode = .connect
    @State private var code = ""
    @State private var scanning = false
    @State private var busy = false
    @State private var error: String?
    /// Turning on found the person's vault there already.
    @State private var found = false
    /// Turned on: the recovery key to keep.
    @State private var recovery = false

    var body: some View {
        NavigationStack {
            Group {
                if recovery {
                    RecoveryKeyView()
                        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
                } else {
                    form
                }
            }
            .navigationTitle(recovery ? String(localized: "Recovery key") : mode == .create ? String(localized: "Turn on sync") : String(localized: "Connect to your vault"))
            .navigationBarTitleDisplayMode(.inline)
        }
        .interactiveDismissDisabled(recovery)
    }

    private var form: some View {
        Form {
            StorageSections(model: storage)
            if mode == .connect {
                Section {
                    HStack {
                        TextField("XXXX-XXXX-XXXX-XXXX", text: $code)
                            .textInputAutocapitalization(.characters)
                            .autocorrectionDisabled()
                            .font(.body.monospaced())
                        if JoinScanSheet.supported {
                            Button { scanning = true } label: { Image(systemName: "qrcode.viewfinder").font(.title3) }
                                .buttonStyle(.borderless)
                                .accessibilityLabel("Scan the code")
                        }
                    }
                } header: {
                    Text("Code from another device or recovery key")
                } footer: {
                    if found { Text(storage.kind == .drive ? "This folder already has your vault" : "This repository already has your vault") }
                }
            }
            if let error {
                Text(error).foregroundStyle(.red).font(.footnote)
            }
        }
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                if busy {
                    ProgressView()
                } else {
                    Button(mode == .create ? String(localized: "Turn on") : String(localized: "Connect")) { Task { await submit() } }.disabled(!ready)
                }
            }
        }
        .sheet(isPresented: $scanning) { JoinScanSheet { apply($0) } }
        .onAppear { apply(app.pendingJoin) }
        .onChange(of: app.pendingJoin) { _, link in apply(link) }
        .onChange(of: storage.kind) { error = nil; found = false }
    }

    /// Fills the form from the other device's QR.
    private func apply(_ link: JoinLink?) {
        guard let link else { return }
        mode = .connect
        code = link.code
        if let repo = link.repo {
            storage.url = repo
            storage.kind = .git
        }
        if let drive = link.drive {
            storage.kind = .drive
            storage.provider = drive.provider
            storage.folder = drive
            storage.driveAccount = drive.account
        }
        app.pendingJoin = nil
    }

    private var ready: Bool {
        storage.ready && (mode == .create || !code.trimmingCharacters(in: .whitespaces).isEmpty)
    }

    private func submit() async {
        busy = true
        error = nil
        defer { busy = false }
        let code = code.trimmingCharacters(in: .whitespaces)
        do {
            switch (mode, storage.kind) {
            case (.connect, .drive):
                _ = try await Core.shared.call("sync_join_drive", ["drive": storage.drive, "code": code])
            case (.connect, .git):
                var params = storage.git
                params["code"] = code
                _ = try await Core.shared.call("sync_join_git", params)
            case (.create, .drive):
                _ = try await Core.shared.call("sync_enable_drive", ["drive": storage.drive])
            case (.create, .git):
                _ = try await Core.shared.call("sync_enable_git", storage.git)
            }
            await app.reload()
            if mode == .create { recovery = true } else { dismiss() }
        } catch let f as CoreFailure where f.kind == "syncExists" {
            // The person's vault is there already: this phone joins it.
            mode = .connect
            found = true
        } catch {
            self.error = coreErrorText(error, folder: storage.kind == .drive)
        }
    }
}

/// The recovery key of the synced vault: the way back in without another device.
struct RecoveryKeyView: View {
    @State private var key = ""
    @State private var copies = 0

    var body: some View {
        Form {
            Section {
                Text(key.isEmpty ? "…" : key)
                    .font(.callout.monospaced())
                    .textSelection(.enabled)
            }
            Section {
                Button {
                    UIPasteboard.general.string = key
                    Haptics.success()
                    copies += 1
                } label: {
                    Label(copies > 0 ? String(localized: "Copied") : String(localized: "Copy"), systemImage: copies > 0 ? "checkmark" : "doc.on.doc")
                }
                .disabled(key.isEmpty)
                .sensoryFeedback(.success, trigger: copies)
                if !key.isEmpty {
                    ShareLink(item: key) { Label("Save or send", systemImage: "square.and.arrow.up") }
                }
            }
        }
        .task { key = (try? await Core.shared.call("sync_recovery_key") as? String) ?? "" }
    }
}
