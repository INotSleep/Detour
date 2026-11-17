import Foundation

/// A call the core refused, by its kind (`locked`, `needSecret`, `hostKeyUnknown`, ...).
struct CoreFailure: Error {
    let kind: String
    let fields: [String: Any]

    init(json: String) {
        let obj = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
        kind = obj["kind"] as? String ?? "internal"
        fields = obj
    }

    subscript(_ key: String) -> String? { fields[key] as? String }
}

/// Detour's core, inside the app. Calls go to it as JSON; what it sends on its own
/// (events, terminal output, a session's end) reaches whoever listens here.
final class Core: @unchecked Sendable {
    static let shared = Core()

    private let core: DetourCore
    private let listener = Listener()
    private var nextStream: UInt64 = 1
    private let lock = NSLock()

    private init() {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Detour", isDirectory: true)
        // The vault and its logs aren't backed up to iCloud: they are synced on their own.
        var url = dir
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? url.setResourceValues(values)
        core = try! DetourCore(dataDir: dir.path, listener: listener)
    }

    /// Calls the core; the result is its JSON.
    func call(_ method: String, _ params: [String: Any?] = [:]) async throws -> Any {
        let clean = params.compactMapValues { $0 }
        let body = String(data: try JSONSerialization.data(withJSONObject: clean), encoding: .utf8) ?? "{}"
        do {
            let out = try await core.call(method: method, params: body)
            return (try? JSONSerialization.jsonObject(with: Data(out.utf8), options: [.fragmentsAllowed])) ?? NSNull()
        } catch let CoreError.Failed(json) {
            throw CoreFailure(json: json)
        }
    }

    /// Calls the core and decodes its answer.
    func call<T: Decodable>(_ method: String, _ params: [String: Any?] = [:], as type: T.Type) async throws -> T {
        let value = try await call(method, params)
        let data = try JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed])
        return try JSONDecoder().decode(T.self, from: data)
    }

    /// A stream id for a session's output.
    func newStream() -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        nextStream += 1
        return nextStream
    }

    func watch(stream: UInt64, data: @escaping (Data) -> Void, end: @escaping ([String: Any]) -> Void) {
        listener.watch(stream: stream, data: data, end: end)
    }

    func unwatch(stream: UInt64) {
        listener.unwatch(stream: stream)
    }

    /// Progress of a transfer (`{ done, total }`), as often as the core reports it, until
    /// `unwatch`.
    func watchProgress(stream: UInt64, _ progress: @escaping (_ done: Double, _ total: Double) -> Void) {
        listener.watchProgress(stream: stream, progress)
    }

    /// Events of the core (`sessions.changed`, `locked`, `data.changed`...).
    func onEvent(_ handler: @escaping (String, [String: Any]) -> Void) {
        listener.onEvent(handler)
    }

    func stop() {
        core.stop()
    }
}

/// Receives from the core on its threads, hands on to the main one.
private final class Listener: CoreListener, @unchecked Sendable {
    private let lock = NSLock()
    private var streams: [UInt64: (data: (Data) -> Void, end: ([String: Any]) -> Void)] = [:]
    private var handlers: [(String, [String: Any]) -> Void] = []
    private var progress: [UInt64: (Double, Double) -> Void] = [:]

    func watchProgress(stream: UInt64, _ p: @escaping (Double, Double) -> Void) {
        lock.lock()
        progress[stream] = p
        lock.unlock()
    }

    func watch(stream: UInt64, data: @escaping (Data) -> Void, end: @escaping ([String: Any]) -> Void) {
        lock.lock()
        streams[stream] = (data, end)
        lock.unlock()
    }

    func unwatch(stream: UInt64) {
        lock.lock()
        streams[stream] = nil
        progress[stream] = nil
        lock.unlock()
    }

    func onEvent(_ handler: @escaping (String, [String: Any]) -> Void) {
        lock.lock()
        handlers.append(handler)
        lock.unlock()
    }

    func onEvent(name: String, data: String) {
        let obj = (try? JSONSerialization.jsonObject(with: Data(data.utf8), options: [.fragmentsAllowed])) as? [String: Any] ?? [:]
        lock.lock()
        let hs = handlers
        lock.unlock()
        DispatchQueue.main.async { hs.forEach { $0(name, obj) } }
    }

    func onData(stream: UInt64, bytes: Data) {
        lock.lock()
        let w = streams[stream]
        lock.unlock()
        guard let w else { return }
        DispatchQueue.main.async { w.data(bytes) }
    }

    func onStreamEnd(stream: UInt64, data: String) {
        let obj = (try? JSONSerialization.jsonObject(with: Data(data.utf8))) as? [String: Any] ?? [:]
        lock.lock()
        // A transfer's progress comes the same way as a session's end, and keeps coming.
        if let p = progress[stream] {
            lock.unlock()
            let done = (obj["done"] as? NSNumber)?.doubleValue ?? 0
            let total = (obj["total"] as? NSNumber)?.doubleValue ?? 0
            DispatchQueue.main.async { p(done, total) }
            return
        }
        let w = streams.removeValue(forKey: stream)
        lock.unlock()
        guard let w else { return }
        DispatchQueue.main.async { w.end(obj) }
    }
}
