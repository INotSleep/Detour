import Foundation

/// A file on the server, as the core lists it.
struct RemoteFile: Decodable, Identifiable, Hashable {
    let name: String
    let kind: String
    let size: Double
    let modified: Double?
    let permissions: Int?

    var id: String { name }
    var isDir: Bool { kind == "dir" }
    var isLink: Bool { kind == "link" }
}

/// An SFTP connection to a host, held by the core while the files screen is open.
@MainActor
final class FilesSession: ObservableObject {
    enum State: Equatable {
        case connecting
        case ready
        case needPassword(prompt: String, passphrase: Bool)
        case unknownKey(fingerprint: String)
        case failed(String)
    }

    /// Something going on with files: a download, an upload or a delete.
    struct Transfer: Identifiable, Equatable {
        let id: String
        let name: String
        let kind: Kind
        var done: Double = 0
        var total: Double = 0

        enum Kind { case down, up, delete }
        var fraction: Double? { total > 0 ? min(1, done / total) : nil }
    }

    let host: Host
    @Published var state = State.connecting
    @Published var transfers: [Transfer] = []
    private(set) var home = "/"
    private var id: String?

    init(host: Host) {
        self.host = host
    }

    func open(secret: String? = nil, trust: String? = nil) async {
        state = .connecting
        let args: [String: Any?] = ["vaultId": host.vaultId, "hostId": host.id, "secret": secret, "trustFingerprint": trust]
        do {
            let r = try await Core.shared.call("files_open", args) as? [String: Any] ?? [:]
            id = r["id"] as? String
            home = r["home"] as? String ?? "/"
            state = .ready
        } catch let f as CoreFailure {
            switch f.kind {
            case "needSecret": state = .needPassword(prompt: f["prompt"] ?? "", passphrase: f["secret"] == "passphrase")
            case "hostKeyUnknown":
                #if DEBUG
                if Shots.screen != nil, trust == nil, let fp = f["fingerprint"] { return await open(trust: fp) }
                #endif
                state = .unknownKey(fingerprint: f["fingerprint"] ?? "")
            default: state = .failed(TerminalSession.describe(f))
            }
        } catch {
            state = .failed(error.localizedDescription)
        }
    }

    func close() {
        guard let id else { return }
        self.id = nil
        Task { _ = try? await Core.shared.call("files_close", ["id": id]) }
    }

    static func join(_ dir: String, _ name: String) -> String {
        dir.hasSuffix("/") ? dir + name : dir + "/" + name
    }

    func list(_ path: String) async throws -> [RemoteFile] {
        let files = try await Core.shared.call("files_list", ["id": id ?? "", "path": path], as: [RemoteFile].self)
        return files.sorted { a, b in a.isDir != b.isDir ? a.isDir : a.name.localizedStandardCompare(b.name) == .orderedAscending }
    }

    func mkdir(_ path: String) async throws {
        _ = try await Core.shared.call("files_mkdir", ["id": id ?? "", "path": path])
    }

    func rename(_ from: String, to: String) async throws {
        _ = try await Core.shared.call("files_rename", ["id": id ?? "", "from": from, "to": to])
    }

    func remove(_ path: String) async throws {
        try await transfer(name: (path as NSString).lastPathComponent, kind: .delete) { t, s in
            _ = try await Core.shared.call("files_remove", ["id": self.id ?? "", "path": path, "transfer": t, "stream": s])
        }
    }

    /// Downloads a file into a folder of its own in the app's temporary files; returns where.
    func download(_ remote: String) async throws -> URL {
        let name = (remote as NSString).lastPathComponent
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("files/\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let local = dir.appendingPathComponent(name)
        try await transfer(name: name, kind: .down) { t, s in
            _ = try await Core.shared.call("files_download", ["id": self.id ?? "", "remote": remote, "local": local.path, "transfer": t, "stream": s])
        }
        return local
    }

    /// Uploads a file picked on the phone into `dir`. Fails with `exists` unless `replace`.
    func upload(_ file: URL, into dir: String, replace: Bool) async throws {
        let open = file.startAccessingSecurityScopedResource()
        defer { if open { file.stopAccessingSecurityScopedResource() } }
        // The core reads it from the app's own files.
        let copy = FileManager.default.temporaryDirectory.appendingPathComponent("uploads/\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: copy, withIntermediateDirectories: true)
        let local = copy.appendingPathComponent(file.lastPathComponent)
        try FileManager.default.copyItem(at: file, to: local)
        defer { try? FileManager.default.removeItem(at: copy) }
        try await transfer(name: file.lastPathComponent, kind: .up) { t, s in
            _ = try await Core.shared.call("files_upload", ["id": self.id ?? "", "local": local.path, "remote": Self.join(dir, file.lastPathComponent), "replace": replace, "transfer": t, "stream": s])
        }
    }

    func cancel(_ t: Transfer) {
        Task { _ = try? await Core.shared.call("files_cancel", ["transfer": t.id]) }
    }

    private func transfer(name: String, kind: Transfer.Kind, _ run: (String, UInt64) async throws -> Void) async throws {
        let tid = UUID().uuidString
        let stream = Core.shared.newStream()
        transfers.append(Transfer(id: tid, name: name, kind: kind))
        Core.shared.watchProgress(stream: stream) { [weak self] done, total in
            guard let self, let i = self.transfers.firstIndex(where: { $0.id == tid }) else { return }
            self.transfers[i].done = done
            self.transfers[i].total = total
        }
        defer {
            Core.shared.unwatch(stream: stream)
            transfers.removeAll { $0.id == tid }
        }
        try await run(tid, stream)
    }
}
