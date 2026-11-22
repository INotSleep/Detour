import SwiftTerm
import SwiftUI
import UIKit

/// A terminal colour theme, the desktop's (`src/lib/themes.ts`): background, foreground,
/// cursor, selection, then the 16 ANSI colours (normal, bright). Names are the themes' own.
struct TerminalTheme: Identifiable, Equatable {
    let id: String
    let name: String
    let c: [String]

    var background: UIColor { UIColor(hex: c[0]) }
    var foreground: UIColor { UIColor(hex: c[1]) }
    var cursor: UIColor { UIColor(hex: c[2]) }
    var selection: UIColor { UIColor(hex: c[3], alpha: c[3].count > 7 ? nil : 0.4) }
    var ansi: [SwiftTerm.Color] { c[4...].map { SwiftTerm.Color(hex: $0) } }
    var isLight: Bool { UIColor(hex: c[0]).luminance > 0.45 }

    private static func t(_ id: String, _ name: String, _ base: String, _ normal: String, _ bright: String) -> TerminalTheme {
        TerminalTheme(id: id, name: name, c: (base + " " + normal + " " + bright).split(separator: " ").map(String.init))
    }

    static let all: [TerminalTheme] = [
        t("detour-dark", "Detour Dark", "#0b0c0e #d9dbe1 #8d9cff #7a8cff52", "#1b1d22 #ef6461 #4cc38a #e5a54b #7a8cff #c58af9 #56c7d8 #c9ccd5", "#5d616c #ff8582 #6fd8a4 #f2c178 #9eabff #d8acff #7fdcea #f2f3f6"),
        t("detour-light", "Detour Light", "#fbfbfc #17181c #4d5ee8 #4d5ee833", "#17181c #d6453f #2f9e66 #b7791f #4d5ee8 #9a4fd0 #1d8fa3 #8a8d96", "#4f525a #e5534b #38b174 #c98a2a #6573f0 #ad66de #2aa5ba #c9cbd1"),
        t("one-dark", "One Dark", "#282c34 #abb2bf #528bff #3e4451", "#282c34 #e06c75 #98c379 #e5c07b #61afef #c678dd #56b6c2 #abb2bf", "#5c6370 #e06c75 #98c379 #e5c07b #61afef #c678dd #56b6c2 #ffffff"),
        t("dracula", "Dracula", "#282a36 #f8f8f2 #f8f8f2 #44475a", "#21222c #ff5555 #50fa7b #f1fa8c #bd93f9 #ff79c6 #8be9fd #f8f8f2", "#6272a4 #ff6e6e #69ff94 #ffffa5 #d6acff #ff92df #a4ffff #ffffff"),
        t("nord", "Nord", "#2e3440 #d8dee9 #d8dee9 #434c5e", "#3b4252 #bf616a #a3be8c #ebcb8b #81a1c1 #b48ead #88c0d0 #e5e9f0", "#4c566a #bf616a #a3be8c #ebcb8b #81a1c1 #b48ead #8fbcbb #eceff4"),
        t("tokyo-night", "Tokyo Night", "#1a1b26 #c0caf5 #c0caf5 #33467c", "#15161e #f7768e #9ece6a #e0af68 #7aa2f7 #bb9af7 #7dcfff #a9b1d6", "#414868 #f7768e #9ece6a #e0af68 #7aa2f7 #bb9af7 #7dcfff #c0caf5"),
        t("catppuccin-mocha", "Catppuccin Mocha", "#1e1e2e #cdd6f4 #f5e0dc #45475a", "#45475a #f38ba8 #a6e3a1 #f9e2af #89b4fa #f5c2e7 #94e2d5 #bac2de", "#585b70 #f38ba8 #a6e3a1 #f9e2af #89b4fa #f5c2e7 #94e2d5 #a6adc8"),
        t("gruvbox-dark", "Gruvbox Dark", "#282828 #ebdbb2 #ebdbb2 #504945", "#282828 #cc241d #98971a #d79921 #458588 #b16286 #689d6a #a89984", "#928374 #fb4934 #b8bb26 #fabd2f #83a598 #d3869b #8ec07c #ebdbb2"),
        t("monokai", "Monokai", "#272822 #f8f8f2 #f8f8f0 #49483e", "#272822 #f92672 #a6e22e #f4bf75 #66d9ef #ae81ff #a1efe4 #f8f8f2", "#75715e #f92672 #a6e22e #f4bf75 #66d9ef #ae81ff #a1efe4 #f9f8f5"),
        t("tomorrow-night", "Tomorrow Night", "#1d1f21 #c5c8c6 #c5c8c6 #373b41", "#1d1f21 #cc6666 #b5bd68 #f0c674 #81a2be #b294bb #8abeb7 #c5c8c6", "#969896 #cc6666 #b5bd68 #f0c674 #81a2be #b294bb #8abeb7 #ffffff"),
        t("solarized-dark", "Solarized Dark", "#002b36 #839496 #93a1a1 #073642", "#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #eee8d5", "#586e75 #cb4b16 #586e75 #657b83 #839496 #6c71c4 #93a1a1 #fdf6e3"),
        t("solarized-light", "Solarized Light", "#fdf6e3 #657b83 #586e75 #eee8d5", "#073642 #dc322f #859900 #b58900 #268bd2 #d33682 #2aa198 #eee8d5", "#002b36 #cb4b16 #586e75 #657b83 #839496 #6c71c4 #93a1a1 #fdf6e3"),
        t("one-light", "One Light", "#fafafa #383a42 #526fff #e5e5e6", "#383a42 #e45649 #50a14f #c18401 #4078f2 #a626a4 #0184bc #a0a1a7", "#696c77 #e45649 #50a14f #c18401 #4078f2 #a626a4 #0184bc #fafafa"),
        t("github-light", "GitHub Light", "#ffffff #24292f #0969da #b6d7ff", "#24292f #cf222e #116329 #4d2d00 #0969da #8250df #1b7c83 #6e7781", "#57606a #a40e26 #1a7f37 #633c01 #218bff #a475f9 #3192aa #8c959f"),
    ]

