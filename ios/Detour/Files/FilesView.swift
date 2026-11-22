import QuickLook
import SwiftUI
import UniformTypeIdentifiers

/// A host's files over SFTP, a session like a terminal: folders as a navigation stack, a file
/// opened in Quick Look (which shares it or saves it to Files), uploads from the phone, new
/// folders, rename and delete. The session opens and closes with its tab.
struct FilesView: View {
    @ObservedObject var files: FilesSession
    @State private var password = ""

    var body: some View {
        NavigationStack {
            Group {
                switch files.state {
                case .ready:
                    FolderView(files: files, path: files.home)
                default:
                    gate
                }
            }
            .navigationDestination(for: String.self) { path in FolderView(files: files, path: path) }
        }
    }

    @ViewBuilder private var gate: some View {
        VStack(spacing: 14) {
            switch files.state {
            case .connecting, .ready:
                ProgressView()
                Text("\(files.host.login)@\(files.host.address)").font(.subheadline.monospaced()).foregroundStyle(.secondary)
            case .needPassword(let prompt, let passphrase):
                let label = passphrase ? String(localized: "Passphrase") : String(localized: "Password")
                Text(prompt.isEmpty ? label : prompt).font(.subheadline)
                SecureField(label, text: $password)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(submit)
                Button("Connect", action: submit).buttonStyle(.borderedProminent).disabled(password.isEmpty)
            case .unknownKey(let fp):
                Text("New server key").font(.headline)
                Text(fp).font(.caption.monospaced()).multilineTextAlignment(.center)
                Button("Trust and connect") { Task { await files.open(trust: fp) } }.buttonStyle(.borderedProminent)
            case .failed(let message):
                Text(message).font(.subheadline).multilineTextAlignment(.center)
                Button("Retry") { Task { await files.open() } }.buttonStyle(.borderedProminent)
            }
        }
        .padding(24)
        .frame(maxWidth: 360)
        .navigationTitle(files.host.label)
        .navigationBarTitleDisplayMode(.inline)
    }

    private func submit() {
        guard !password.isEmpty else { return }
        let pw = password
        password = ""
        Task { await files.open(secret: pw) }
    }
}

/// One folder of the server.
private struct FolderView: View {
    @ObservedObject var files: FilesSession
    let path: String
    @State private var entries: [RemoteFile]?
    @State private var error: String?
    @State private var preview: URL?
    @State private var opening: String?
    @State private var uploading = false
    @State private var newFolder = false
    @State private var folderName = ""
    @State private var renaming: RemoteFile?
    @State private var newName = ""
    @State private var deleting: RemoteFile?
    @State private var exists: [URL] = []
    /// A link that turned out to be a folder.
    @State private var linked: String?
    @State private var flash: String?

