import SwiftUI

/// Settings of the keys: the bar over the keyboard as it looks, then the keys keyboard's rows of
/// eight. A tap on a key (or an empty place) picks what goes there.
struct KeyboardEditor: View {
    @ObservedObject private var bar = KeyBarStore.shared
    @ObservedObject private var keys = KeysKeyboardStore.shared
    @State private var picking: Slot?
    @State private var confirmReset = false

    /// A place a key goes: in a set of the bar (nil: a new key at its end) or a row of the keyboard.
    enum Slot: Identifiable {
        case bar(set: UUID, index: Int?)
        case block(UUID, Int)

        var id: String {
            switch self {
            case .bar(let s, let i): "bar/\(s)/\(i.map(String.init) ?? "new")"
            case .block(let b, let i): "block/\(b)/\(i)"
            }
        }
    }

    var body: some View {
        List {
            Section {
                Toggle("Show the key bar", isOn: Binding(get: { !bar.hidden }, set: { bar.hidden = !$0 }))
                if !bar.hidden {
                    ForEach($bar.sets) { $set in
                        VStack(alignment: .leading, spacing: 8) {
                            if bar.sets.count > 1 {
                                TextField("Set name", text: $set.name).font(.footnote).foregroundStyle(.secondary)
                            }
                            ScrollView(.horizontal, showsIndicators: false) {
                                HStack(spacing: 4) {
                                    ForEach(Array(set.keys.enumerated()), id: \.element.id) { i, key in
                                        Button { picking = .bar(set: set.id, index: i) } label: { KeyCap(key: key, width: KeyCap.barWidth, height: 34) }
                                            .buttonStyle(.plain)
                                    }
                                    Button { picking = .bar(set: set.id, index: nil) } label: { addCap(width: KeyCap.barWidth, height: 34) }
                                        .buttonStyle(.plain)
                                        .accessibilityLabel("Add key")
                                }
                                .padding(.vertical, 2)
                            }
                        }
                        .swipeActions(edge: .trailing) {
                            if bar.sets.count > 1 {
                                Button(role: .destructive) { bar.sets.removeAll { $0.id == set.id } } label: { Label("Delete", systemImage: "trash") }
                            }
                        }
                    }
                    Button {
                        bar.sets.append(KeySet(name: String(localized: "Set \(bar.sets.count + 1)"), keys: []))
                    } label: {
                        Label("Add set", systemImage: "plus.rectangle.on.rectangle")
                    }
                }
            } header: {
                Text("Over the keyboard")
            }

            Section {
                ForEach(keys.blocks) { block in
                    HStack(spacing: 5) {
                        ForEach(0..<KeyBlock.size, id: \.self) { i in
                            Button { picking = .block(block.id, i) } label: {
                                KeyCap(key: block.keys.indices.contains(i) ? block.keys[i] : nil, height: 40)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .swipeActions(edge: .trailing) {
                        Button(role: .destructive) { keys.blocks.removeAll { $0.id == block.id } } label: { Label("Delete", systemImage: "trash") }
                    }
                }
                .onMove { keys.blocks.move(fromOffsets: $0, toOffset: $1) }
                Button { keys.blocks.append(KeyBlock()) } label: { Label("Add row", systemImage: "plus") }
            } header: {
                Text("Keys keyboard")
            }

            Section {
                Button("Reset to default", role: .destructive) { confirmReset = true }
            }
        }
        .navigationTitle("Keyboard")
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog("Put back the default keys?", isPresented: $confirmReset, titleVisibility: .visible) {
            Button("Reset", role: .destructive) {
                bar.reset()
                keys.reset()
            }
        }
        .sheet(item: $picking) { slot in
            KeyPicker(current: key(at: slot), movable: movable(slot)) { apply($0, to: slot) }
        }
    }

    private func addCap(width: CGFloat?, height: CGFloat) -> some View {
        Image(systemName: "plus")
            .font(.system(size: 14, weight: .semibold))
            .foregroundStyle(Color.accentColor)
            .frame(width: width, height: height)
            .background(Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 7))
    }

    private func key(at slot: Slot) -> BarKey? {
        switch slot {
        case .bar(let s, let i):
            guard let i, let set = bar.sets.first(where: { $0.id == s }), set.keys.indices.contains(i) else { return nil }
            return set.keys[i]
        case .block(let b, let i):
            guard let block = keys.blocks.first(where: { $0.id == b }), block.keys.indices.contains(i) else { return nil }
            return block.keys[i]
        }
    }

    private func movable(_ slot: Slot) -> Bool {
        if case .bar(_, let i) = slot { return i != nil }
        return false
    }

    private func apply(_ change: KeyPicker.Change, to slot: Slot) {
        switch slot {
        case .bar(let s, let i):
            guard let si = bar.sets.firstIndex(where: { $0.id == s }) else { return }
            var list = bar.sets[si].keys
            switch change {
            case .set(var key):
                if let i, list.indices.contains(i) {
                    key.id = list[i].id
                    list[i] = key
                } else {
                    list.append(key)
                }
            case .remove:
                if let i, list.indices.contains(i) { list.remove(at: i) }
            case .move(let by):
                if let i, list.indices.contains(i), list.indices.contains(i + by) { list.swapAt(i, i + by) }
            }
            bar.sets[si].keys = list
        case .block(let b, let i):
            guard let bi = keys.blocks.firstIndex(where: { $0.id == b }) else { return }
            var block = keys.blocks[bi]
            while block.keys.count < KeyBlock.size { block.keys.append(nil) }
            switch change {
            case .set(let key): block.keys[i] = key
            case .remove: block.keys[i] = nil
            case .move: break
            }
            keys.blocks[bi] = block
        }
    }
}

/// Picks what a key is: one of the usual keys, a combination, or a snippet.
struct KeyPicker: View {
    enum Change {
        case set(BarKey)
        case remove
        /// Swap with the key before (-1) or after (1).
        case move(Int)
    }

    enum Tab: String, CaseIterable, Identifiable {
        case keys, combination, snippet
        var id: String { rawValue }
    }

    let current: BarKey?
    let movable: Bool
    let done: (Change) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var tab: Tab = .keys

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("Kind", selection: $tab) {
                        Text("Keys").tag(Tab.keys)
                        Text("Combination").tag(Tab.combination)
                        Text("Snippet").tag(Tab.snippet)
                    }
                    .pickerStyle(.segmented)
                }
                switch tab {
                case .keys: KeyTable(pick: pick)
                case .combination: ComboBuilder(start: current?.kind == .combo ? current : nil, pick: pick)
                case .snippet: SnippetChooser(pick: pick)
                }
                if current != nil {
                    Section {
                        if movable {
                            Button { finish(.move(-1)) } label: { Label("Move left", systemImage: "arrow.left") }
                            Button { finish(.move(1)) } label: { Label("Move right", systemImage: "arrow.right") }
                        }
                        Button(role: .destructive) { finish(.remove) } label: { Label("Remove", systemImage: "trash") }
                    }
                }
            }
            .navigationTitle(current.map { $0.title } ?? String(localized: "Add key"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
            .onAppear {
                switch current?.kind {
                case .combo: tab = .combination
                case .snippet, .text: tab = .snippet
                default: tab = .keys
                }
            }
        }
    }

