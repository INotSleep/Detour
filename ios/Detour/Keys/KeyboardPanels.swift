import SwiftUI

/// What the keys keyboard shows: its keys, the snippets, or the shell's history.
enum KeysPanel: String, CaseIterable, Identifiable {
    case keys, snippets, history
    var id: String { rawValue }

    var title: LocalizedStringKey {
        switch self {
        case .keys: "Keys"
        case .snippets: "Snippets"
        case .history: "History"
        }
    }
}

/// The snippets on the keyboard: a tap runs one.
struct SnippetsPanel: View {
    @ObservedObject var session: TerminalSession
    @State private var groups: [SnippetGroup]?

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 6) {
                if let groups {
                    if groups.isEmpty {
                        Text("No snippets").foregroundStyle(.secondary).frame(maxWidth: .infinity).padding(.top, 40)
                    }
                    ForEach(groups) { g in
                        if let name = g.name {
                            Text(name).font(.caption.weight(.semibold)).foregroundStyle(.secondary).padding(.top, 4)
                        }
                        ForEach(g.snippets) { s in
                            PanelRow(title: s.name, detail: s.command) { session.run(s.command) }
                        }
                    }
                } else {
                    ProgressView().frame(maxWidth: .infinity).padding(.top, 40)
                }
            }
            .padding(.vertical, 4)
        }
        .task { groups = await SnippetGroup.load() }
    }
}

/// The shell's history on the server, newest first: a tap types a command at the prompt to change
/// or run, the arrow runs it at once.
struct HistoryPanel: View {
    @ObservedObject var session: TerminalSession
    /// Kept with the session: what was read shows at once, and is read again behind it.
    @ObservedObject var history: ShellHistory

    init(session: TerminalSession) {
        self.session = session
        self.history = session.history
    }

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 6) {
                if let lines = history.lines {
                    if lines.isEmpty {
                        Text("No history yet").foregroundStyle(.secondary).frame(maxWidth: .infinity).padding(.top, 40)
                    }
                    ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                        PanelRow(title: line, detail: nil, mono: true, run: { session.run(line) }) {
                            session.input(Array(line.utf8))
                        }
                    }
                } else if let error = history.error {
                    VStack(spacing: 10) {
                        Text(error).font(.footnote).foregroundStyle(.secondary).multilineTextAlignment(.center)
                        Button("Try again") { Task { await history.load(session) } }.buttonStyle(.bordered)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.top, 30)
                } else {
                    ProgressView().frame(maxWidth: .infinity).padding(.top, 40)
                }
            }
            .padding(.vertical, 4)
        }
        .task { await history.load(session) }
    }
}

/// A row of a panel: a tap does its thing; with `run`, an arrow on the right runs it.
private struct PanelRow: View {
    let title: String
    let detail: String?
    var mono = false
    var run: (() -> Void)? = nil
    let tap: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Button {
                Haptics.tap()
                tap()
            } label: {
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .font(mono ? .system(size: 14, design: .monospaced) : .system(size: 15, weight: .medium))
                        .lineLimit(mono ? 2 : 1)
                        .truncationMode(.tail)
                    if let detail {
                        Text(detail).font(.system(size: 12, design: .monospaced)).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 8))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if let run {
                Button {
                    Haptics.tap()
                    run()
                } label: {
                    Image(systemName: "return")
                        .font(.system(size: 15, weight: .semibold))
                        .frame(width: 44, height: 40)
                        .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 8))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Run")
            }
        }
    }
}

/// The commands run in this app's sessions to the host (newest first), then the history files of
/// the host's shells (bash, zsh, fish), read over a connection of its own with the session's login.
/// A shell writes its file when it exits: what the open sessions ran comes from the screen instead.
@MainActor
final class ShellHistory: ObservableObject {
    @Published var lines: [String]?
    @Published var error: String?

