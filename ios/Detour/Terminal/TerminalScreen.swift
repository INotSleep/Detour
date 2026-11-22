import SwiftTerm
import SwiftUI
import UIKit

/// SwiftTerm has a `Color` of its own.
private typealias Color = SwiftUI.Color

/// SwiftTerm's view, with always something to delete. SwiftTerm says there is no text once what
/// was typed since the last Enter is gone, and iOS then stops repeating a held delete key, while
/// the shell's line may still have plenty: with no text of its own, SwiftTerm sends Backspace.
final class DetourTerminalView: TerminalView {
    override var hasText: Bool { true }

    /// Where the finger was when the space bar's trackpad last moved the cursor.
    private var trackpad: CGPoint?

    // Holding the space bar makes the keyboard a trackpad. SwiftTerm moves the cursor along the
    // line only (up and down in full-screen programs alone); here it is the arrows both ways
    // everywhere, so up and down walk the shell's history.
    override func beginFloatingCursor(at point: CGPoint) {
        trackpad = point
    }

    override func updateFloatingCursor(at point: CGPoint) {
        guard let last = trackpad else { return }
        let dx = point.x - last.x
        let dy = point.y - last.y
        if abs(dx) >= 9 {
            arrow(dx > 0 ? "right" : "left")
            trackpad = point
        } else if abs(dy) >= 18 {
            arrow(dy > 0 ? "down" : "up")
            trackpad = point
        }
    }

    override func endFloatingCursor() {
        trackpad = nil
    }

    private func arrow(_ name: String) {
        let appCursor = terminalInputStateSnapshot()?.applicationCursor ?? false
        guard let bytes = KeyCodes.bytes(name, appCursor: appCursor) else { return }
        terminalDelegate?.send(source: self, data: bytes[...])
    }
}

/// SwiftTerm's view for a session. Its own bar over the keyboard is off: ours sits in the
/// layout under the terminal, where it can be used with the keyboard hidden too.
struct TerminalHost: UIViewRepresentable {
    @ObservedObject var session: TerminalSession
    @ObservedObject private var settings = TerminalSettings.shared
    @Environment(\.colorScheme) private var scheme

    func makeCoordinator() -> Coordinator { Coordinator(session: session) }

    func makeUIView(context: Context) -> TerminalView {
        let tv = DetourTerminalView(frame: .zero)
        tv.terminalDelegate = context.coordinator
        tv.apply(settings.theme(dark: scheme == .dark), fontSize: settings.fontSize)
        tv.inputAccessoryView = nil
        session.view = tv
        DispatchQueue.main.async { _ = tv.becomeFirstResponder() }
        return tv
    }

    func updateUIView(_ tv: TerminalView, context: Context) {
        let theme = settings.theme(dark: scheme == .dark)
        if context.coordinator.theme != theme.id || tv.font.pointSize != CGFloat(settings.fontSize) {
            context.coordinator.theme = theme.id
            tv.apply(theme, fontSize: settings.fontSize)
        }
        if context.coordinator.session !== session {
            context.coordinator.session = session
            session.view = tv
        }
    }

    @MainActor
    final class Coordinator: NSObject, TerminalViewDelegate {
        var session: TerminalSession
        var theme = ""

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

/// The open sessions: a bar (back to the hosts, the sessions' tabs, the keyboard switch, a new
/// session), and the current one: a terminal or a host's files.
struct TerminalsView: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.colorScheme) private var scheme
    @ObservedObject private var settings = TerminalSettings.shared
    @State private var newSession: NewSessionKind?
    @State private var snippets = false

    private var current: OpenSession? { app.sessions.first { $0.id == app.current } }

    var body: some View {
        VStack(spacing: 0) {
            switch current {
            case .terminal(let t):
                // The sessions' bar sits on the keyboard with the terminal's own: in reach.
                SessionScreen(session: t) { bar }.id(t.id)
            case .files(let f):
                FilesView(files: f).id(f.uid)
                Divider()
                bar
            case nil:
                Spacer()
                bar
            }
        }
        .background(Color(uiColor: settings.theme(dark: scheme == .dark).background))
        .ignoresSafeArea(.keyboard)
        .sheet(item: $newSession) { kind in NewSessionSheet(kind: kind) }
        .sheet(isPresented: $snippets) {
            SnippetsView(session: current?.terminal)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
        }
    }

