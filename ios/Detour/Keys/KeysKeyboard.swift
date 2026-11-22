import SwiftUI
import UIKit

/// A row of the keys keyboard: eight places, each a key or empty.
struct KeyBlock: Codable, Identifiable, Hashable {
    static let size = 8
    var id = UUID()
    var keys: [BarKey?] = Array(repeating: nil, count: KeyBlock.size)
}

/// The keys keyboard's rows, kept on this device.
@MainActor
final class KeysKeyboardStore: ObservableObject {
    static let shared = KeysKeyboardStore()
    private static let storeKey = "keysKeyboard"

    @Published var blocks: [KeyBlock] { didSet { save() } }

    private init() {
        if let data = UserDefaults.standard.data(forKey: Self.storeKey), let b = try? JSONDecoder().decode([KeyBlock].self, from: data) {
            blocks = b
        } else {
            blocks = Self.defaults
        }
    }

    private func save() {
        if let data = try? JSONEncoder().encode(blocks) { UserDefaults.standard.set(data, forKey: Self.storeKey) }
    }

    func reset() { blocks = Self.defaults }

    static var defaults: [KeyBlock] {
        func key(_ v: String) -> BarKey { BarKey(kind: .key, value: v) }
        func combo(_ v: String) -> BarKey { BarKey(kind: .combo, value: v) }
        func row(_ keys: [BarKey]) -> KeyBlock { KeyBlock(keys: keys.map { Optional($0) }) }
        func chars(_ s: String) -> KeyBlock { row(s.map { key(String($0)) }) }
        return [
            row([key("esc"), key("tab"), BarKey(kind: .ctrl), BarKey(kind: .alt), key("home"), key("end"), key("pageup"), key("pagedown")]),
            row([key("left"), key("down"), key("up"), key("right"), key("space"), key("backspace"), key("delete"), key("enter")]),
            row((1...8).map { key("f\($0)") }),
            row((9...12).map { key("f\($0)") } + [combo("ctrl+c"), combo("ctrl+d"), combo("ctrl+z"), combo("ctrl+r")]),
            chars("|~/\\-_=+"),
            chars("<>{}[]()"),
            chars("!@#$%^&*"),
            chars("'\"`,.?;:"),
        ]
    }
}

/// The keys keyboard, shown in place of the system one: a button to change it, then its rows,
/// eight keys each (two rows side by side where there is room).
struct KeysKeyboardView: View {
    @ObservedObject var session: TerminalSession
    @ObservedObject private var store = KeysKeyboardStore.shared

    var body: some View {
        GeometryReader { geo in
            let wide = geo.size.width >= 700
            VStack(spacing: 6) {
                HStack(spacing: 6) {
                    Button { session.customizing = true } label: {
                        Label("Customize", systemImage: "slider.horizontal.3")
                            .font(.subheadline.weight(.medium))
                            .frame(maxWidth: .infinity, minHeight: 30)
                    }
                    .buttonStyle(.bordered)
                    Button { session.toggleKeyboard(shown: true) } label: {
                        Image(systemName: "keyboard.chevron.compact.down")
                            .font(.system(size: 16))
                            .frame(width: 44, height: 30)
                    }
                    .buttonStyle(.bordered)
                    .accessibilityLabel("Hide keyboard")
                }
                ScrollView {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 14), count: wide ? 2 : 1), spacing: 6) {
                        ForEach(store.blocks) { block in
                            HStack(spacing: 5) {
                                ForEach(0..<KeyBlock.size, id: \.self) { i in
                                    KeysKeyboardKey(key: block.keys.indices.contains(i) ? block.keys[i] : nil, session: session)
                                }
                            }
                        }
                    }
                    .padding(.bottom, 8)
                }
            }
            .padding(.horizontal, 6)
            .padding(.top, 6)
        }
    }
}

/// A key of the keys keyboard. The space key also moves the cursor: drag along it for arrows.
private struct KeysKeyboardKey: View {
    let key: BarKey?
    @ObservedObject var session: TerminalSession
    @State private var dragged: CGSize = .zero
    @State private var moved = false

    var body: some View {
        if let key, key.kind == .key, key.value == "space" {
            // Ahead of the keyboard's own scrolling, or a drag up or down would scroll it.
            KeyCap(key: key, height: 40)
                .highPriorityGesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { g in drag(g.translation) }
                        .onEnded { _ in
                            if !moved { press(key) }
                            dragged = .zero
                            moved = false
                        }
                )
        } else if let key {
            Button { press(key) } label: { KeyCap(key: key, on: on(key), height: 40) }
                .buttonStyle(.plain)
        } else {
            Color.clear.frame(maxWidth: .infinity, maxHeight: 40)
        }
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

    /// Every 12 points along sends an arrow, as the system keyboard's space bar moves its cursor.
    private func drag(_ t: CGSize) {
        let step: CGFloat = 12
        var dx = t.width - dragged.width
        var dy = t.height - dragged.height
        while abs(dx) >= step {
            session.send(KeyCodes.bytes(dx > 0 ? "right" : "left") ?? [])
            dragged.width += dx > 0 ? step : -step
            dx = t.width - dragged.width
            moved = true
        }
        while abs(dy) >= step * 1.5 {
            session.send(KeyCodes.bytes(dy > 0 ? "down" : "up") ?? [])
            dragged.height += dy > 0 ? step * 1.5 : -step * 1.5
            dy = t.height - dragged.height
            moved = true
        }
        if moved { UISelectionFeedbackGenerator().selectionChanged() }
    }
}

/// The keys keyboard as the terminal's input view: iOS puts it where the keyboard goes.
final class KeysInputView: UIInputView {
    private var host: UIHostingController<KeysKeyboardView>?
    private var heightConstraint: NSLayoutConstraint?

    /// The bar over the system keyboard (`TerminalBar.height`).
    static let barHeight: CGFloat = 46
    /// The system keyboard and the bar together, as last seen: the keys keyboard takes their place.
    static var matched: CGFloat?
    static var height: CGFloat { matched ?? (UIDevice.current.userInterfaceIdiom == .pad ? 376 : 382) }

    /// Takes the height of the keyboard it replaces.
    func fit() {
        heightConstraint?.constant = Self.height
        frame.size.height = Self.height
    }

    init(session: TerminalSession) {
        super.init(frame: CGRect(x: 0, y: 0, width: UIScreen.main.bounds.width, height: Self.height), inputViewStyle: .keyboard)
        let h = UIHostingController(rootView: KeysKeyboardView(session: session))
        h.view.backgroundColor = .clear
        h.view.translatesAutoresizingMaskIntoConstraints = false
        addSubview(h.view)
        NSLayoutConstraint.activate([
            h.view.leadingAnchor.constraint(equalTo: leadingAnchor),
            h.view.trailingAnchor.constraint(equalTo: trailingAnchor),
            h.view.topAnchor.constraint(equalTo: topAnchor),
            h.view.bottomAnchor.constraint(equalTo: safeAreaLayoutGuide.bottomAnchor),
        ])
        let c = heightAnchor.constraint(equalToConstant: Self.height)
        c.isActive = true
        heightConstraint = c
        host = h
    }

    required init?(coder: NSCoder) { nil }
}
