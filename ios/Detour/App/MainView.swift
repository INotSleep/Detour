import SwiftUI

/// The app's screens with a tab bar of all seven at the bottom (the system's would fold the
/// last ones into "More"). A tab is made when first opened and keeps its state after. Going to
/// another tab slides its screen in from that tab's side, as the tabs lie in the bar.
struct MainView: View {
    @EnvironmentObject var app: AppModel
    @ObservedObject private var keyboard = KeyboardObserver.shared
    @State private var opened: Set<MainTab> = [.hosts]
    /// Where each screen is pushed sideways while tabs change.
    @State private var offsets: [MainTab: CGFloat] = [:]
    /// The screen sliding out.
    @State private var leaving: MainTab?

    var body: some View {
        // The screens end where the tab bar starts: what a screen puts at its bottom (Undo) shows.
        VStack(spacing: 0) {
            screens
            if !keyboard.shown || app.showTerminals {
                TabBar(selection: $app.tab, sessions: app.sessions.count)
            }
        }
    }

    private var screens: some View {
        GeometryReader { geo in
            ZStack {
                ForEach(MainTab.allCases) { tab in
                    if opened.contains(tab) {
                        screen(tab)
                            .offset(x: offsets[tab] ?? 0)
                            .opacity(tab == app.tab || tab == leaving ? 1 : 0)
                            .allowsHitTesting(app.tab == tab)
                            .accessibilityHidden(app.tab != tab)
                    }
                }
            }
            .clipped()
            .onChange(of: app.tab) { old, new in slide(from: old, to: new, width: geo.size.width) }
        }
    }

    /// The new screen comes in from the side its tab is on, the old one goes out the other way.
    private func slide(from old: MainTab, to new: MainTab, width: CGFloat) {
        let all = MainTab.allCases
        let dir: CGFloat = (all.firstIndex(of: new) ?? 0) > (all.firstIndex(of: old) ?? 0) ? 1 : -1
        var still = Transaction()
        still.disablesAnimations = true
        withTransaction(still) {
            opened.insert(new)
            leaving = old
            offsets[old] = 0
            offsets[new] = dir * width
        }
        // The starting place has to be drawn before the move, or there is nothing to move from.
        DispatchQueue.main.async {
            withAnimation(.spring(response: 0.36, dampingFraction: 0.9)) {
                offsets[new] = 0
                offsets[old] = -dir * width
            } completion: {
                withTransaction(still) {
                    if leaving == old { leaving = nil }
                    if app.tab != old { offsets[old] = 0 }
                }
            }
        }
    }

    @ViewBuilder private func screen(_ tab: MainTab) -> some View {
        switch tab {
        case .hosts: HostsView()
        case .sessions: SessionsView()
        case .keys: NavigationStack { KeysView() }
        case .snippets: SnippetsView(session: nil)
        case .proxies: ProxiesView()
        case .teams: NavigationStack { TeamsView() }
        case .settings: SettingsView()
        }
    }
}

extension MainTab {
    var title: String {
        switch self {
        case .hosts: String(localized: "Hosts")
        case .sessions: String(localized: "Sessions")
        // "Keys" is the keyboard's keys elsewhere: SSH keys have a word of their own.
        case .keys: String(localized: "tab.keys", defaultValue: "Keys")
        case .snippets: String(localized: "Snippets")
        case .proxies: String(localized: "Proxies")
        case .teams: String(localized: "Teams")
        case .settings: String(localized: "Settings")
        }
    }

    var icon: String {
        switch self {
        case .hosts: "server.rack"
        case .sessions: "terminal"
        case .keys: "key"
        case .snippets: "chevron.left.forwardslash.chevron.right"
        case .proxies: "shield"
        case .teams: "person.3"
        case .settings: "gearshape"
        }
    }
}

/// Seven tabs in one row: the chosen one an icon and its name in a pill, the others icons only;
/// the pill slides to the tab tapped.
private struct TabBar: View {
    @Binding var selection: MainTab
    let sessions: Int
    @Namespace private var pill