    private var bar: some View {
        HStack(spacing: 4) {
            Button { app.showTerminals = false } label: {
                Image(systemName: "chevron.left").font(.body.weight(.semibold)).frame(width: 36, height: 36)
            }
            .accessibilityLabel("Hosts")
            ScrollViewReader { proxy in
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(app.sessions) { s in
                            SessionChip(session: s, selected: s.id == app.current, select: { app.current = s.id }, close: { app.close(s) })
                                .id(s.id)
                        }
                    }
                    .padding(.vertical, 6)
                }
                .onChange(of: app.current) { _, id in withAnimation { proxy.scrollTo(id) } }
            }
            if let t = current?.terminal {
                KeyboardSwitch(session: t)
            }
            Menu {
                Button { newSession = .terminal } label: { Label("Host", systemImage: "terminal") }
                Button { newSession = .files } label: { Label("SFTP", systemImage: "folder") }
                if current?.terminal != nil {
                    Divider()
                    Button { snippets = true } label: { Label("Snippets", systemImage: "chevron.left.forwardslash.chevron.right") }
                }
            } label: {
                Image(systemName: "plus").font(.body.weight(.semibold)).frame(width: 36, height: 36)
            }
            .accessibilityLabel("New session")
        }
        .padding(.horizontal, 6)
        .frame(height: SessionScreen<EmptyView>.sessionsHeight)
        .background(.bar)
    }
}

/// Switches a terminal between the system keyboard (ABC) and the keys keyboard.
private struct KeyboardSwitch: View {
    @ObservedObject var session: TerminalSession

    var body: some View {
        Button { session.showKeysKeyboard(!session.keysKeyboard) } label: {
            Group {
                if session.keysKeyboard {
                    Text(verbatim: "ABC").font(.footnote.weight(.bold))
                } else {
                    Image(systemName: "square.grid.3x3.fill").font(.body)
                }
            }
            .frame(width: 36, height: 36)
        }
        .accessibilityLabel(session.keysKeyboard ? "Letters keyboard" : "Keys keyboard")
    }
}

/// What a new session opens: a terminal or a host's files.
enum NewSessionKind: String, Identifiable {
    case terminal, files
    var id: String { rawValue }
}

/// A new session: the kind, then the host.
struct NewSessionSheet: View {
    @EnvironmentObject var app: AppModel
    @Environment(\.dismiss) private var dismiss
    @State var kind: NewSessionKind
    @State private var query = ""

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Picker("Session", selection: $kind) {
                        Text("Terminal").tag(NewSessionKind.terminal)
                        Text(verbatim: "SFTP").tag(NewSessionKind.files)
                    }
                    .pickerStyle(.segmented)
                }
                ForEach(filtered) { section in
                    Section(section.title) {
                        ForEach(section.hosts) { host in
                            Button {
                                if kind == .terminal { app.open(host) } else { app.openFiles(host) }
                                dismiss()
                            } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(host.label).font(.body.weight(.medium))
                                    Text("\(host.login)@\(host.address)").font(.caption.monospaced()).foregroundStyle(.secondary)
                                }
                            }
                            .tint(.primary)
                        }
                    }
                }
            }
            .searchable(text: $query)
            .navigationTitle("New session")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
    }

    private var filtered: [HostSection] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return app.sections }
        return app.sections.compactMap { s in
            let hosts = s.hosts.filter { "\($0.label) \($0.address) \($0.login)".lowercased().contains(q) }
            return hosts.isEmpty ? nil : HostSection(id: s.id, title: s.title, hosts: hosts)
        }
    }
}

/// A session's tab. The shell may set a long title (user@host: ~/a/long/path): it is cut short.
struct SessionChip: View {
    let session: OpenSession
    let selected: Bool
    let select: () -> Void
    let close: () -> Void

    var body: some View {
        switch session {
        case .terminal(let t): TerminalChip(session: t, selected: selected, select: select, close: close)
        case .files(let f): FilesChip(files: f, selected: selected, select: select, close: close)
        }
    }
}

private struct TerminalChip: View {
    @ObservedObject var session: TerminalSession
    let selected: Bool
    let select: () -> Void
    let close: () -> Void

    var body: some View {
        ChipFrame(icon: nil, dot: dot, title: session.title, selected: selected, select: select, close: close)
    }

    private var dot: Color {
        switch session.state {
        case .connected: return .green
        case .connecting: return .yellow
        default: return .red
        }
    }
}

private struct FilesChip: View {
    @ObservedObject var files: FilesSession
    let selected: Bool
    let select: () -> Void
    let close: () -> Void

    var body: some View {
        ChipFrame(icon: "folder", dot: dot, title: files.host.label, selected: selected, select: select, close: close)
    }

