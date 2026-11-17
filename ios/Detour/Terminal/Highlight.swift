import Foundation

// Colours for terminal output that has none of its own, as the desktop app paints it
// (`src/lib/highlight.ts`): the prompt, addresses, paths, sizes, percentages, dates and status
// words. Colours sent by the server always win: only text in the default colour (no dim,
// inverse or background) is painted, and full screen programs (the alternate screen) are left
// alone. Colours are the theme's ANSI ones, so highlights suit any theme.

enum Highlight {
    private enum Colour: String {
        case green = "32", yellow = "33", red = "31", blue = "34", magenta = "35", cyan = "36", dim = "90"
    }

    private static func paint(_ c: Colour, _ s: String) -> String { "\u{1b}[\(c.rawValue)m\(s)\u{1b}[39m" }

    /// "word" also as "Word" and "WORD".
    private static func words(_ list: String) -> String {
        list.split(separator: " ").flatMap { w in [String(w), w.prefix(1).uppercased() + w.dropFirst(), w.uppercased()] }.joined(separator: "|")
    }

    /// Nil only if the pattern didn't compile: then output goes without highlights.
    static let rules: NSRegularExpression? = {
        let ok = words("active running ok success successful successfully succeeded completed enabled up online healthy passed done ready listen listening estab established connected started accepted")
        let warn = words("warning warn warnings deprecated degraded pending retrying skipped disabled exited activating deactivating reloading restarting")
        let err = words("error errors err failed failure fail fatal denied refused critical crit emerg panic inactive dead down unreachable invalid killed exception rejected unhealthy timeout aborted")
        let month = "Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec"
        let day = "Mon|Tue|Wed|Thu|Fri|Sat|Sun"
        let hex = "[0-9a-fA-F]{1,4}"
        // Letters and digits of any script, for word edges.
        let w = #"\p{L}\p{N}_"#
        let parts = [
            // user@host:path$ and [user@host dir]$, as bash prints them without colours.
            #"(?<![\#(w).-])(?<pu>[a-z_][\w.-]*@[\w.-]+):(?<pp>[~/][^\s$#]*)(?<ps>[$#])(?= |$)"#,
            #"\[(?<bu>[a-z_][\w.-]*@[\w.-]+) (?<bp>[^\]\s]+)\](?<bs>[$#])(?= |$)"#,
            #"(?<url>\b(?:https?|ftp|ssh|wss?)://[^\s'"<>()\x00-\x1f]*[^\s'"<>().,;:!?\x00-\x1f])"#,
            // nginx: 26/Oct/2026:13:51:33 +0000
            #"(?<date>(?<![\#(w)])\d{2}/(?:\#(month))/\d{4}:\d{2}:\d{2}:\d{2}(?: [+-]\d{4})?"#
                // 2026-10-26 13:51:33, ISO with a zone, "Mon 2026-10-26 ... UTC"
                + #"|(?<![\#(w)])(?:(?:\#(day)) )?\d{4}[-/]\d{2}[-/]\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:[.,]\d+)?)?(?:Z|[+-]\d{2}:?\d{2}| UTC| GMT)?)?(?![\#(w)])"#
                // Oct 26 13:51:33, Mon Oct 26 13:51:33 UTC 2026, Oct 26  2025
                + #"|(?<![\#(w)])(?:(?:\#(day)),? )?(?:\#(month)) {1,2}\d{1,2}(?: \d{2}:\d{2}(?::\d{2})?| {1,2}\d{4})(?: (?:UTC|GMT))?(?: \d{4})?(?![\#(w)])"#
                // 13:51:33
                + #"|(?<![\#(w):.])\d{1,2}:\d{2}:\d{2}(?:[.,]\d+)?(?![\#(w):]))"#,
            // [::]:22, *:80, localhost:8080, example.com:443, 10.0.0.1:22
            #"(?<addr>(?<![\#(w).-])(?:\[[0-9a-fA-F:.]+\]|\*|localhost|(?:[a-zA-Z0-9-]+\.)+[a-zA-Z]{2,}|(?:\d{1,3}\.){3}\d{1,3}):\d{1,5}(?![\#(w):]|\.\d)"#
                // MAC
                + #"|(?<![\#(w):])(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}(?![\#(w):])"#
                // IPv6, with :: or all eight groups
                + #"|(?<![\#(w):])(?:(?:\#(hex):){7}\#(hex)|(?:\#(hex):){1,7}:(?:\#(hex)(?::\#(hex)){0,6})?|::\#(hex)(?::\#(hex)){0,6})(?:/\d{1,3})?(?![\#(w):])"#
                // IPv4, with a prefix length
                + #"|(?<![\#(w).])(?:\d{1,3}\.){3}\d{1,3}(?:/\d{1,2})?(?![\#(w)]|\.\d))"#,
            // /var/log/x, ~/x, ./x, ../x, C:\x
            #"(?<path>(?<![\#(w).~/\\-])(?:~|\.{1,2})?(?:/[\#(w).@%+,~-]+)+/?(?<![.,])|(?<![\#(w)])[A-Za-z]:\\(?:[^\\\s:*?"<>|]+\\?)*)"#,
            // HTTP/1.1 200 OK; an access log's "GET / HTTP/1.1" 200
            #"(?<=HTTP/\d(?:\.\d)? |" )(?<http>[1-5]\d{2})(?![\#(w)])"#,
            #"(?<pct>(?<![\#(w).])\d+(?:[.,]\d+)?%)"#,
            // 412.6M, 80G, 1.5GiB, 48ms, 12 MB, 23 days
            #"(?<num>(?<![\#(w).])\d+(?:[.,]\d+)?(?:(?:[KMGTPE]i?B?|kB|B|bytes|ms|us|µs|ns|s|sec|min|h|d)|(?: (?:[KMGTPE]i?B|kB|bytes|ms|sec|secs|seconds?|mins?|minutes?|hours?|days?|weeks?)))(?![\#(w)]))"#,
            // Not "Active:", a label.
            #"(?<ok>(?<![\#(w)-])(?:\#(ok))(?![\#(w)-]|\.\w|:))"#,
            #"(?<warn>(?<![\#(w)-])(?:\#(warn))(?![\#(w)-]|\.\w))"#,
            #"(?<err>(?<![\#(w)-])(?:\#(err))(?![\#(w)-]|\.\w))"#,
        ]
        return try? NSRegularExpression(pattern: parts.joined(separator: "|"))
    }()