    var body: some View {
        HStack(spacing: 2) {
            ForEach(MainTab.allCases) { tab in
                let chosen = selection == tab
                Button {
                    Haptics.select()
                    withAnimation(.spring(response: 0.35, dampingFraction: 0.82)) { selection = tab }
                } label: {
                    HStack(spacing: 6) {
                        icon(tab)
                        if chosen {
                            Text(tab.title)
                                .font(.subheadline.weight(.semibold))
                                .lineLimit(1)
                                .fixedSize()
                                .transition(.opacity.combined(with: .scale(scale: 0.8, anchor: .leading)))
                        }
                    }
                    .padding(.horizontal, chosen ? 14 : 0)
                    .frame(height: 40)
                    .frame(maxWidth: chosen ? nil : .infinity)
                    .background {
                        if chosen {
                            Capsule().fill(Color.accentColor.opacity(0.16)).matchedGeometryEffect(id: "pill", in: pill)
                        }
                    }
                    .foregroundStyle(chosen ? Color.accentColor : Color.secondary)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(tab.title)
                .accessibilityAddTraits(chosen ? .isSelected : [])
            }
        }
        .padding(.horizontal, 8)
        .padding(.top, 6)
        .padding(.bottom, 2)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
    }

    private func icon(_ tab: MainTab) -> some View {
        Image(systemName: tab.icon)
            .font(.system(size: 18, weight: .medium))
            .frame(width: 26, height: 24)
            .overlay(alignment: .topTrailing) {
                if tab == .sessions && sessions > 0 {
                    Text(verbatim: "\(sessions)")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 4)
                        .frame(minWidth: 15, minHeight: 15)
                        .background(Color.red, in: Capsule())
                        .offset(x: 8, y: -5)
                }
            }
    }
}

/// The open sessions, terminals and files: a tap brings one forward, a swipe closes it.
struct SessionsView: View {
    @EnvironmentObject var app: AppModel
    @State private var newSession: NewSessionKind?

    var body: some View {
        NavigationStack {
            List {
                ForEach(app.sessions) { s in
                    Button { app.show(s) } label: { SessionRow(session: s) }
                        .tint(.primary)
                        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                            Button(role: .destructive) { app.close(s) } label: { Label("Close", systemImage: "xmark") }
                                .tint(.red)
                        }
                }
            }
            .overlay {
                if app.sessions.isEmpty {
                    ContentUnavailableView {
                        Label("No sessions", systemImage: "terminal")
                    } actions: {
                        Button("New session") { newSession = .terminal }.buttonStyle(.borderedProminent)
                    }
                }
            }
            .navigationTitle("Sessions")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Menu {
                        Button { newSession = .terminal } label: { Label("Host", systemImage: "terminal") }
                        Button { newSession = .files } label: { Label("SFTP", systemImage: "folder") }
                    } label: {
                        Image(systemName: "plus")
                    }
                    .accessibilityLabel("New session")
                }
            }
            .sheet(item: $newSession) { kind in NewSessionSheet(kind: kind) }
        }
    }
}

private struct SessionRow: View {
    let session: OpenSession

    var body: some View {
        switch session {
        case .terminal(let t): TerminalRow(session: t)
        case .files(let f): FilesRow(files: f)
        }
    }

    private struct TerminalRow: View {
        @ObservedObject var session: TerminalSession

        var body: some View {
            SessionRowLayout(icon: "terminal", title: session.title, host: session.host, live: session.state == .connected)
        }
    }

    private struct FilesRow: View {
        @ObservedObject var files: FilesSession

        var body: some View {
            SessionRowLayout(icon: "folder", title: files.host.label, host: files.host, live: files.state == .ready)
        }
    }
}

private struct SessionRowLayout: View {
    let icon: String
    let title: String
    let host: Host
    let live: Bool

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon).foregroundStyle(Color.accentColor).frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.body.weight(.medium)).lineLimit(1).truncationMode(.middle)
                Text("\(host.login)@\(host.address)").font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer()
            Circle().fill(live ? Color.green : Color.orange).frame(width: 8, height: 8)
        }
    }
}
