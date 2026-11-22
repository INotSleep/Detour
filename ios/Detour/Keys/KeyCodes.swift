import Foundation

/// What keys send, as xterm does: named keys, function keys and their modified forms.
enum KeyCodes {
    /// Named keys a combination or a button can be, in the order the pickers show them.
    static let named = ["esc", "tab", "enter", "backspace", "delete", "insert", "space", "up", "down", "left", "right", "home", "end", "pageup", "pagedown"]
        + (1...24).map { "f\($0)" }

    /// The bytes of `key` (a named key or one character) with modifiers.
    /// `appCursor`: the program asked for application cursor keys (arrows as `Esc O A`).
    static func bytes(_ key: String, ctrl: Bool = false, alt: Bool = false, shift: Bool = false, appCursor: Bool = false) -> [UInt8]? {
        let esc: [UInt8] = [0x1b]
        let k = key.count == 1 ? key : key.lowercased()
        // xterm's modifier parameter: 1 + Shift 1 + Alt 2 + Ctrl 4.
        let m = 1 + (shift ? 1 : 0) + (alt ? 2 : 0) + (ctrl ? 4 : 0)
        func csi(_ final: String, _ n: Int = 1) -> [UInt8] {
            m == 1 ? esc + Array("[\(n == 1 ? "" : "\(n)")\(final)".utf8) : esc + Array("[\(n);\(m)\(final)".utf8)
        }
        func tilde(_ n: Int) -> [UInt8] { esc + Array((m == 1 ? "[\(n)~" : "[\(n);\(m)~").utf8) }
        let arrows = ["up": "A", "down": "B", "right": "C", "left": "D", "home": "H", "end": "F"]
        if let f = arrows[k] {
            if m == 1 { return esc + Array(((appCursor ? "O" : "[") + f).utf8) }
            return csi(f)
        }
        switch k {
        case "pageup": return tilde(5)
        case "pagedown": return tilde(6)
        case "insert": return tilde(2)
        case "delete": return tilde(3)
        case "tab": return shift && !ctrl && !alt ? esc + Array("[Z".utf8) : (alt ? esc : []) + [0x09]
        case "esc": return (alt ? esc : []) + esc
        case "enter": return (alt ? esc : []) + [0x0d]
        case "backspace": return (alt ? esc : []) + (ctrl ? [0x08] : [0x7f])
        case "space": return (alt ? esc : []) + (ctrl ? [0x00] : [0x20])
        default: break
        }
        if k.hasPrefix("f"), let n = Int(k.dropFirst()), (1...24).contains(n) {
            if n <= 4 {
                let final = ["P", "Q", "R", "S"][n - 1]
                return m == 1 ? esc + Array("O\(final)".utf8) : esc + Array("[1;\(m)\(final)".utf8)
            }
            // F5 is 15, then gaps where the old terminals' keys were.
            let codes = [15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34, 42, 43, 44, 45]
            return tilde(codes[n - 5])
        }
        guard k.count == 1 else { return nil }
        var out = Array((shift ? k.uppercased() : k).utf8)
        if ctrl {
            guard out.count == 1, let c = TerminalSession.control(out[0]) else { return nil }
            out = [c]
        }
        if alt { out = esc + out }
        return out
    }

    /// How a named key or a character is shown on a small key.
    static func title(_ key: String) -> String {
        switch key.count == 1 ? key : key.lowercased() {
        case "esc": "Esc"
        case "tab": "Tab"
        case "enter": "⏎"
        case "backspace": "⌫"
        case "delete": "Del"
        case "insert": "Ins"
        case "space": "␣"
        case "up": "↑"
        case "down": "↓"
        case "left": "←"
        case "right": "→"
        case "home": "Home"
        case "end": "End"
        case "pageup": "PgUp"
        case "pagedown": "PgDn"
        case let k where k.hasPrefix("f") && Int(k.dropFirst()) != nil: k.uppercased()
        default: key.uppercased()
        }
    }
}