    private static func httpColour(_ code: String) -> Colour {
        switch code.first ?? " " {
        case "2": .green
        case "3": .cyan
        case "4": .yellow
        case "5": .red
        default: .dim
        }
    }

    private static func pctColour(_ v: Double) -> Colour { v >= 90 ? .red : v >= 70 ? .yellow : .green }

    /// `text` with highlights; it must be plain text in the default colour, without escape sequences.
    static func text(_ text: String) -> String {
        guard let rules else { return text }
        let ns = text as NSString
        var out = ""
        var last = 0
        for m in rules.matches(in: text, range: NSRange(location: 0, length: ns.length)) {
            out += ns.substring(with: NSRange(location: last, length: m.range.location - last))
            last = m.range.location + m.range.length
            let whole = ns.substring(with: m.range)
            func g(_ name: String) -> String? {
                let r = m.range(withName: name)
                return r.location == NSNotFound ? nil : ns.substring(with: r)
            }
            if let pu = g("pu"), let pp = g("pp"), let ps = g("ps") {
                out += "\(paint(.green, pu)):\(paint(.blue, pp))\(ps)"
            } else if let bu = g("bu"), let bp = g("bp"), let bs = g("bs") {
                out += "[\(paint(.green, bu)) \(paint(.blue, bp))]\(bs)"
            } else if g("url") != nil || g("addr") != nil {
                out += paint(.cyan, whole)
            } else if g("date") != nil {
                out += paint(.dim, whole)
            } else if g("path") != nil {
                out += paint(.blue, whole)
            } else if g("http") != nil {
                out += paint(httpColour(whole), whole)
            } else if g("pct") != nil {
                out += paint(pctColour(Double(whole.dropLast().replacingOccurrences(of: ",", with: ".")) ?? 0), whole)
            } else if g("num") != nil {
                out += paint(.magenta, whole)
            } else if g("ok") != nil {
                out += paint(.green, whole)
            } else if g("warn") != nil {
                out += paint(.yellow, whole)
            } else if g("err") != nil {
                out += paint(.red, whole)
            } else {
                out += whole
            }
        }
        out += ns.substring(from: last)
        return out
    }
}