    var body: some View {
        List {
            if let entries {
                ForEach(entries) { f in row(f) }
            }
        }
        .overlay {
            if entries == nil && error == nil {
                ProgressView()
            } else if let error {
                ContentUnavailableView {
                    Label(error, systemImage: "exclamationmark.triangle")
                } actions: {
                    Button("Retry") { Task { await load() } }
                }
            } else if entries?.isEmpty == true {
                ContentUnavailableView("Empty folder", systemImage: "folder")
            }
        }
        .navigationTitle(path == "/" ? "/" : (path as NSString).lastPathComponent)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Menu {
                    Button { uploading = true } label: { Label("Upload", systemImage: "square.and.arrow.up") }
                    Button { folderName = ""; newFolder = true } label: { Label("New folder", systemImage: "folder.badge.plus") }
                    Button { UIPasteboard.general.string = path } label: { Label("Copy path", systemImage: "doc.on.doc") }
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityLabel("Add")
            }
        }
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 6) {
                if let flash {
                    Text(flash)
                        .font(.subheadline)
                        .foregroundStyle(.red)
                        .multilineTextAlignment(.center)
                        .padding(12)
                        .frame(maxWidth: .infinity)
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
                        .padding(.horizontal)
                        .transition(.opacity)
                }
                TransfersBar(files: files)
            }
            .animation(.easeOut(duration: 0.2), value: flash)
        }
        .navigationDestination(item: $linked) { p in FolderView(files: files, path: p) }
        .refreshable { await load() }
        .task { if entries == nil { await load() } }
        .quickLookPreview($preview)
        .fileImporter(isPresented: $uploading, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result else { return }
            Task { await upload(urls, replace: false) }
        }
        .alert("New folder", isPresented: $newFolder) {
            TextField("Name", text: $folderName)
            Button("Cancel", role: .cancel) {}
            Button("Create") { Task { await act { try await files.mkdir(FilesSession.join(path, folderName)) } } }
        }
        .alert("Rename", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("Name", text: $newName)
            Button("Cancel", role: .cancel) {}
            Button("Rename") {
                if let f = renaming, !newName.isEmpty, newName != f.name {
                    Task { await act { try await files.rename(FilesSession.join(path, f.name), to: FilesSession.join(path, newName)) } }
                }
            }
        }
        .confirmationDialog(deleting.map { String(localized: "Delete \($0.name)?") } ?? "", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                if let f = deleting { Task { await act { try await files.remove(FilesSession.join(path, f.name)) } } }
            }
        }
        .alert(exists.count == 1 ? String(localized: "\(exists[0].lastPathComponent) already exists") : String(localized: "Some files already exist"), isPresented: Binding(get: { !exists.isEmpty }, set: { if !$0 { exists = [] } })) {
            Button("Cancel", role: .cancel) {}
            Button("Replace") {
                let urls = exists
                Task { await upload(urls, replace: true) }
            }
        }
    }

    @ViewBuilder private func row(_ f: RemoteFile) -> some View {
        let full = FilesSession.join(path, f.name)
        Group {
            if f.isDir {
                NavigationLink(value: full) { FileRow(file: f) }
            } else {
                Button { Task { await open(f) } } label: {
                    HStack {
                        FileRow(file: f)
                        if opening == f.name { ProgressView() }
                    }
                }
                .foregroundStyle(.primary)
            }
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
            Button(role: .destructive) { deleting = f } label: { Label("Delete", systemImage: "trash") }
                .tint(.red)
            Button { newName = f.name; renaming = f } label: { Label("Rename", systemImage: "pencil") }.tint(.accentColor)
        }
        .contextMenu {
            if !f.isDir { Button { Task { await open(f) } } label: { Label("Open", systemImage: "eye") } }
            Button { newName = f.name; renaming = f } label: { Label("Rename", systemImage: "pencil") }
            Button { UIPasteboard.general.string = full } label: { Label("Copy path", systemImage: "doc.on.doc") }
            Button(role: .destructive) { deleting = f } label: { Label("Delete", systemImage: "trash") }
        }
    }

    private func load() async {
        do {
            entries = try await files.list(path)
            error = nil
        } catch let f as CoreFailure {
            error = f["message"] ?? f.kind
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// A link may lead to a folder: it opens like one if it lists.
    private func open(_ f: RemoteFile) async {
        let full = FilesSession.join(path, f.name)
        opening = f.name
        defer { opening = nil }
        if f.isLink, (try? await files.list(full)) != nil {
            linked = full
            return
        }
        do {
            preview = try await files.download(full)
        } catch let e as CoreFailure {
            showError(e["message"] ?? e.kind)
        } catch {
            showError(error.localizedDescription)
        }
    }

    private func upload(_ urls: [URL], replace: Bool) async {
        var taken: [URL] = []
        for url in urls {
            do {
                try await files.upload(url, into: path, replace: replace)
            } catch let f as CoreFailure where (f["message"] ?? "").hasSuffix("already exists") {
                taken.append(url)
            } catch let f as CoreFailure {
                showError(f["message"] ?? f.kind)
            } catch {
                showError(error.localizedDescription)
            }
        }
        await load()
        exists = taken
    }

    /// Runs a change, then shows the folder as it is now.
    private func act(_ change: () async throws -> Void) async {
        do {
            try await change()
        } catch let f as CoreFailure {
            showError(f["message"] ?? f.kind)
        } catch {
            showError(error.localizedDescription)
        }
        await load()
    }

    private func showError(_ message: String) {
        flash = message
        Task {
            try? await Task.sleep(for: .seconds(4))
            if flash == message { flash = nil }
        }
    }
}

private struct FileRow: View {
    let file: RemoteFile

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: file.isDir ? "folder.fill" : file.isLink ? "arrow.turn.down.right" : "doc")
                .foregroundStyle(file.isDir ? Color.accentColor : .secondary)
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(file.name).lineLimit(1).truncationMode(.middle)
                if !detail.isEmpty {
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                }
            }
        }
    }

    private var detail: String {
        var parts: [String] = []
        if !file.isDir { parts.append(ByteCountFormatter.string(fromByteCount: Int64(file.size), countStyle: .file)) }
        if let m = file.modified {
            parts.append(Date(timeIntervalSince1970: m / 1000).formatted(date: .abbreviated, time: .shortened))
        }
        return parts.joined(separator: " · ")
    }
}

/// What is being downloaded, uploaded or deleted, with Cancel.
private struct TransfersBar: View {
    @ObservedObject var files: FilesSession

    var body: some View {
        if let t = files.transfers.first {
            HStack(spacing: 12) {
                Image(systemName: t.kind == .down ? "arrow.down.circle" : t.kind == .up ? "arrow.up.circle" : "trash.circle")
                    .foregroundStyle(Color.accentColor)
                VStack(alignment: .leading, spacing: 4) {
                    Text(files.transfers.count > 1 ? "\(t.name) +\(files.transfers.count - 1)" : t.name)
                        .font(.subheadline)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if let f = t.fraction { ProgressView(value: f) } else { ProgressView(value: 0).opacity(0.4) }
                }
                Button("Cancel") { files.cancel(t) }.font(.subheadline)
            }
            .padding(12)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
            .padding(.horizontal)
            .padding(.bottom, 4)
        }
    }
}
