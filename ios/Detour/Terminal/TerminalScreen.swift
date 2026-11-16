import GameController
import SwiftTerm
import SwiftUI
import UIKit

/// SwiftTerm has a `Color` of its own.
private typealias Color = SwiftUI.Color

/// SwiftTerm's view for a session, with the key bar over the keyboard.
struct TerminalHost: UIViewRepresentable {
    @ObservedObject var session: TerminalSession
    @ObservedObject var keys = KeyBarStore.shared

    func makeCoordinator() -> Coordinator { Coordinator(session: session) }

    func makeUIView(context: Context) -> TerminalView {
        let tv = TerminalView(frame: .zero)
        tv.terminalDelegate = context.coordinator
        tv.backgroundColor = .black
        tv.nativeBackgroundColor = .black
        tv.nativeForegroundColor = UIColor(white: 0.9, alpha: 1)
        tv.font = UIFont.monospacedSystemFont(ofSize: 13, weight: .regular)
        context.coordinator.view = tv
        context.coordinator.updateBar()
        session.view = tv
        DispatchQueue.main.async { _ = tv.becomeFirstResponder() }
        return tv
    }

    func updateUIView(_ tv: TerminalView, context: Context) {
        if context.coordinator.session !== session {
            context.coordinator.session = session
            session.view = tv
        }
        context.coordinator.updateBar()
    }

    @MainActor
    final class Coordinator: NSObject, TerminalViewDelegate {
        var session: TerminalSession
        weak var view: TerminalView?
        private var barHost: UIHostingController<KeyBarView>?
        private var keyboards: [NSObjectProtocol] = []

        init(session: TerminalSession) {
            self.session = session
            super.init()
            // A hardware keyboard (iPad) needs no bar.
            for name in [NSNotification.Name.GCKeyboardDidConnect, .GCKeyboardDidDisconnect] {
                keyboards.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.updateBar() }
                })
            }
        }

        func updateBar() {
            guard let view else { return }
            let store = KeyBarStore.shared
            if store.hidden || GCKeyboard.coalesced != nil {
                if view.inputAccessoryView != nil {
                    view.inputAccessoryView = nil
                    view.reloadInputViews()
                }
                return
            }
            let bar = KeyBarView(store: store, session: session)
            if let host = barHost {
                host.rootView = bar
            } else {
                let host = UIHostingController(rootView: bar)
                host.view.frame = CGRect(x: 0, y: 0, width: 320, height: 46)
                host.view.autoresizingMask = [.flexibleWidth]
                host.view.backgroundColor = .clear
                barHost = host
            }
            if view.inputAccessoryView !== barHost?.view {
                view.inputAccessoryView = barHost?.view
                view.reloadInputViews()
            }
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

/// A session's screen, with what it waits for over it.
private struct SessionScreen: View {
    @ObservedObject var session: TerminalSession
    @State private var password = ""

    var body: some View {
        ZStack {
            TerminalHost(session: session)
                .ignoresSafeArea(.container, edges: .bottom)
            overlay
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
