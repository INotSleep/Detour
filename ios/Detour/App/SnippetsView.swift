import SwiftUI

struct Snippet: Decodable, Identifiable, Hashable {
    let id: String
    let name: String
    let command: String
}

private struct SnippetsData: Decodable {
    let snippets: [Snippet]
}

/// Snippets of one vault: the personal one (no name) or a team's.
struct SnippetGroup: Identifiable {
    let id: String
    let name: String?
    let snippets: [Snippet]

    static func load() async -> [SnippetGroup] {
        let mine = (try? await Core.shared.call("vault_get", as: SnippetsData.self).snippets) ?? []
        let teams = (try? await Core.shared.call("teams_list", as: [TeamSnippets].self)) ?? []
        var out = mine.isEmpty ? [] : [SnippetGroup(id: "personal", name: nil, snippets: mine)]
        for t in teams {
            guard let v = t.view, !v.data.snippets.isEmpty else { continue }
            out.append(SnippetGroup(id: v.id, name: v.name, snippets: v.data.snippets))
        }
        return out
    }
}

private struct TeamSnippets: Decodable {
    let view: Team?

    struct Team: Decodable {
        let id: String
        let name: String
        let data: SnippetsData
    }
}

/// Snippets of the vault and its teams. With a session, a tap runs one in it; without one, a tap
/// opens it (to change, for the personal vault's; to read and copy, for a team's). Deleting
/// one can be undone for a few seconds.
struct SnippetsView: View {
    /// Where a tap runs a snippet; nil only manages them.
    var session: TerminalSession?
    @Environment(\.dismiss) private var dismiss
    @State private var mine: [Snippet]?
    @State private var teams: [TeamList] = []

    private struct TeamList: Identifiable {
        let id: String
        let name: String
        let snippets: [Snippet]
    }
    @State private var editing: Snippet?
    /// A team's snippet, shown to read and copy.
    @State private var viewing: Snippet?
    @State private var adding = false
    @State private var error: String?
    /// A snippet just deleted, for Undo.
    @State private var deleted: Snippet?
    @State private var deletedAt = Date.distantPast

    var body: some View {
        NavigationStack {
            List {
                if let mine {
                    Section {
                        ForEach(mine) { s in
                            row(s, open: { editing = s })
                                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                                    Button(role: .destructive) { Task { await delete(s) } } label: { Label("Delete", systemImage: "trash") }
                                        .tint(.red)
                                    Button { editing = s } label: { Label("Edit", systemImage: "pencil") }.tint(.accentColor)
                                }
                                .contextMenu {
                                    Button { editing = s } label: { Label("Edit", systemImage: "pencil") }
                                    Button { UIPasteboard.general.string = s.command } label: { Label("Copy", systemImage: "doc.on.doc") }
                                    Button(role: .destructive) { Task { await delete(s) } } label: { Label("Delete", systemImage: "trash") }
                                }
                        }
                    } header: {
                        if !teams.isEmpty { Text("My snippets") }
                    }
                }
                ForEach(teams) { t in
                    Section(t.name) {
                        ForEach(t.snippets) { s in
                            row(s, open: { viewing = s }).contextMenu {
                                Button { UIPasteboard.general.string = s.command } label: { Label("Copy", systemImage: "doc.on.doc") }
                            }
                        }
                    }
                }
                if let error {
                    Text(error).foregroundStyle(.red).font(.footnote)
                }
            }
            .overlay {
                if mine == nil {
                    ProgressView()
                } else if mine?.isEmpty == true && teams.isEmpty {
                    ContentUnavailableView {
                        Label("No snippets", systemImage: "chevron.left.forwardslash.chevron.right")
                    } actions: {
                        Button("New snippet") { adding = true }.buttonStyle(.borderedProminent)
                    }
                }
            }
            .navigationTitle("Snippets")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if session != nil {
                    ToolbarItem(placement: .cancellationAction) { Button("Close") { dismiss() } }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button { adding = true } label: { Image(systemName: "plus") }
                        .accessibilityLabel("New snippet")
                }
            }
            .sheet(isPresented: $adding) { SnippetEditor(snippet: nil, saved: { await load() }) }
            .sheet(item: $editing) { s in SnippetEditor(snippet: s, saved: { await load() }) }
            .sheet(item: $viewing) { s in SnippetViewer(snippet: s) }
            .safeAreaInset(edge: .bottom) { undoBar }
            .task { await load() }
            .refreshable { await load() }
        }
    }

    private func row(_ s: Snippet, open: @escaping () -> Void) -> some View {
        Button {
            guard let session else { return open() }
            session.run(s.command)
            dismiss()
        } label: {
            VStack(alignment: .leading, spacing: 3) {
                Text(s.name).font(.body.weight(.medium)).foregroundStyle(.primary)
                Text(s.command).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(2)
            }
        }
        // A row, not a link: iOS 26 tints a list's buttons with the accent colour.
        .tint(.primary)
    }

    private func load() async {
        mine = (try? await Core.shared.call("vault_get", as: SnippetsData.self).snippets) ?? []
        let list = (try? await Core.shared.call("teams_list", as: [TeamSnippets].self)) ?? []
        teams = list.compactMap { t in
            guard let v = t.view, !v.data.snippets.isEmpty else { return nil }
            return TeamList(id: v.id, name: v.name, snippets: v.data.snippets)
        }
    }

    @ViewBuilder private var undoBar: some View {
        if let d = deleted {
            HStack {
                Text("Deleted \(d.name)").lineLimit(1)
                Spacer()
                Button("Undo") { Task { await undo() } }.bold()
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14))
            .padding()
            .transition(.move(edge: .bottom).combined(with: .opacity))
        }
    }

    private func undo() async {
        guard let d = deleted else { return }
        withAnimation { deleted = nil }
        let item: PersonalVault.JSON = ["id": d.id, "name": d.name, "command": d.command]
        try? await PersonalVault.edit { v in v["snippets"] = (v["snippets"] as? [PersonalVault.JSON] ?? []) + [item] }
        await load()
    }

    private func delete(_ s: Snippet) async {
        withAnimation { deleted = s }
        let at = Date()
        deletedAt = at
        Task {
            try? await Task.sleep(for: .seconds(5))
            if deletedAt == at { withAnimation { deleted = nil } }
        }
        do {
            try await PersonalVault.edit { v in
                v["snippets"] = (v["snippets"] as? [PersonalVault.JSON] ?? []).filter { $0["id"] as? String != s.id }
            }
            error = nil
        } catch let f as CoreFailure {
            error = f["message"] ?? f.kind
        } catch {
            self.error = error.localizedDescription
        }
        await load()
    }
}

