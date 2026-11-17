// Checks the output highlighter on macOS (CI), built with Detour/Terminal/Highlight.swift:
// the pattern compiles under ICU and paints what the desktop app paints.
import Foundation

var failed = 0
func expect(_ input: String, _ wanted: String, _ what: String) {
    let got = Highlight.text(input)
    if got != wanted {
        failed += 1
        print("FAIL \(what)\n  in:   \(input.debugDescription)\n  got:  \(got.debugDescription)\n  want: \(wanted.debugDescription)")
    }
}
let e = "\u{1b}["
guard Highlight.rules != nil else { print("FAIL the pattern doesn't compile"); exit(1) }

expect("root@web:~# ", "\(e)32mroot@web\(e)39m:\(e)34m~\(e)39m# ", "prompt")
expect("[deploy@api-1 app]$ ", "[\(e)32mdeploy@api-1\(e)39m \(e)34mapp\(e)39m]$ ", "bracket prompt")
expect("see /var/log/nginx/error.log now", "see \(e)34m/var/log/nginx/error.log\(e)39m now", "path")
expect("listening on 10.0.0.1:22", "\(e)32mlistening\(e)39m on \(e)36m10.0.0.1:22\(e)39m", "address")
expect("disk 93%", "disk \(e)31m93%\(e)39m", "percent")
expect("size 412.6M", "size \(e)35m412.6M\(e)39m", "size")
expect("Active: failed", "Active: \(e)31mfailed\(e)39m", "status")
expect("at 13:51:33 ok", "at \(e)90m13:51:33\(e)39m \(e)32mok\(e)39m", "time and ok")
expect("https://example.com/a.", "\(e)36mhttps://example.com/a\(e)39m.", "url")
expect("HTTP/1.1 404 Not Found", "HTTP/1.1 \(e)33m404\(e)39m Not Found", "http status")
expect("plain words only", "plain words only", "nothing")

// The stream: colours from the server win, the alternate screen is left alone.
let h = OutputHighlighter()
let coloured = "\(e)31merror\(e)0m"
if h.write(coloured, on: true) != coloured { failed += 1; print("FAIL server colours were changed") }
_ = h.write("\(e)?1049h", on: true)
if h.write("error", on: true) != "error" { failed += 1; print("FAIL the alternate screen was painted") }
_ = h.write("\(e)?1049l", on: true)
if h.write("error", on: true) != "\(e)31merror\(e)39m" { failed += 1; print("FAIL plain text after the alternate screen") }

if failed > 0 { print("\(failed) failed"); exit(1) }
print("highlight: all good")
