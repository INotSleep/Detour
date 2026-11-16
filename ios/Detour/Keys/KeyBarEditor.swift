import SwiftUI

/// Settings of the key bar: its sets, their keys and order, own keys, or no bar at all.
struct KeyBarEditor: View {
    @ObservedObject var store = KeyBarStore.shared
    @State private var adding: KeySet.ID?

    var body: some View {
        List {
            Section {
                Toggle("Show the key bar", isOn: Binding(get: { !store.hidden }, set: { store.hidden = !$0 }))
            }
            if !store.hidden {
                ForEach($store.sets) { $set in
                    Section {
                        TextField("Set name", text: $set.name)
                        ForEach(set.keys) { key in
                            Text(key.title).font(.body.monospaced())
                        }
                        .onDelete { set.keys.remove(atOffsets: $0) }
                        .onMove { set.keys.move(fromOffsets: $0, toOffset: $1) }
                        Button {
                            adding = set.id
                        } label: {
                            Label("Add key", systemImage: "plus")
                        }
                    } header: {
                        HStack {
                            Text(set.name)
                            Spacer()
                            if store.sets.count > 1 {
                                Button(role: .destructive) {
                                    store.sets.removeAll { $0.id == set.id }
                                } label: {
                                    Image(systemName: "trash")
                                }
                            }
                        }
                    }
                }
                Section {
                    Button {
                        store.sets.append(KeySet(name: String(localized: "Set \(store.sets.count + 1)"), keys: [.init(kind: .esc)]))
                    } label: {
                        Label("Add set", systemImage: "plus.rectangle.on.rectangle")
                    }
                    Button("Reset to default", role: .destructive) { store.reset() }
                }
            }
        }
        .navigationTitle("Key bar")
        .toolbar { EditButton() }
        .sheet(item: Binding(get: { adding.map { AddingTo(id: $0) } }, set: { adding = $0?.id })) { target in
            AddKeyView { key in
                if let i = store.sets.firstIndex(where: { $0.id == target.id }) { store.sets[i].keys.append(key) }
                adding = nil
            }
        }
    }

    private struct AddingTo: Identifiable { let id: KeySet.ID }
}

/// Picks a key: one of the usual ones, a snippet, or a combination.
private struct AddKeyView: View {
    let done: (BarKey) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var label = ""
    @State private var combo = ""

    private let usual: [BarKey.Kind] = [.esc, .tab, .ctrl, .alt, .up, .down, .left, .right, .home, .end, .pageUp, .pageDown, .enter, .backspace]

    var body: some View {
        NavigationStack {
            Form {
                Section("Keys") {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 64))], spacing: 8) {
                        ForEach(usual, id: \.self) { kind in
                            Button(BarKey(kind: kind).title) { done(BarKey(kind: kind)) }
                                .buttonStyle(.bordered)
                        }
                    }
                }
                Section {
                    TextField("Text to type", text: $text)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.body.monospaced())
                    TextField("Button label (optional)", text: $label)
                    Button("Add") { done(BarKey(kind: .text, label: label, value: text)) }
                        .disabled(text.isEmpty)
                } header: {
                    Text("Snippet")
                } footer: {
                    Text("\\n in the text presses Enter")
                }
                Section {
                    TextField("ctrl+c", text: $combo)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.body.monospaced())
                    Button("Add") { done(BarKey(kind: .combo, value: combo)) }
                        .disabled(BarKey.combo(combo) == nil)
                } header: {
                    Text("Combination")
                }
            }
            .navigationTitle("Add key")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
        }
    }
}
