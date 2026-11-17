import SwiftUI
import UniformTypeIdentifiers

/// The personal vault's SSH keys: make one, import one, copy or share its public key, delete it.
struct KeysView: View {
    @State private var vault: VaultKeys?
    @State private var making = false
    @State private var importing = false
    @State private var error: String?

    var body: some View {
        List {
            if let vault {
                ForEach(vault.keys) { key in
                    NavigationLink { KeyDetail(key: key, users: vault.users(of: key), changed: { await load() }) } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(key.name).font(.body.weight(.medium))
                            Text(key.kind + (key.hasPassphrase ? " · " + String(localized: "passphrase") : ""))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .overlay {
            if vault == nil {
                ProgressView()
            } else if vault?.keys.isEmpty == true {
                ContentUnavailableView {
                    Label("No keys", systemImage: "key")
                } actions: {
                    Button("New key") { making = true }.buttonStyle(.borderedProminent)
                    Button("Import a key") { importing = true }
                }
            }
        }
        .navigationTitle("SSH keys")
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button { making = true } label: { Label("New key", systemImage: "plus") }
                    Button { importing = true } label: { Label("Import a key", systemImage: "square.and.arrow.down") }
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityLabel("Add")
            }
        }
        .sheet(isPresented: $making) { NewKeySheet(changed: { await load() }) }
        .sheet(isPresented: $importing) { ImportKeySheet(changed: { await load() }) }
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        vault = try? await Core.shared.call("vault_get", as: VaultKeys.self)
    }
}

private struct KeyDetail: View {
    let key: SshKey
    let users: [String]
    let changed: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var copied = 0
    @State private var confirmDelete = false
    @State private var error: String?

    var body: some View {
        List {
            Section {
                Text(key.publicKey)
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                    .lineLimit(6)
                Button {
                    UIPasteboard.general.string = key.publicKey
                    copied += 1
                } label: {
                    Label(copied > 0 ? "Copied" : "Copy the public key", systemImage: copied > 0 ? "checkmark" : "doc.on.doc")
                }
                .sensoryFeedback(.success, trigger: copied)
                ShareLink(item: key.publicKey + "\n") { Label("Share", systemImage: "square.and.arrow.up") }
            } header: {
                Text("Public key")
            }
            Section {
                LabeledContent("Type", value: key.kind)
                LabeledContent("Fingerprint") {
                    Text(key.fingerprint).font(.caption.monospaced()).multilineTextAlignment(.trailing)
                }
                if !users.isEmpty {
                    LabeledContent("Hosts", value: users.joined(separator: ", "))
                }
            }
            Section {
                Button("Delete the key", role: .destructive) { confirmDelete = true }
                if let error { Text(error).foregroundStyle(.red).font(.footnote) }
            }
        }
        .navigationTitle(key.name)
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog("Delete \(key.name)?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive) { Task { await delete() } }
        }
    }

    private func delete() async {
        do {
            try await PersonalVault.remove(key: key.id)
            await changed()
            dismiss()
        } catch let f as CoreFailure where f.kind == "keyInUse" {
            error = String(localized: "Hosts log in with this key: \(users.joined(separator: ", "))")
        } catch let f as CoreFailure {
            error = f["message"] ?? f.kind
        } catch {
            self.error = error.localizedDescription
        }
    }
}

/// A new Ed25519 key, its public key shown at once to put on a server.
private struct NewKeySheet: View {
    let changed: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var made: SshKey?
    @State private var busy = false
    @State private var error: String?
    @State private var copied = 0

