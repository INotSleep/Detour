import SwiftTerm
import SwiftUI
import UIKit

/// SwiftTerm has a `Color` of its own.
private typealias Color = SwiftUI.Color

/// SwiftTerm's view for a session. Its own bar over the keyboard is off: ours sits in the
/// layout under the terminal, where it can be used with the keyboard hidden too.
struct TerminalHost: UIViewRepresentable {
    @ObservedObject var session: TerminalSession

    func makeCoordinator() -> Coordinator { Coordinator(session: session) }

    func makeUIView(context: Context) -> TerminalView {
        let tv = TerminalView(frame: .zero)
        tv.terminalDelegate = context.coordinator
        tv.backgroundColor = .black
        tv.nativeBackgroundColor = .black
        tv.nativeForegroundColor = UIColor(white: 0.9, alpha: 1)
        tv.font = UIFont.monospacedSystemFont(ofSize: 13, weight: .regular)
        tv.inputAccessoryView = nil
        session.view = tv
        DispatchQueue.main.async { _ = tv.becomeFirstResponder() }
        return tv
    }

    func updateUIView(_ tv: TerminalView, context: Context) {
        if context.coordinator.session !== session {
            context.coordinator.session = session
            session.view = tv
        }
    }

    @MainActor
    final class Coordinator: NSObject, TerminalViewDelegate {
        var session: TerminalSession

        init(session: TerminalSession) {
            self.session = session
            super.init()
        }

        nonisolated func send(source: TerminalView, data: ArraySlice<UInt8>) {
            let bytes = Array(data)
            MainActor.assumeIsolated { session.send(bytes) }
        }

        nonisolated func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
            MainActor.assumeIsolated { session.resize(cols: newCols, rows: newRows) }
        }

        nonisolated func setTerminalTitle(source: TerminalView, title: String) {
            MainActor.assumeIsolated { if !title.isEmpty { session.title = title } }
        }

        nonisolated func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
        nonisolated func scrolled(source: TerminalView, position: Double) {}
        nonisolated func bell(source: TerminalView) {}
        nonisolated func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {}
        nonisolated func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}

        nonisolated func clipboardCopy(source: TerminalView, content: Data) {
            if let s = String(data: content, encoding: .utf8) { UIPasteboard.general.string = s }
        }

        nonisolated func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
            if let url = URL(string: link) { DispatchQueue.main.async { UIApplication.shared.open(url) } }
        }
    }
}

/// The open terminals: a strip of tabs, the current one's screen, and what it waits for.
struct TerminalsView: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                tabs
                if let t = app.terminals.first(where: { $0.id == app.current }) {
                    SessionScreen(session: t)
                        .id(t.id)
                }
            }
            .background(Color.black)
            .ignoresSafeArea(.keyboard)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button { dismiss() } label: { Image(systemName: "chevron.down") }
                        .accessibilityLabel("Hosts")
                }
            }
            .toolbarBackground(.visible, for: .navigationBar)
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private var tabs: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(app.terminals) { t in
                    TabChip(session: t, selected: t.id == app.current, select: { app.current = t.id }, close: { app.close(t) })
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
        }
        .background(Color(.systemBackground))
    }
}

private struct TabChip: View {
    @ObservedObject var session: TerminalSession
    let selected: Bool
    let select: () -> Void
    let close: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(dot).frame(width: 7, height: 7)
            Text(session.title).font(.subheadline).lineLimit(1)
            Button(action: close) { Image(systemName: "xmark").font(.caption2.weight(.bold)) }
                .buttonStyle(.plain)
                .accessibilityLabel("Close")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(selected ? Color.accentColor.opacity(0.2) : Color(.secondarySystemBackground), in: Capsule())
        .onTapGesture(perform: select)
    }

    private var dot: Color {
        switch session.state {
        case .connected: return .green
        case .connecting: return .yellow
        default: return .red
        }
    }
}

/// A session's screen, with what it waits for over it and the key bar under it.
private struct SessionScreen: View {
    @ObservedObject var session: TerminalSession
    @ObservedObject private var keyboard = KeyboardObserver.shared
    @ObservedObject private var keys = KeyBarStore.shared
    @State private var password = ""

    var body: some View {
        GeometryReader { geo in
            // How far the keyboard reaches into this screen.
            let lift = keyboard.shown ? max(0, geo.frame(in: .global).maxY - keyboard.top) : 0
            let bar = keyboard.hardware || keys.hidden ? 0 : TerminalBar.height
            ZStack(alignment: .bottom) {
                // The terminal takes its new size at once rather than following the keyboard's
                // animation: resizing it on every frame is what made it jump.
                TerminalHost(session: session)
                    .padding(.bottom, lift + bar)
                    .transaction { $0.animation = nil }
                overlay
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .padding(.bottom, lift + bar)
                if !keyboard.hardware {
                    TerminalBar(session: session, showKeys: !keys.hidden, keyboardShown: keyboard.shown)
                        .padding(.bottom, lift)
                        .animation(.easeOut(duration: keyboard.duration), value: lift)
                }
            }
        }
        .ignoresSafeArea(.keyboard)
    }

    @ViewBuilder private var overlay: some View {
        switch session.state {
        case .connected:
            EmptyView()
        case .connecting:
            card { ProgressView(); Text("Connecting to \(session.host.login)@\(session.host.address)").font(.subheadline) }
        case .closed(let dropped):
            card {
                Text(dropped ? "Connection lost" : "Session ended").font(.headline)
                Button("Reconnect") { session.reconnect() }.buttonStyle(.borderedProminent)
            }
        case .needPassword(let prompt):
            card {
                Text(prompt.isEmpty ? String(localized: "Password") : prompt).font(.subheadline)
                SecureField("Password", text: $password)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(submit)
                Button("Connect", action: submit).buttonStyle(.borderedProminent).disabled(password.isEmpty)
            }
        case .unknownKey(let fp):
            card {
                Text("New server key").font(.headline)
                Text(fp).font(.caption.monospaced()).multilineTextAlignment(.center)
                Button("Trust and connect") { session.connect(trust: fp) }.buttonStyle(.borderedProminent)
            }
        case .failed(let message):
            card {
                Text(message).font(.subheadline).multilineTextAlignment(.center)
                Button("Retry") { session.connect() }.buttonStyle(.borderedProminent)
            }
        }
    }

    private func submit() {
        guard !password.isEmpty else { return }
        session.connect(secret: password)
        password = ""
    }

    private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(spacing: 12) { content() }
            .padding(20)
            .frame(maxWidth: 340)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
            .padding()
    }
}

/// Under the terminal: the key bar and the button that shows or hides the keyboard (alone, in
/// a corner, when the key bar is turned off).
private struct TerminalBar: View {
    static let height: CGFloat = 46
    @ObservedObject var session: TerminalSession
    let showKeys: Bool
    let keyboardShown: Bool

    var body: some View {
        if showKeys {
            HStack(spacing: 0) {
                KeyBarView(store: .shared, session: session)
                Divider().frame(height: 26)
                toggle.frame(width: 44, height: Self.height)
            }
            .frame(height: Self.height)
            .background(.bar)
        } else {
            toggle
                .frame(width: 44, height: 44)
                .background(.regularMaterial, in: Circle())
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }

    private var toggle: some View {
        Button { session.toggleKeyboard(shown: keyboardShown) } label: {
            Image(systemName: keyboardShown ? "keyboard.chevron.compact.down" : "keyboard")
                .font(.system(size: 17))
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(keyboardShown ? "Hide keyboard" : "Show keyboard")
    }
}
