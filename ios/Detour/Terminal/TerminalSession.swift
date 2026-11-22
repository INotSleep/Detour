import Foundation
import SwiftTerm
import UIKit

/// One terminal tab: an SSH session the core holds, its output drawn by SwiftTerm.
@MainActor
final class TerminalSession: ObservableObject, Identifiable {
    enum State: Equatable {
        case connecting
        case connected
        /// The session ended: by itself (`exit`) or because the connection dropped.
        case closed(dropped: Bool)
        case needPassword(prompt: String)
        case unknownKey(fingerprint: String)
        case failed(String)
    }

    let id = UUID()
    let host: Host
    @Published var state: State = .connecting
    @Published var title: String
    /// Ctrl and Alt from the key bar, for the next key typed.
    @Published var ctrl = false
    @Published var alt = false
    /// The keys keyboard is up in place of the system one.
    @Published private(set) var keysKeyboard = false
    /// The keys keyboard asked for its settings.
    @Published var customizing = false
    private var keysView: KeysInputView?

    private(set) var sessionId: String?
    private var stream: UInt64?
    private var pending: [UInt8] = []
    private var userClosed = false
    /// A check found the connection gone: connect again as soon as the core ends the session.
    private var reconnectWhenEnded = false
    weak var view: TerminalView? {
        didSet {
            view?.inputView = keysKeyboard ? keysInput() : nil
            flush()
        }
    }
    var cols = 80
    var rows = 24

    init(host: Host) {
        self.host = host
        title = host.label
    }

    func connect(secret: String? = nil, trust: String? = nil) {
        state = .connecting
        let s = Core.shared.newStream()
        stream = s
        Core.shared.watch(stream: s, data: { [weak self] d in self?.output(d) }, end: { [weak self] info in self?.ended(info) })
        let args: [String: Any] = ([
            "vaultId": host.vaultId, "hostId": host.id, "secret": secret, "trustFingerprint": trust, "cols": cols, "rows": rows,
        ] as [String: Any?]).compactMapValues { $0 }
        Task {
            do {
                let r = try await Core.shared.call("session_open", ["args": args, "stream": s]) as? [String: Any]
                sessionId = r?["sessionId"] as? String
                state = .connected
            } catch let f as CoreFailure {
                Core.shared.unwatch(stream: s)
                switch f.kind {
                case "needSecret": state = .needPassword(prompt: f["prompt"] ?? "")
                case "hostKeyUnknown":
                    #if DEBUG
                    // Screenshots: the simulator's own Mac.
                    if Shots.screen != nil, let fp = f["fingerprint"] {
                        connect(trust: fp)
                        return
                    }
                    #endif
                    state = .unknownKey(fingerprint: f["fingerprint"] ?? "")
                default: state = .failed(Self.describe(f))
                }
            } catch {
                state = .failed(error.localizedDescription)
            }
        }
    }

    /// Output with the highlights the desktop app adds (when they're on).
    private lazy var shown = HighlightedOutput(deliver: { [weak self] bytes in self?.show(bytes) }, enabled: { TerminalSettings.shared.highlight })

    private func output(_ data: Data) {
        shown.write([UInt8](data))
    }

    private func show(_ bytes: [UInt8]) {
        if let view { view.feed(byteArray: bytes[...]) } else { pending += bytes }
    }

    private func flush() {
        guard let view, !pending.isEmpty else { return }
        view.feed(byteArray: pending[...])
        pending = []
    }

    private func ended(_ info: [String: Any]) {
        sessionId = nil
        guard !userClosed else { return }
        // An exit status means the shell ended; none means the connection went away.
        let dropped = info["exitStatus"] == nil || info["exitStatus"] is NSNull
        if dropped && reconnectWhenEnded {
            reconnectWhenEnded = false
            reconnect()
            return
        }
        state = .closed(dropped: dropped)
    }

    /// After the phone slept or changed networks: a connection that no longer answers is
    /// replaced at once, instead of after the keepalives give up on it (minutes).
    func checkConnection() {
        switch state {
        case .closed(dropped: true):
            reconnect()
        case .connected:
            guard let id = sessionId else { return }
            Task {
                let r = try? await Core.shared.call("session_check", ["sessionId": id, "timeoutMs": 5000]) as? [String: Any]
                guard (r?["alive"] as? Bool) == false, !userClosed else { return }
                if case .closed(dropped: true) = state { reconnect() } else { reconnectWhenEnded = true }
            }
        default:
            break
        }
    }

