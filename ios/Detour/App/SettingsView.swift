import SwiftUI

/// Settings: the app's and the terminal's look, the keyboard, sync, Face ID and locking.
struct SettingsView: View {
    @EnvironmentObject var app: AppModel
    @ObservedObject private var terminal = TerminalSettings.shared
    @ObservedObject private var appearance = AppAppearance.shared
    @State private var biometry = Keychain.isSaved
    @State private var askPassword = false
    @State private var password = ""
    @State private var error: String?
    @AppStorage("haptics") private var haptics = true

    var body: some View {
        NavigationStack {
            Form {
                Section("Appearance") {
                    NavigationLink {
                        ThemePicker(selection: $appearance.themeId, title: "App theme", system: String(localized: "As the system"))
                    } label: {
                        LabeledContent("App theme", value: themeName(appearance.themeId, system: String(localized: "As the system")))
                    }
                    Toggle("Haptic feedback", isOn: $haptics)
                }
                Section("Terminal") {
                    NavigationLink {
                        ThemePicker(selection: $terminal.themeId, title: "Terminal theme", system: String(localized: "As the app"))
                    } label: {
                        LabeledContent("Theme", value: themeName(terminal.themeId, system: String(localized: "As the app")))
                    }
                    Stepper(value: $terminal.fontSize, in: 9...24, step: 1) {
                        LabeledContent("Font size", value: "\(Int(terminal.fontSize))")
                    }
                    Toggle("Highlight output", isOn: $terminal.highlight)
                    NavigationLink { KeyboardEditor() } label: { Label("Keyboard", systemImage: "keyboard") }
                }
                Section {
                    NavigationLink { SyncView() } label: {
                        LabeledContent {
                            Text(app.sync?.enabled == true ? String(localized: "On") : String(localized: "Off"))
                        } label: {
                            Label("Sync", systemImage: "arrow.triangle.2.circlepath")
                        }
                    }
                }
                Section {
                    if Keychain.biometryAvailable {
                        Toggle("Unlock with \(Keychain.biometryName)", isOn: Binding(get: { biometry }, set: { on in
                            if on { askPassword = true } else { Keychain.delete(); biometry = false }
                        }))
                        if let error { Text(error).foregroundStyle(.red).font(.footnote) }
                    }
                    Button { Task { await app.lock() } } label: { Label("Lock", systemImage: "lock") }
                }
                Section {
                    LabeledContent("Version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "")
                }
            }
            .navigationTitle("Settings")
            .alert("Master password", isPresented: $askPassword) {
                SecureField("Master password", text: $password)
                Button("Cancel", role: .cancel) { password = "" }
                Button("Turn on") { Task { await turnOn() } }
            }
        }
    }

    private func themeName(_ id: String, system: String) -> String {
        id == TerminalSettings.system ? system : TerminalTheme.named(id)?.name ?? system
    }

    private func turnOn() async {
        let pw = password
        password = ""
        do {
            // Checked by the core before it is kept.
            _ = try await Core.shared.call("vault_unlock", ["masterPassword": pw])
            biometry = Keychain.save(pw)
            error = biometry ? nil : String(localized: "Couldn't save it on this device")
        } catch {
            self.error = String(localized: "Wrong master password")
        }
    }
}

/// The themes, each shown in its own colours; the first row follows the system (or the app).
struct ThemePicker: View {
    @Binding var selection: String
    let title: LocalizedStringKey
    let system: String

    var body: some View {
        List {
            row(id: TerminalSettings.system, name: system, theme: nil)
            ForEach(TerminalTheme.all) { t in row(id: t.id, name: t.name, theme: t) }
        }
        .navigationTitle(title)
    }

    private func row(id: String, name: String, theme: TerminalTheme?) -> some View {
        Button {
            if selection != id { Haptics.select() }
            withAnimation(.spring(response: 0.3, dampingFraction: 0.6)) { selection = id }
        } label: {
            HStack(spacing: 12) {
                swatch(theme)
                Text(name).foregroundStyle(.primary)
                Spacer()
                // The check pops in on the theme picked: seen even past the finger that picked it.
                if selection == id {
                    Image(systemName: "checkmark").foregroundStyle(Color.accentColor)
                        .transition(.scale.combined(with: .opacity))
                }
            }
        }
    }

    /// The background with a prompt in the theme's own colours.
    @ViewBuilder private func swatch(_ theme: TerminalTheme?) -> some View {
        if let theme {
            HStack(spacing: 0) {
                Text(verbatim: "$ ").foregroundStyle(Color(uiColor: theme.foreground))
                Text(verbatim: "~").foregroundStyle(Color(uiColor: UIColor(hex: theme.c[8])))
                Text(verbatim: " ok").foregroundStyle(Color(uiColor: UIColor(hex: theme.c[6])))
            }
            .font(.system(size: 13, design: .monospaced))
            .frame(width: 64, height: 32)
            .background(Color(uiColor: theme.background), in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.3)))
        } else {
            HStack(spacing: 0) {
                Color(uiColor: TerminalTheme.all[0].background)
                Color(uiColor: TerminalTheme.all[1].background)
            }
            .frame(width: 64, height: 32)
            .clipShape(RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.3)))
        }
    }
}
