import Foundation
import SwiftUI

/// A button of the key bar or of the keys keyboard.
struct BarKey: Codable, Identifiable, Hashable {
    enum Kind: String, Codable, CaseIterable {
        case esc, tab, ctrl, alt, up, down, left, right, home, end, pageUp, pageDown, enter, backspace
        /// Types `value` (`\n` in it is Enter).
        case text
        /// A combination: `ctrl+c`, `alt+b`, `shift+tab`, `ctrl+alt+left`.
        case combo
        /// A named key (`f5`, `insert`, `space`, see `KeyCodes.named`) or one character (`|`).
        case key
        /// A snippet: `label` is its name, `value` its command, typed and run with Enter.
        case snippet
    }

    var id = UUID()
    var kind: Kind
    var label: String = ""
    var value: String = ""

    /// Modifiers and the key of a combination.
    var comboParts: (ctrl: Bool, alt: Bool, shift: Bool, key: String)? {
        guard kind == .combo else { return nil }
        let parts = value.split(separator: "+", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespaces) }
        // "ctrl++" is Ctrl and the plus key.
        let key = value.hasSuffix("++") ? "+" : parts.last ?? ""
        let mods = Set(parts.dropLast().map { $0.lowercased() })
        guard !key.isEmpty else { return nil }
        return (mods.contains("ctrl"), mods.contains("alt") || mods.contains("meta"), mods.contains("shift"), key)
    }

    /// What the button shows, short enough for a small square key.
    var title: String {
        if !label.isEmpty { return label }
        switch kind {
        case .esc: return "Esc"
        case .tab: return "Tab"
        case .ctrl: return "Ctrl"
        case .alt: return "Alt"
        case .up: return "↑"
        case .down: return "↓"
        case .left: return "←"
        case .right: return "→"
        case .home: return "Home"
        case .end: return "End"
        case .pageUp: return "PgUp"
        case .pageDown: return "PgDn"
        case .enter: return "⏎"
        case .backspace: return "⌫"
        case .text, .snippet: return value
        case .key: return KeyCodes.title(value)
        case .combo:
            guard let c = comboParts else { return value }
            return (c.ctrl ? "^" : "") + (c.alt ? "⌥" : "") + (c.shift ? "⇧" : "") + KeyCodes.title(c.key)
        }
    }

    /// The bytes it sends; nil for Ctrl and Alt, which apply to the next key.
    func bytes(appCursor: Bool = false) -> [UInt8]? {
        let named: [Kind: String] = [.esc: "esc", .tab: "tab", .up: "up", .down: "down", .left: "left", .right: "right", .home: "home", .end: "end", .pageUp: "pageup", .pageDown: "pagedown", .enter: "enter", .backspace: "backspace"]
        if let n = named[kind] { return KeyCodes.bytes(n, appCursor: appCursor) }
        switch kind {
        case .ctrl, .alt: return nil
        case .text: return Array(value.replacingOccurrences(of: "\\n", with: "\r").replacingOccurrences(of: "\n", with: "\r").utf8)
        case .snippet:
            let text = value.trimmingCharacters(in: .newlines).replacingOccurrences(of: "\r\n", with: "\r").replacingOccurrences(of: "\n", with: "\r") + "\r"
            return Array(text.utf8)
        case .key: return KeyCodes.bytes(value, appCursor: appCursor)
        case .combo:
            guard let c = comboParts else { return nil }
            return KeyCodes.bytes(c.key, ctrl: c.ctrl, alt: c.alt, shift: c.shift, appCursor: appCursor)
        default: return nil
        }
    }

    /// Whether a combination's text is one Detour can send.
    static func combo(_ text: String) -> [UInt8]? {
        BarKey(kind: .combo, value: text).bytes()
    }
}

/// A row of keys the bar swipes between.
struct KeySet: Codable, Identifiable, Hashable {
    var id = UUID()
    var name: String
    var keys: [BarKey]
}

/// The bar's keys and whether it shows, kept on this device. The bar is one row that scrolls:
/// sets saved by earlier versions are joined into it.
@MainActor
final class KeyBarStore: ObservableObject {
    static let shared = KeyBarStore()