private struct SnippetEditor: View {
    let snippet: Snippet?
    let saved: () async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var command = ""
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $name)
                }
                Section {
                    TextEditor(text: $command)
                        .font(.body.monospaced())
                        .frame(minHeight: 120)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                } header: {
                    Text("Command")
                }
                if let error { Text(error).foregroundStyle(.red).font(.footnote) }
            }
            .navigationTitle(snippet == nil ? "New snippet" : "Edit snippet")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    if busy { ProgressView() } else {
                        Button("Save") { Task { await save() } }
                            .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || command.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    }
                }
            }
            .onAppear {
                if let snippet, name.isEmpty, command.isEmpty {
                    name = snippet.name
                    command = snippet.command
                }
            }
        }
    }

    private func save() async {
        busy = true
        let item: PersonalVault.JSON = ["id": snippet?.id ?? PersonalVault.newId(), "name": name.trimmingCharacters(in: .whitespaces), "command": command]
        do {
            try await PersonalVault.edit { v in
                var list = v["snippets"] as? [PersonalVault.JSON] ?? []
                if let i = list.firstIndex(where: { $0["id"] as? String == item["id"] as? String }) { list[i] = item } else { list.append(item) }
                v["snippets"] = list
            }
            await saved()
            dismiss()
        } catch let f as CoreFailure {
            error = f["message"] ?? f.kind
        } catch {
            self.error = error.localizedDescription
        }
        busy = false
    }
}

/// A team's snippet: read it, copy it.
private struct SnippetViewer: View {
    let snippet: Snippet
    @Environment(\.dismiss) private var dismiss
    @State private var copies = 0

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(snippet.command).font(.body.monospaced()).textSelection(.enabled)
                }
                Section {
                    Button {
                        UIPasteboard.general.string = snippet.command
                        copies += 1
                    } label: {
                        Label(copies > 0 ? String(localized: "Copied") : String(localized: "Copy"), systemImage: copies > 0 ? "checkmark" : "doc.on.doc")
                    }
                    .sensoryFeedback(.success, trigger: copies)
                }
            }
            .navigationTitle(snippet.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
    }
}