/// Reads the terminal's output as it comes, keeping track of escape sequences, the current
/// colours and the alternate screen, and adds highlights to plain text.
final class OutputHighlighter {
    private enum St { case ground, esc, escInter, csi, osc, str, strEsc }

    /// Below this a write is typed echo or a prompt and goes out at once; above it a word cut at
    /// the end waits for the rest.
    private static let holdFrom = 256
    private static let holdMax = 200

    private var state = St.ground
    private var csi: [UInt16] = []
    private var fg = false, bg = false, faint = false, inverse = false, hidden = false, alt = false
    private(set) var held: [UInt16] = []

    /// `text` with highlights when `on`; either way the state follows the stream.
    func write(_ input: String, on: Bool) -> String {
        let text = held + Array(input.utf16)
        held = []
        var out = ""
        var from = 0
        func str(_ a: Int, _ b: Int) -> String { String(decoding: text[a..<b], as: UTF16.self) }
        func plainUpTo(_ end: Int) {
            if end > from { out += on && plain ? Highlight.text(str(from, end)) : str(from, end) }
            from = end
        }
        func seq(_ last: Int) {
            out += str(from, last + 1)
            from = last + 1
            state = .ground
        }
        var i = 0
        while i < text.count {
            let c = text[i]
            switch state {
            case .ground:
                if c == 0x1b {
                    plainUpTo(i)
                    state = .esc
                }
            case .esc:
                if c == 0x5b {
                    state = .csi
                    csi = []
                } else if c == 0x5d {
                    state = .osc
                } else if c == 0x50 || c == 0x58 || c == 0x5e || c == 0x5f {
                    state = .str
                } else if c >= 0x20 && c <= 0x2f {
                    state = .escInter
                } else {
                    if c == 0x63 { reset() }
                    seq(i)
                }
            case .escInter:
                if c >= 0x30 && c <= 0x7e { seq(i) }
            case .csi:
                if c >= 0x40 && c <= 0x7e {
                    onCsi(String(decoding: csi, as: UTF16.self), c)
                    seq(i)
                } else if c == 0x18 || c == 0x1a {
                    seq(i)
                } else {
                    csi.append(c)
                }
            case .osc, .str:
                if c == 0x07 && state == .osc { seq(i) } else if c == 0x1b { state = .strEsc }
            case .strEsc:
                // ESC \ ends the string; anything else is cut short by a new sequence.
                if c == 0x5c {
                    seq(i)
                } else {
                    out += str(from, i - 1)
                    from = i - 1
                    state = .esc
                    i -= 1
                }
            }
            i += 1
        }
        if state != .ground {
            // A sequence cut in two: what came of it so far goes out as it is.
            out += str(from, text.count)
            return out
        }
        var end = text.count
        // A long write cut in the middle of a word: the word waits for the next write, or flush().
        if on && plain && text.count >= Self.holdFrom {
            var start = end
            let limit = max(from, end - Self.holdMax)
            while start > limit, !Self.isSpaceOrControl(text[start - 1]) { start -= 1 }
            if start < end, start > limit {
                held = Array(text[start..<end])
                end = start
            }
        }
        plainUpTo(end)
        return out
    }

    /// What waits, now.
    func flush(on: Bool) -> String {
        let rest = String(decoding: held, as: UTF16.self)
        held = []
        return !rest.isEmpty && on && plain ? Highlight.text(rest) : rest
    }

    private static func isSpaceOrControl(_ c: UInt16) -> Bool {
        c <= 0x20 || c == 0x7f || c == 0xa0 || (c >= 0x2000 && c <= 0x200a) || c == 0x2028 || c == 0x2029 || c == 0x3000
    }