    @Published var keys: [BarKey] { didSet { save() } }
    @Published var hidden: Bool { didSet { save() } }

    private struct Saved: Codable {
        var keys: [BarKey]?
        /// Earlier versions: rows swiped between.
        var sets: [KeySet]?
        var hidden: Bool
    }

    private static let key = "keyBar"

    private init() {
        if let data = UserDefaults.standard.data(forKey: Self.key), let s = try? JSONDecoder().decode(Saved.self, from: data) {
            keys = s.keys ?? (s.sets ?? []).flatMap(\.keys)
            hidden = s.hidden
        } else {
            keys = Self.defaults
            hidden = false
        }
    }

    private func save() {
        if let data = try? JSONEncoder().encode(Saved(keys: keys, sets: nil, hidden: hidden)) {
            UserDefaults.standard.set(data, forKey: Self.key)
        }
    }

    func reset() {
        keys = Self.defaults
        hidden = false
    }

    static let defaults: [BarKey] = [
        .init(kind: .esc), .init(kind: .ctrl), .init(kind: .tab), .init(kind: .left), .init(kind: .down), .init(kind: .up), .init(kind: .right),
        .init(kind: .text, value: "|"), .init(kind: .text, value: "~"), .init(kind: .text, value: "/"), .init(kind: .text, value: "-"),
        .init(kind: .alt), .init(kind: .home), .init(kind: .end), .init(kind: .pageUp), .init(kind: .pageDown),
        .init(kind: .combo, value: "ctrl+c"), .init(kind: .combo, value: "ctrl+d"), .init(kind: .combo, value: "ctrl+z"), .init(kind: .combo, value: "ctrl+r"),
    ]
}

/// The bar over the keyboard: one row of keys that scrolls sideways.
struct KeyBarView: View {
    @ObservedObject var store: KeyBarStore
    @ObservedObject var session: TerminalSession

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                ForEach(store.keys) { key in
                    KeyButton(key: key, on: on(key)) { press(key) }
                }
            }
            .padding(.horizontal, 6)
        }
        .frame(height: 46)
        .background(.bar)
    }

    private func on(_ key: BarKey) -> Bool {
        (key.kind == .ctrl && session.ctrl) || (key.kind == .alt && session.alt)
    }

    private func press(_ key: BarKey) {
        Haptics.tap()
        switch key.kind {
        case .ctrl: session.ctrl.toggle()
        case .alt: session.alt.toggle()
        default: if let b = key.bytes(appCursor: session.appCursor) { session.send(b) }
        }
    }
}

private struct KeyButton: View {
    let key: BarKey
    let on: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) { KeyCap(key: key, on: on, width: KeyCap.barWidth, height: 34) }
            .buttonStyle(.plain)
    }
}

/// How a key looks, the same in the bar, the keys keyboard and their editor: every key the same
/// size, its text shrunk to fit.
struct KeyCap: View {
    static let barWidth: CGFloat = 40
    let key: BarKey?
    var on = false
    var width: CGFloat? = nil
    var height: CGFloat = 40

    var body: some View {
        Group {
            if let key {
                VStack(spacing: 1) {
                    Text(key.title)
                        .font(.system(size: 15, weight: .medium, design: key.kind == .snippet ? .default : .monospaced))
                        .lineLimit(key.kind == .snippet ? 2 : 1)
                        .minimumScaleFactor(0.45)
                        .multilineTextAlignment(.center)
                    if key.kind == .snippet {
                        Image(systemName: "play.fill").font(.system(size: 6)).foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 3)
            } else {
                Color.clear
            }
        }
        .frame(width: width, height: height)
        .frame(maxWidth: width == nil ? .infinity : nil)
        // A fill, not a background colour: it shows on the keyboard, the bar and a list's row alike.
        .background(key == nil ? Color.secondary.opacity(0.12) : on ? Color.accentColor : Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 7))
        .overlay {
            if key == nil { RoundedRectangle(cornerRadius: 7).strokeBorder(Color.secondary.opacity(0.25), style: StrokeStyle(lineWidth: 1, dash: [3, 3])) }
        }
        .foregroundStyle(on ? Color.white : Color.primary)
        .contentShape(Rectangle())
    }
}
