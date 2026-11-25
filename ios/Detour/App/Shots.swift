#if DEBUG
import SwiftTerm
import UIKit

/// App Store screenshots (debug builds only): launched with `-shot <screen>`, the app fills a
/// vault of its own with sample hosts and opens that screen. `scripts/screenshots.sh` runs it
/// in the simulator with a key for the Mac it runs on (`DETOUR_SHOTS_KEY`, `DETOUR_SHOTS_USER`),
/// so the terminal and the files are real.
enum Shots {
    static let screen: String? = {
        let a = ProcessInfo.processInfo.arguments
        guard let i = a.firstIndex(of: "-shot"), i + 1 < a.count else { return nil }
        return a[i + 1]
    }()

    /// The host the terminal and the files open: the Mac the simulator runs on.
    static let localLabel = "build-mac"
    private static let password = "screenshots only"
    private static var env: [String: String] { ProcessInfo.processInfo.environment }

    /// The sample vault, made on the first launch and unlocked on the next ones.
    static func prepare() async {
        guard screen != nil else { return }
        let st = try? await Core.shared.call("vault_status", as: VaultStatus.self)
        if st?.initialized != true {
            _ = try? await Core.shared.call("vault_create", ["masterPassword": password])
            await seed()
        } else if st?.unlocked != true {
            _ = try? await Core.shared.call("vault_unlock", ["masterPassword": password])
            // Every drag case starts from the same groups.
            if screen == "drag" { await seed() }
        }
        log("== case \(dragCase)")
    }

    private static func seed() async {
        if let pem = env["DETOUR_SHOTS_KEY"] {
            _ = try? await Core.shared.call("key_import", ["vaultId": "personal", "name": "build-mac", "text": pem, "remember": true])
        }
        _ = try? await Core.shared.call("key_generate", ["vaultId": "personal", "name": "deploy", "algorithm": "ed25519", "remember": true])
        let keys = ((try? await PersonalVault.get())?["keys"] as? [PersonalVault.JSON]) ?? []
        let localKey = keys.first { $0["name"] as? String == "build-mac" }?["id"] as? String
        func host(_ label: String, _ group: String, _ address: String, _ user: String, key: String? = nil) -> PersonalVault.JSON {
            ["id": PersonalVault.newId(), "label": label, "group": group, "address": address, "port": 22, "user": user, "tags": [String](),
             "auth": key.map { ["type": "key", "keyId": $0] } ?? ["type": "password"]]
        }
        let hosts = [
            host("web-1", "Production", "10.0.1.11", "deploy"),
            host("web-2", "Production", "10.0.1.12", "deploy"),
            host("db-primary", "Production", "10.0.2.5", "postgres"),
            host("staging", "Staging", "staging.example.com", "deploy"),
            host("ci-runner", "Staging", "10.0.9.3", "runner"),
            host(localLabel, "Lab", "127.0.0.1", env["DETOUR_SHOTS_USER"] ?? "deploy", key: localKey),
        ]
        let snippets: [PersonalVault.JSON] = [
            ["id": PersonalVault.newId(), "name": "Disk usage", "command": "df -h"],
            ["id": PersonalVault.newId(), "name": "Top processes", "command": "ps aux | sort -rk 3 | head"],
            ["id": PersonalVault.newId(), "name": "Follow nginx logs", "command": "journalctl -fu nginx"],
            ["id": PersonalVault.newId(), "name": "Restart nginx", "command": "sudo systemctl restart nginx"],
        ]
        try? await PersonalVault.edit { v in
            v["hosts"] = hosts
            v["snippets"] = snippets
            if screen == "drag" {
                // The drag test (UITests/DragTests.swift): empty groups between the others.
                v["groups"] = ["Empty A", "Production", "Empty B", "Staging", "Empty C", "Lab"].enumerated().map { i, name in
                    ["id": PersonalVault.newId(), "name": name, "order": Double(i + 1)] as PersonalVault.JSON
                }
            }
        }
    }

    /// The drag test's case (`-dragcase`), for its record.
    static let dragCase: String = {
        let a = ProcessInfo.processInfo.arguments
        guard let i = a.firstIndex(of: "-dragcase"), i + 1 < a.count else { return "" }
        return a[i + 1]
    }()

    /// A line in Documents/drag.txt (the drag test reads it back).
    static func log(_ line: String) {
        guard screen == "drag", let dir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first else { return }
        let url = dir.appendingPathComponent("drag.txt")
        let old = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
        try? (old + line + "\n").write(to: url, atomically: true, encoding: .utf8)
    }

    /// What the terminal shows: a few everyday commands on the Mac (scripts/screenshots.sh puts
    /// the log there), short enough for a phone's width.
    static let terminalCommands = "clear\ndf -h / | awk '{print $2, $4, $5}'\ntail -n 7 logs/app.log"

    /// The terminal's whole buffer with its size and cursor, added to Documents/<screen>.txt: the
    /// text itself, which a screenshot can't tell from its drawing.
    @MainActor static func dump(_ session: TerminalSession?, _ label: String) {
        guard let view = session?.view, let screen,
              let dir = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first else { return }
        let s = view.terminalStateSnapshot()
        var text = "== \(label): \(s.dimensions.cols)x\(s.dimensions.rows) cursor \(s.cursor.col),\(s.cursor.row) viewport \(s.viewportRow) bounds \(view.bounds.height) offset \(view.contentOffset.y)\n"
        text += String(decoding: view.getBufferAsData(), as: UTF8.self)
        let url = dir.appendingPathComponent("\(screen).txt")
        let old = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
        try? (old + text).write(to: url, atomically: true, encoding: .utf8)
    }
}
#endif