    var body: some View {
        NavigationStack {
            Form {
                if let made {
                    Section {
                        Text(made.publicKey).font(.caption.monospaced()).textSelection(.enabled)
                        Button {
                            UIPasteboard.general.string = made.publicKey
                            copied += 1
                        } label: {
                            Label(copied > 0 ? "Copied" : "Copy the public key", systemImage: copied > 0 ? "checkmark" : "doc.on.doc")
                        }
                        .sensoryFeedback(.success, trigger: copied)
                        ShareLink(item: made.publicKey + "\n") { Label("Share", systemImage: "square.and.arrow.up") }
                    } header: {
                        Text("Public key")
                    }
                } else {
                    Section {
                        TextField("Name", text: $name)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .onSubmit { Task { await make() } }
                    } footer: {
                        Text("Ed25519")
                    }
                    if let error { Text(error).foregroundStyle(.red).font(.footnote) }
                }
            }
            .navigationTitle(made.map { Text($0.name) } ?? Text("New key"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if made == nil {
                    ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                    ToolbarItem(placement: .confirmationAction) {
                        if busy { ProgressView() } else { Button("Create") { Task { await make() } } }
                    }
                } else {
                    ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
                }
            }
        }
        .onAppear { if name.isEmpty { name = "iphone" } }
    }

    private func make() async {
        let n = name.trimmingCharacters(in: .whitespaces)
        guard !n.isEmpty, !busy else { return }
        busy = true
        error = nil
        do {
            _ = try await Core.shared.call("key_generate", ["vaultId": "personal", "name": n, "algorithm": "ed25519", "remember": true])
            let keys = try await Core.shared.call("vault_get", as: VaultKeys.self).keys
            made = keys.first { $0.name == n } ?? keys.last
            await changed()
        } catch let f as CoreFailure {
            error = f["message"] ?? f.kind
        } catch {
            self.error = error.localizedDescription
        }
        busy = false
    }
}

/// A private key pasted or picked from Files.
private struct ImportKeySheet: View {
    let changed: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var text = ""
    @State private var passphrase = ""
    @State private var needPassphrase = false
    @State private var picking = false
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
                Section {
                    TextEditor(text: $text)
                        .font(.caption.monospaced())
                        .frame(minHeight: 140)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    HStack {
                        Button { if let s = UIPasteboard.general.string { text = s } } label: { Label("Paste", systemImage: "doc.on.clipboard") }
                            .buttonStyle(.borderless)
                        Spacer()
                        Button { picking = true } label: { Label("From Files", systemImage: "folder") }
                            .buttonStyle(.borderless)
                    }
                } header: {
                    Text("Private key")
                }
                if needPassphrase {
                    Section {
                        SecureField("Passphrase", text: $passphrase).onSubmit { Task { await save() } }
                    }
                }
                if let error { Text(error).foregroundStyle(.red).font(.footnote) }
            }
            .navigationTitle("Key import")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else { Button("Add") { Task { await save() } }.disabled(text.isEmpty) }
                }
            }
            .fileImporter(isPresented: $picking, allowedContentTypes: [.data, .text, .item]) { result in
                guard case .success(let url) = result else { return }
                let open = url.startAccessingSecurityScopedResource()
                defer { if open { url.stopAccessingSecurityScopedResource() } }
                if let s = try? String(contentsOf: url, encoding: .utf8) {
                    text = s
                    if name.isEmpty { name = url.deletingPathExtension().lastPathComponent }
                }
            }
        }
    }

    private func save() async {
        guard !text.isEmpty, !busy else { return }
        busy = true
        error = nil
        let n = name.trimmingCharacters(in: .whitespaces)
        do {
            let args: [String: Any?] = [
                "vaultId": "personal", "name": n.isEmpty ? "imported" : n, "text": text,
                "passphrase": needPassphrase && !passphrase.isEmpty ? passphrase : nil, "remember": true,
            ]
            _ = try await Core.shared.call("key_import", args.compactMapValues { $0 })
            await changed()
            dismiss()
        } catch let f as CoreFailure {
            switch f.kind {
            case "passphraseRequired": needPassphrase = true
            case "wrongPassphrase": needPassphrase = true; error = String(localized: "Wrong passphrase")
            case "invalidKey": error = String(localized: "This isn't a private key")
            default: error = f["message"] ?? f.kind
            }
        } catch {
            self.error = error.localizedDescription
        }
        busy = false
    }
}