    static func named(_ id: String) -> TerminalTheme? { all.first { $0.id == id } }
}

/// The terminal's look and behaviour on this phone: theme, font size, output highlighting.
@MainActor
final class TerminalSettings: ObservableObject {
    static let shared = TerminalSettings()
    /// "system" follows the phone's dark or light appearance.
    static let system = "system"

    @Published var themeId: String { didSet { UserDefaults.standard.set(themeId, forKey: "terminalTheme") } }
    @Published var fontSize: Double { didSet { UserDefaults.standard.set(fontSize, forKey: "terminalFontSize") } }
    @Published var highlight: Bool { didSet { UserDefaults.standard.set(highlight, forKey: "terminalHighlight") } }

    private init() {
        let d = UserDefaults.standard
        themeId = d.string(forKey: "terminalTheme") ?? Self.system
        fontSize = d.object(forKey: "terminalFontSize") as? Double ?? 13
        highlight = d.object(forKey: "terminalHighlight") as? Bool ?? true
    }

    /// The terminal's own theme, else the app's, else Detour Dark or Light as the phone is.
    func theme(dark: Bool) -> TerminalTheme {
        if let t = TerminalTheme.named(themeId) { return t }
        if let t = AppAppearance.shared.theme { return t }
        return TerminalTheme.all[dark ? 0 : 1]
    }
}

/// The app's look, as on the desktop: the phone's dark or light, or a theme. A theme sets dark
/// or light, the accent colour, and the terminal's colours while the terminal follows the app.
@MainActor
final class AppAppearance: ObservableObject {
    static let shared = AppAppearance()

    @Published var themeId: String { didSet { UserDefaults.standard.set(themeId, forKey: "appTheme") } }

    private init() {
        themeId = UserDefaults.standard.string(forKey: "appTheme") ?? TerminalSettings.system
    }

    var theme: TerminalTheme? { TerminalTheme.named(themeId) }
    var scheme: ColorScheme? { theme.map { $0.isLight ? .light : .dark } }
    /// The theme's blue.
    var accent: SwiftUI.Color? { theme.map { SwiftUI.Color(uiColor: UIColor(hex: $0.c[8])) } }
}

extension UIColor {
    convenience init(hex: String, alpha: CGFloat? = nil) {
        let h = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        let v = UInt64(h.prefix(6), radix: 16) ?? 0
        let a = alpha ?? (h.count >= 8 ? CGFloat(UInt64(h.dropFirst(6).prefix(2), radix: 16) ?? 255) / 255 : 1)
        self.init(red: CGFloat((v >> 16) & 255) / 255, green: CGFloat((v >> 8) & 255) / 255, blue: CGFloat(v & 255) / 255, alpha: a)
    }

    var luminance: Double {
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        getRed(&r, green: &g, blue: &b, alpha: &a)
        func lin(_ s: CGFloat) -> Double { let s = Double(s); return s <= 0.03928 ? s / 12.92 : pow((s + 0.055) / 1.055, 2.4) }
        return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }
}

extension SwiftTerm.Color {
    convenience init(hex: String) {
        let v = UInt64(hex.trimmingCharacters(in: CharacterSet(charactersIn: "#")).prefix(6), radix: 16) ?? 0
        // SwiftTerm's colours are 16 bits a channel.
        self.init(red: UInt16((v >> 16) & 255) * 257, green: UInt16((v >> 8) & 255) * 257, blue: UInt16(v & 255) * 257)
    }
}

extension TerminalView {
    func apply(_ theme: TerminalTheme, fontSize: Double) {
        installColors(theme.ansi)
        nativeForegroundColor = theme.foreground
        nativeBackgroundColor = theme.background
        // SwiftTerm clears old glyphs to transparent and lets the layer's colour show through:
        // that needs a view that isn't opaque, or the cleared places come out black.
        layer.backgroundColor = theme.background.cgColor
        isOpaque = false
        caretColor = theme.cursor
        selectedTextBackgroundColor = theme.selection
        keyboardAppearance = theme.isLight ? .light : .dark
        let size = CGFloat(fontSize)
        if font.pointSize != size { font = UIFont.monospacedSystemFont(ofSize: size, weight: .regular) }
    }
}