    private func keysInput() -> KeysInputView {
        if let keysView { return keysView }
        let v = KeysInputView(session: self)
        keysView = v
        return v
    }

    /// Swaps the system keyboard and the keys keyboard, and shows the one chosen. The keys
    /// keyboard takes the place of the system one and the bar over it together, so the terminal
    /// keeps its size; the swap itself is not animated.
    func showKeysKeyboard(_ on: Bool) {
        let kb = KeyboardObserver.shared
        if on, kb.shown, !keysKeyboard {
            KeysInputView.matched = kb.height + (KeyBarStore.shared.hidden ? 0 : KeysInputView.barHeight)
        }
        keysKeyboard = on
        guard let view else { return }
        if on { keysInput().fit() }
        view.inputView = on ? keysInput() : nil
        UIView.performWithoutAnimation { view.reloadInputViews() }
        if !view.isFirstResponder { _ = view.becomeFirstResponder() }
    }

    func toggleKeyboard(shown: Bool) {
        guard let view else { return }
        if shown { _ = view.resignFirstResponder() } else { _ = view.becomeFirstResponder() }
    }

    func reconnect() {
        output(Data("\r\n".utf8))
        connect()
    }

    /// What the person typed (or a key of the bar), with Ctrl or Alt if armed.
    func send(_ bytes: [UInt8]) {
        var out = bytes
        if ctrl, out.count == 1, let c = Self.control(out[0]) {
            out = [c]
        }
        if alt { out = [0x1b] + out }
        ctrl = false
        alt = false
        input(out)
    }

    /// Types a snippet's command and presses Enter (each line of a longer one too).
    func run(_ command: String) {
        let text = command.trimmingCharacters(in: .newlines).replacingOccurrences(of: "\r\n", with: "\r").replacingOccurrences(of: "\n", with: "\r") + "\r"
        input(Array(text.utf8))
    }

    func input(_ bytes: [UInt8]) {
        guard let id = sessionId else {
            if case .closed = state, bytes == [13] { reconnect() }
            return
        }
        let text = String(decoding: bytes, as: UTF8.self)
        Task { _ = try? await Core.shared.call("session_input", ["sessionId": id, "data": text]) }
    }

    func resize(cols: Int, rows: Int) {
        self.cols = cols
        self.rows = rows
        guard let id = sessionId else { return }
        Task { _ = try? await Core.shared.call("session_resize", ["sessionId": id, "cols": cols, "rows": rows]) }
    }

    func close() {
        userClosed = true
        view?.inputView = nil
        keysView = nil
        if let s = stream { Core.shared.unwatch(stream: s) }
        guard let id = sessionId else { return }
        Task { _ = try? await Core.shared.call("session_close", ["sessionId": id]) }
    }

    /// Ctrl with a key: its control code (Ctrl+C is 3).
    nonisolated static func control(_ b: UInt8) -> UInt8? {
        switch b {
        case 0x61...0x7a: return b - 0x60
        case 0x41...0x5a: return b - 0x40
        case 0x40, 0x5b...0x5f: return b & 0x1f
        case 0x20: return 0
        case 0x3f: return 0x7f
        default: return nil
        }
    }

    /// iOS turned down a connection into the local network (not allowed yet, or turned off).
    nonisolated static var localNetworkDenied: String { String(localized: "Detour has no access to the local network") }

    nonisolated static func describe(_ f: CoreFailure) -> String {
        let raw = "\(f["message"] ?? "") \(f["endpoint"] ?? "")"
        if raw.localizedCaseInsensitiveContains("no route to host") || raw.contains("os error 65") {
            return localNetworkDenied
        }
        switch f.kind {
        case "unreachable": return String(localized: "Can't reach \(f["endpoint"] ?? "the host")")
        case "authFailed": return String(localized: "The server didn't accept the login")
        case "hostKeyChanged": return String(localized: "The server's key changed. Check it on a computer before connecting.")
        case "agentUnavailable": return String(localized: "This host logs in with ssh-agent, which the phone doesn't have")
        case "proxy": return String(localized: "The proxy failed: \(f["reason"] ?? "")")
        case "locked": return String(localized: "Detour is locked")
        default: return f["message"] ?? f.kind
        }
    }
}
