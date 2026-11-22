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

/// The keys keyboard, shown in place of the system one: tabs at the bottom for its keys (rows of
/// eight; two rows side by side where there is room), the snippets, and the shell's history.
struct KeysKeyboardView: View {
    @ObservedObject var session: TerminalSession
    @ObservedObject private var store = KeysKeyboardStore.shared
    @State private var panel: KeysPanel = .keys

    var body: some View {
        VStack(spacing: 6) {
            Group {
                switch panel {
                case .keys: keys
                case .snippets: SnippetsPanel(session: session)
                case .history: HistoryPanel(session: session)
                }
            }
            .frame(maxHeight: .infinity)
            Picker("Keyboard", selection: $panel) {
                ForEach(KeysPanel.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented)
        }
        .padding(.horizontal, 6)
        .padding(.top, 6)
        .padding(.bottom, 4)
    }

    private var keys: some View {
        GeometryReader { geo in
            ScrollView {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 14), count: geo.size.width >= 700 ? 2 : 1), spacing: 6) {
                    ForEach(store.blocks) { block in
                        HStack(spacing: 5) {
                            ForEach(0..<KeyBlock.size, id: \.self) { i in
                                KeysKeyboardKey(key: block.keys.indices.contains(i) ? block.keys[i] : nil, session: session)
                            }
                        }
                    }
                }
                .padding(.bottom, 4)
            }
        }
    }
}

/// A key of the keys keyboard.
private struct KeysKeyboardKey: View {
    let key: BarKey?
    @ObservedObject var session: TerminalSession

    var body: some View {
        if let key {
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
        default: if let b = key.bytes(appCursor: session.appCursor) { session.send(b) }
        }
    }
}

/// The keys keyboard as the terminal's input view: iOS puts it where the keyboard goes.
final class KeysInputView: UIInputView {
    private var host: UIHostingController<KeysKeyboardView>?
    private var heightConstraint: NSLayoutConstraint?

    /// The system keyboard's height, as last seen: the keys keyboard takes exactly its place.
    static var matched: CGFloat?
    static var height: CGFloat { matched ?? (UIDevice.current.userInterfaceIdiom == .pad ? 330 : 336) }

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