    private func pick(_ key: BarKey) { finish(.set(key)) }

    private func finish(_ c: Change) {
        done(c)
        dismiss()
    }
}

/// The usual keys, by kind: everything a terminal on a phone may need.
private struct KeyTable: View {
    let pick: (BarKey) -> Void

    private func key(_ v: String) -> BarKey { BarKey(kind: .key, value: v) }

    var body: some View {
        group("Modifiers", [BarKey(kind: .ctrl), BarKey(kind: .alt)])
        group("Keys", ["esc", "tab", "enter", "backspace", "delete", "insert", "space"].map(key))
        group("Arrows and pages", ["left", "down", "up", "right", "home", "end", "pageup", "pagedown"].map(key))
        group("Function keys", (1...24).map { key("f\($0)") })
        group("Symbols", "|~/\\-_=+<>{}[]()!@#$%^&*'\"`,.?;:".map { key(String($0)) })
        group("Ctrl and a letter", "abcdefghijklmnopqrstuvwxyz".map { BarKey(kind: .combo, value: "ctrl+\($0)") })
    }

    private func group(_ title: LocalizedStringKey, _ keys: [BarKey]) -> some View {
        Section(title) {
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 6), count: 6), spacing: 6) {
                ForEach(keys) { k in
                    Button { pick(k) } label: { KeyCap(key: k, height: 40) }
                        .buttonStyle(.plain)
                }
            }
            .padding(.vertical, 4)
        }
    }
}