    private var dot: Color {
        switch files.state {
        case .ready: return .green
        case .connecting: return .yellow
        default: return .red
        }
    }
}

private struct ChipFrame: View {
    /// Longest a tab's title goes before it is cut.
    static let maxTitle: CGFloat = 130
    let icon: String?
    let dot: Color
    let title: String
    let selected: Bool
    let select: () -> Void
    let close: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(dot).frame(width: 7, height: 7)
            if let icon { Image(systemName: icon).font(.caption) }
            Text(title).font(.subheadline).lineLimit(1).truncationMode(.middle).frame(maxWidth: Self.maxTitle, alignment: .leading).fixedSize(horizontal: false, vertical: true)
            Button(action: close) { Image(systemName: "xmark").font(.caption2.weight(.bold)) }
                .buttonStyle(.plain)
                .accessibilityLabel("Close")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(selected ? Color.accentColor.opacity(0.2) : Color(.secondarySystemBackground), in: Capsule())
        .onTapGesture(perform: select)
    }
}

/// A session's screen, with what it waits for over it and the key bar under it.
private struct SessionScreen<Sessions: View>: View {
    /// The sessions' bar under the terminal.
    static var sessionsHeight: CGFloat { 48 }
    @ObservedObject var session: TerminalSession
    @ViewBuilder let sessions: Sessions
    @ObservedObject private var keyboard = KeyboardObserver.shared
    @ObservedObject private var keys = KeyBarStore.shared
    @State private var password = ""

    var body: some View {
        GeometryReader { geo in
            // How far the keyboard reaches into this screen.
            let lift = keyboard.shown ? max(0, geo.frame(in: .global).maxY - keyboard.top) : 0
            // The place over the keyboard stays the same whichever keyboard is up: the key bar over
            // the system one, Customize over the keys keyboard (as tall as the system one). So
            // swapping them moves nothing.
            let barShown = !keyboard.hardware && (!keys.hidden || session.keysKeyboard)
            let bottom = lift + (barShown ? TerminalBar.height : 0) + Self.sessionsHeight
            ZStack(alignment: .bottom) {
                // The terminal takes its new size at once rather than following the keyboard's
                // animation: resizing it on every frame is what made it jump.
                TerminalHost(session: session)
                    .padding(.bottom, bottom)
                    .transaction { $0.animation = nil }
                overlay
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .padding(.bottom, bottom)
                VStack(spacing: 0) {
                    Divider()
                    sessions
                    if barShown {
                        TerminalBar(session: session, keyboardShown: keyboard.shown)
                    }
                }
                // With no bar, the keyboard's button floats over the terminal's corner.
                .overlay(alignment: .topTrailing) {
                    if !barShown && !keyboard.hardware {
                        TerminalBar.toggle(session: session, keyboardShown: keyboard.shown)
                            .frame(width: 44, height: 44)
                            .background(.regularMaterial, in: Circle())
                            .padding(10)
                            .offset(y: -64)
                    }
                }
                .padding(.bottom, lift)
                .animation(keyboard.swapping ? nil : .easeOut(duration: keyboard.duration), value: lift)
            }
        }
        .ignoresSafeArea(.keyboard)
        .sheet(isPresented: $session.customizing) {
            NavigationStack {
                KeyboardEditor()
                    .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { session.customizing = false } } }
            }
        }
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
                if message == TerminalSession.localNetworkDenied, let url = URL(string: UIApplication.openSettingsURLString) {
                    Button("Open Settings") { UIApplication.shared.open(url) }
                }
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

/// Over the keyboard: the key bar and the button that shows or hides the keyboard; over the
/// keys keyboard (which has those keys and more), Customize and the same button.
private struct TerminalBar: View {
    static let height: CGFloat = 46
    @ObservedObject var session: TerminalSession
    let keyboardShown: Bool

    var body: some View {
        HStack(spacing: 0) {
            if session.keysKeyboard {
                Button { session.customizing = true } label: {
                    Label("Customize", systemImage: "slider.horizontal.3")
                        .font(.subheadline.weight(.medium))
                        .frame(maxWidth: .infinity, minHeight: 32)
                }
                .buttonStyle(.bordered)
                .padding(.horizontal, 6)
            } else {
                KeyBarView(store: .shared, session: session)
            }
            Divider().frame(height: 26)
            Self.toggle(session: session, keyboardShown: keyboardShown).frame(width: 44, height: Self.height)
        }
        .frame(height: Self.height)
        .background(.bar)
    }

    static func toggle(session: TerminalSession, keyboardShown: Bool) -> some View {
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
