import SwiftUI

/// The app's screens with a tab bar of all seven at the bottom (the system's would fold the
/// last ones into "More"). A tab is made when first opened and keeps its state after.
struct MainView: View {
    @EnvironmentObject var app: AppModel
    @ObservedObject private var keyboard = KeyboardObserver.shared
    @State private var opened: Set<MainTab> = [.hosts]

    var body: some View {
        ZStack {
            ForEach(MainTab.allCases) { tab in
                if opened.contains(tab) {
                    screen(tab)
                        .opacity(app.tab == tab ? 1 : 0)
                        .allowsHitTesting(app.tab == tab)
                        .accessibilityHidden(app.tab != tab)
                }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if !keyboard.shown || app.showTerminals {
                TabBar(selection: $app.tab, sessions: app.sessions.count)
            }
        }
        .onChange(of: app.tab) { _, t in opened.insert(t) }
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

/// Seven tabs in one row, as the system's tab bar looks.
private struct TabBar: View {
    @Binding var selection: MainTab
    let sessions: Int

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 0) {
                ForEach(MainTab.allCases) { tab in
                    Button { selection = tab } label: {
                        VStack(spacing: 3) {
                            Image(systemName: tab.icon)
                                .font(.system(size: 19))
                                .frame(height: 24)
                                .overlay(alignment: .topTrailing) {
                                    if tab == .sessions && sessions > 0 {
                                        Text(verbatim: "\(sessions)")
                                            .font(.system(size: 10, weight: .bold))
                                            .foregroundStyle(.white)
                                            .padding(.horizontal, 4)
                                            .frame(minWidth: 15, minHeight: 15)
                                            .background(Color.red, in: Capsule())
                                            .offset(x: 9, y: -4)
                                    }
                                }
                            Text(tab.title)
                                .font(.system(size: 10, weight: .medium))
                                .lineLimit(1)
                                .minimumScaleFactor(0.7)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.top, 6)
                        .padding(.bottom, 2)
                        .foregroundStyle(selection == tab ? Color.accentColor : Color.secondary)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(selection == tab ? .isSelected : [])
                }
            }
            .padding(.horizontal, 2)
        }
        .background(.bar)
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
                        .swipeActions(edge: .trailing) {
                            Button(role: .destructive) { app.close(s) } label: { Label("Close", systemImage: "xmark") }
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