/// A combination: modifiers and a key (a named one, or any character typed).
private struct ComboBuilder: View {
    let start: BarKey?
    let pick: (BarKey) -> Void
    @State private var ctrl = true
    @State private var alt = false
    @State private var shift = false
    @State private var key = ""

    private var value: String {
        ([ctrl ? "ctrl" : nil, alt ? "alt" : nil, shift ? "shift" : nil].compactMap { $0 } + [key]).joined(separator: "+")
    }

    private var made: BarKey { BarKey(kind: .combo, value: value) }
    private var valid: Bool { !key.isEmpty && (ctrl || alt || shift) && made.bytes() != nil }

    var body: some View {
        Section {
            HStack(spacing: 8) {
                modifier("Ctrl", $ctrl)
                modifier("Alt", $alt)
                modifier("Shift", $shift)
                Spacer()
                KeyCap(key: key.isEmpty ? nil : made, on: valid, width: 56, height: 40)
            }
            TextField("Letter, digit or symbol", text: Binding(get: { key.count == 1 ? key : "" }, set: { key = String($0.suffix(1)) }))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.body.monospaced())
            Button("Use") { pick(made) }.disabled(!valid)
        }
        Section("Or a key") {
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 6), count: 6), spacing: 6) {
                ForEach(KeyCodes.named, id: \.self) { n in
                    Button { key = n } label: { KeyCap(key: BarKey(kind: .key, value: n), on: key == n, height: 40) }
                        .buttonStyle(.plain)
                }
            }
            .padding(.vertical, 4)
        }
        .onAppear {
            guard let c = start?.comboParts else { return }
            ctrl = c.ctrl
            alt = c.alt
            shift = c.shift
            key = c.key
        }
    }

    private func modifier(_ name: String, _ on: Binding<Bool>) -> some View {
        Button { on.wrappedValue.toggle() } label: {
            Text(verbatim: name)
                .font(.subheadline.weight(.medium))
                .padding(.horizontal, 10)
                .frame(height: 34)
                .background(on.wrappedValue ? Color.accentColor : Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 7))
                .foregroundStyle(on.wrappedValue ? Color.white : Color.primary)
        }
        .buttonStyle(.plain)
    }
}

/// A snippet of the vault or a team, run with a tap of the key; or any text to type.
private struct SnippetChooser: View {
    let pick: (BarKey) -> Void
    @State private var groups: [SnippetGroup]?
    @State private var text = ""
    @State private var label = ""

    var body: some View {
        if let groups {
            if groups.isEmpty {
                Section { Text("No snippets").foregroundStyle(.secondary) }
            }
            ForEach(groups) { g in
                Section(g.name ?? String(localized: "My snippets")) {
                    ForEach(g.snippets) { s in
                        Button { pick(BarKey(kind: .snippet, label: s.name, value: s.command)) } label: {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(s.name).foregroundStyle(.primary)
                                Text(s.command).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                            }
                        }
                    }
                }
            }
        } else {
            Section { ProgressView().task { groups = await SnippetGroup.load() } }
        }
        Section {
            TextField("Text to type", text: $text)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .font(.body.monospaced())
            TextField("Label on the key", text: $label)
            Button("Use") { pick(BarKey(kind: .text, label: label, value: text)) }.disabled(text.isEmpty)
        } header: {
            Text("Text")
        } footer: {
            Text("\\n in the text presses Enter")
        }
    }
}
