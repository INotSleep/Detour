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

/// The bar's sets and whether it shows, kept on this device.
@MainActor
final class KeyBarStore: ObservableObject {
    static let shared = KeyBarStore()

    @Published var sets: [KeySet] { didSet { save() } }
    @Published var hidden: Bool { didSet { save() } }

    private struct Saved: Codable {
        var sets: [KeySet]
        var hidden: Bool
    }

    private static let key = "keyBar"

    private init() {
        if let data = UserDefaults.standard.data(forKey: Self.key), let s = try? JSONDecoder().decode(Saved.self, from: data) {
            sets = s.sets
            hidden = s.hidden
        } else {
            sets = Self.defaults
            hidden = false
        }
    }

    private func save() {
        if let data = try? JSONEncoder().encode(Saved(sets: sets, hidden: hidden)) {
            UserDefaults.standard.set(data, forKey: Self.key)
        }
    }

    func reset() {
        sets = Self.defaults
        hidden = false
    }

    static let defaults: [KeySet] = [
        KeySet(name: "Main", keys: [.init(kind: .esc), .init(kind: .ctrl), .init(kind: .tab), .init(kind: .left), .init(kind: .down), .init(kind: .up), .init(kind: .right), .init(kind: .text, value: "|"), .init(kind: .text, value: "~"), .init(kind: .text, value: "/"), .init(kind: .text, value: "-")]),
        KeySet(name: "More", keys: [.init(kind: .alt), .init(kind: .home), .init(kind: .end), .init(kind: .pageUp), .init(kind: .pageDown), .init(kind: .combo, value: "ctrl+c"), .init(kind: .combo, value: "ctrl+d"), .init(kind: .combo, value: "ctrl+z"), .init(kind: .combo, value: "ctrl+r")]),
    ]
}

/// The bar over the keyboard: the sets, swiped between.
struct KeyBarView: View {
    @ObservedObject var store: KeyBarStore
    @ObservedObject var session: TerminalSession
    @State private var page = 0

    var body: some View {
        TabView(selection: $page) {
            ForEach(Array(store.sets.enumerated()), id: \.element.id) { i, set in
                ScrollView(.horizontal, showsIndicators: false) {
                    // Tight enough that the main set, arrows included, fits a 375-point phone
                    // beside the keyboard button.
                    HStack(spacing: 4) {
                        ForEach(set.keys) { key in
                            KeyButton(key: key, on: on(key)) { press(key) }
                        }
                    }
                    .padding(.horizontal, 6)
                }
                .tag(i)
            }
        }
        // The system's page dots take taps (one moves to the next set) and cover the keys'
        // lower half, so they are drawn here instead, for looking only.
        .tabViewStyle(.page(indexDisplayMode: .never))
        .overlay(alignment: .bottom) {
            if store.sets.count > 1 {
                HStack(spacing: 4) {
                    ForEach(store.sets.indices, id: \.self) { i in
                        Circle()
                            .fill(i == page ? Color.primary : Color.secondary.opacity(0.4))
                            .frame(width: 4, height: 4)
                    }
                }
                .padding(.bottom, 1)
                .allowsHitTesting(false)
            }
        }
        .frame(height: 46)
        .background(.bar)
    }

    private func on(_ key: BarKey) -> Bool {
        (key.kind == .ctrl && session.ctrl) || (key.kind == .alt && session.alt)
    }

    private func press(_ key: BarKey) {
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        switch key.kind {
        case .ctrl: session.ctrl.toggle()
        case .alt: session.alt.toggle()
        default: if let b = key.bytes() { session.send(b) }
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
        .background(key == nil ? Color.secondary.opacity(0.18) : on ? Color.accentColor : Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 7))
        .foregroundStyle(on ? Color.white : Color.primary)
        .contentShape(Rectangle())
    }
}
