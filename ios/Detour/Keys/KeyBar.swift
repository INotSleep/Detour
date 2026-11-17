import Foundation
import SwiftUI

/// A button of the key bar.
struct BarKey: Codable, Identifiable, Hashable {
    enum Kind: String, Codable, CaseIterable {
        case esc, tab, ctrl, alt, up, down, left, right, home, end, pageUp, pageDown, enter, backspace
        /// Types `value` (a snippet; `\n` in it is Enter).
        case text
        /// A combination like `ctrl+c` or `alt+b`.
        case combo
    }

    var id = UUID()
    var kind: Kind
    var label: String = ""
    var value: String = ""

    /// What the button shows.
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
        case .text: return value
        case .combo: return value.split(separator: "+").map { $0.count == 1 ? $0.uppercased() : $0.capitalized }.joined(separator: "+")
        }
    }

    /// The bytes it sends; nil for Ctrl and Alt, which apply to the next key.
    var bytes: [UInt8]? {
        let esc: [UInt8] = [0x1b]
        switch kind {
        case .esc: return esc
        case .tab: return [0x09]
        case .ctrl, .alt: return nil
        case .up: return esc + Array("[A".utf8)
        case .down: return esc + Array("[B".utf8)
        case .right: return esc + Array("[C".utf8)
        case .left: return esc + Array("[D".utf8)
        case .home: return esc + Array("[H".utf8)
        case .end: return esc + Array("[F".utf8)
        case .pageUp: return esc + Array("[5~".utf8)
        case .pageDown: return esc + Array("[6~".utf8)
        case .enter: return [0x0d]
        case .backspace: return [0x7f]
        case .text: return Array(value.replacingOccurrences(of: "\\n", with: "\r").replacingOccurrences(of: "\n", with: "\r").utf8)
        case .combo: return Self.combo(value)
        }
    }

    /// `ctrl+c` → 3, `alt+b` → Esc b, `ctrl+alt+x` → Esc 24.
    static func combo(_ text: String) -> [UInt8]? {
        let parts = text.lowercased().split(separator: "+").map { $0.trimmingCharacters(in: .whitespaces) }
        guard let key = parts.last, !key.isEmpty else { return nil }
        let named: [String: [UInt8]] = ["esc": [0x1b], "tab": [0x09], "enter": [0x0d], "space": [0x20], "backspace": [0x7f]]
        var out = named[key] ?? Array(key.utf8)
        if parts.contains("ctrl") {
            guard out.count == 1, let c = TerminalSession.control(out[0]) else { return nil }
            out = [c]
        }
        if parts.contains("alt") || parts.contains("meta") { out = [0x1b] + out }
        return out
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
                    HStack(spacing: 6) {
                        ForEach(set.keys) { key in
                            KeyButton(key: key, on: on(key)) { press(key) }
                        }
                    }
                    .padding(.horizontal, 8)
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
        default: if let b = key.bytes { session.send(b) }
        }
    }
}

private struct KeyButton: View {
    let key: BarKey
    let on: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(key.title)
                .font(.system(size: 15, weight: .medium, design: .monospaced))
                .lineLimit(1)
                .padding(.horizontal, 12)
                .frame(minWidth: 40, minHeight: 34)
                .background(on ? Color.accentColor : Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 7))
                .foregroundStyle(on ? Color.white : Color.primary)
        }
        .buttonStyle(.plain)
    }
}