    private var plain: Bool { !alt && !fg && !bg && !faint && !inverse && !hidden }

    private func reset() {
        fg = false; bg = false; faint = false; inverse = false; hidden = false; alt = false
    }

    private func onCsi(_ params: String, _ final: UInt16) {
        if final == 0x6d { return onSgr(params) }
        if (final == 0x68 || final == 0x6c) && params.hasPrefix("?") {
            for p in params.dropFirst().split(separator: ";") where p == "47" || p == "1047" || p == "1049" {
                alt = final == 0x68
            }
        }
    }

    private func onSgr(_ params: String) {
        let ps = params.isEmpty ? ["0"] : params.split(separator: ";", omittingEmptySubsequences: false).map(String.init)
        var i = 0
        while i < ps.count {
            let head = ps[i].split(separator: ":", omittingEmptySubsequences: false).first.map(String.init) ?? ""
            let sub = ps[i].contains(":")
            let n = head.isEmpty ? 0 : Int(head) ?? -1
            switch n {
            case 0: fg = false; bg = false; faint = false; inverse = false; hidden = false
            case 2: faint = true
            case 22: faint = false
            case 7: inverse = true
            case 27: inverse = false
            case 8: hidden = true
            case 28: hidden = false
            case 30...37, 90...97: fg = true
            case 39: fg = false
            case 40...47, 100...107: bg = true
            case 49: bg = false
            default: break
            }
            if n == 38 || n == 48 || n == 58 {
                if n == 38 { fg = true }
                if n == 48 { bg = true }
                // 38;5;n and 38;2;r;g;b carry their colour in the next parameters.
                if !sub, i + 1 < ps.count { i += ps[i + 1] == "5" ? 2 : ps[i + 1] == "2" ? 4 : 0 }
            }
            i += 1
        }
    }
}

/// A session's output on its way to the terminal: bytes decoded as UTF-8 across writes, with
/// highlights while they are on. Highlights never cost output: if anything is off, the text
/// goes as it came.
@MainActor
final class HighlightedOutput {
    /// Above this many characters a second (a big `cat`), output goes out as it is.
    private static let flood = 1_000_000
    /// How long a word cut at the end of a write waits for the rest.
    private static let holdFor = 0.015

    private let deliver: ([UInt8]) -> Void
    private let enabled: () -> Bool
    private var hl = OutputHighlighter()
    private var partial: [UInt8] = []
    private var windowStart = Date.distantPast
    private var windowChars = 0
    private var holdToken = 0

    init(deliver: @escaping ([UInt8]) -> Void, enabled: @escaping () -> Bool) {
        self.deliver = deliver
        self.enabled = enabled
    }

    func write(_ data: [UInt8]) {
        let bytes = partial + data
        partial = []
        // A UTF-8 character cut at the end waits for its other bytes.
        var cut = bytes.count
        var back = 0
        while back < 3, cut - back - 1 >= 0, bytes[cut - back - 1] & 0xC0 == 0x80 { back += 1 }
        if cut - back - 1 >= 0 {
            let lead = bytes[cut - back - 1]
            let need = lead >= 0xF0 ? 4 : lead >= 0xE0 ? 3 : lead >= 0xC0 ? 2 : 1
            if need > back + 1 { cut = cut - back - 1 }
        }
        partial = Array(bytes[cut...])
        let text = String(decoding: bytes[..<cut], as: UTF8.self)

        let now = Date()
        if now.timeIntervalSince(windowStart) > 1 {
            windowStart = now
            windowChars = 0
        }
        windowChars += text.utf16.count
        let on = enabled() && windowChars < Self.flood
        let out = hl.write(text, on: on)
        if !out.isEmpty { deliver(Array(out.utf8)) }
        if !hl.held.isEmpty {
            holdToken += 1
            let token = holdToken
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.holdFor) { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, self.holdToken == token else { return }
                    self.flush()
                }
            }
        }
    }

    /// Before anything else is written to the terminal.
    func flush() {
        holdToken += 1
        let rest = hl.flush(on: enabled())
        if !rest.isEmpty { deliver(Array(rest.utf8)) }
    }
}