    /// Runs under sh whatever the login shell is (fish can't read a POSIX loop).
    private static let command = #"sh -c 'for f in "$HOME/.bash_history" "$HOME/.zsh_history" "$HOME/.local/share/fish/fish_history"; do [ -f "$f" ] && tail -n 1000 "$f"; done'"#

    private final class Buffer {
        var data = Data()
        var ended = false
    }

    private var loading = false

    func load(_ session: TerminalSession) async {
        guard !loading else { return }
        loading = true
        defer { loading = false }
        let host = session.host
        let typed = TypedHistory.shared.list(host: host.id)
        // What was read before stays up while it is read again.
        if let before = lines {
            var seen = Set<String>()
            lines = (typed + before).filter { seen.insert($0).inserted }
        } else {
            lines = typed.isEmpty ? nil : typed
            error = nil
        }
        let stream = Core.shared.newStream()
        let buffer = Buffer()
        Core.shared.watch(stream: stream, data: { buffer.data.append($0) }, end: { _ in buffer.ended = true })
        defer { Core.shared.unwatch(stream: stream) }
        do {
            _ = try await Core.shared.call("host_exec", ["vaultId": host.vaultId, "hostId": host.id, "command": Self.command, "stream": stream, "secret": session.typedSecret])
            // The output comes before its end, but on the main queue after the call returns: wait
            // for the end (not long) so none of it is missed.
            for _ in 0..<30 where !buffer.ended { try? await Task.sleep(for: .milliseconds(100)) }
            var seen = Set(typed)
            lines = typed + Self.parse(String(decoding: buffer.data, as: UTF8.self)).filter { seen.insert($0).inserted }
            error = nil
        } catch let f as CoreFailure {
            // What was run here still shows when the server's files can't be read.
            if lines?.isEmpty ?? true { error = f.kind == "needSecret" ? String(localized: "The history needs the host's password") : TerminalSession.describe(f) }
        } catch {
            if lines?.isEmpty ?? true { self.error = error.localizedDescription }
        }
    }

    /// Commands, newest first, each once. zsh writes ": <time>:0;command", fish "- cmd: command",
    /// bash may put "#<time>" lines between.
    static func parse(_ text: String) -> [String] {
        var out: [String] = []
        var seen = Set<String>()
        for raw in text.split(whereSeparator: \.isNewline).reversed() {
            var line = String(raw)
            if line.hasPrefix(": "), let i = line.firstIndex(of: ";") {
                line = String(line[line.index(after: i)...])
            } else if line.hasPrefix("- cmd: ") {
                line = String(line.dropFirst("- cmd: ".count))
            } else if line.hasPrefix("  when:") || line.hasPrefix("  paths:") || (line.hasPrefix("#") && Int(line.dropFirst()) != nil) {
                continue
            }
            line = line.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty, seen.insert(line).inserted else { continue }
            out.append(line)
            if out.count >= 300 { break }
        }
        return out
    }
}

/// Commands run in this app's sessions, by host, newest first. Kept in memory only, for as long as
/// the app runs: what people type may hold secrets.
@MainActor
final class TypedHistory {
    static let shared = TypedHistory()
    private var byHost: [String: [String]] = [:]

    func add(_ command: String, host: String) {
        guard !command.isEmpty else { return }
        var list = byHost[host] ?? []
        list.removeAll { $0 == command }
        list.insert(command, at: 0)
        byHost[host] = Array(list.prefix(300))
    }

    func list(host: String) -> [String] { byHost[host] ?? [] }

    /// A line of the screen without its prompt: what follows the first "$ ", "# ", "% ", "❯ " or
    /// "> " (bash, root, zsh, starship and the like); nil when the line has none of them.
    nonisolated static func command(afterPrompt line: String) -> String? {
        let ends = ["$ ", "# ", "% ", "❯ ", "> "]
        let found = ends.compactMap { line.range(of: $0) }.min { $0.lowerBound < $1.lowerBound }
        guard let found else { return nil }
        let command = line[found.upperBound...].trimmingCharacters(in: .whitespaces)
        return command.isEmpty ? nil : command
    }
}
