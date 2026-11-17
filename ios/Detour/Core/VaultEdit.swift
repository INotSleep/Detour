import Foundation

/// Changes to the personal vault, the way the desktop app makes them: the vault as the core
/// last showed it, changed, and put back with what it was (`base`), so only these changes apply.
enum PersonalVault {
    typealias JSON = [String: Any]

    static func get() async throws -> JSON {
        try await Core.shared.call("vault_get") as? JSON ?? [:]
    }

    /// Reads the vault, lets `change` edit it, and saves the result.
    static func edit(_ change: (inout JSON) throws -> Void) async throws {
        let base = try await get()
        var next = base
        try change(&next)
        _ = try await Core.shared.call("vault_put", ["update": pick(next), "base": pick(base)])
    }

    private static func pick(_ d: JSON) -> JSON {
        ["hosts": d["hosts"] ?? [], "keys": d["keys"] ?? [], "snippets": d["snippets"] ?? [], "tunnels": d["tunnels"] ?? []]
    }

    static func newId() -> String { UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased() }

    // MARK: hosts

    /// A host of the personal vault as JSON, for its editor.
    static func host(_ id: String) async throws -> JSON? {
        (try await get()["hosts"] as? [JSON])?.first { $0["id"] as? String == id }
    }

    /// Adds `host`, or replaces the one with its id.
    static func save(host: JSON) async throws {
        try await edit { v in
            var hosts = v["hosts"] as? [JSON] ?? []
            if let i = hosts.firstIndex(where: { $0["id"] as? String == host["id"] as? String }) { hosts[i] = host } else { hosts.append(host) }
            v["hosts"] = hosts
        }
    }

    /// Removes a host; returns it as it was, for Undo.
    @discardableResult
    static func remove(host id: String) async throws -> JSON? {
        var removed: JSON?
        try await edit { v in
            var hosts = v["hosts"] as? [JSON] ?? []
            if let i = hosts.firstIndex(where: { $0["id"] as? String == id }) { removed = hosts.remove(at: i) }
            v["hosts"] = hosts
        }
        return removed
    }

    // MARK: keys

    static func remove(key id: String) async throws {
        try await edit { v in
            v["keys"] = (v["keys"] as? [JSON] ?? []).filter { $0["id"] as? String != id }
        }
    }
}

/// A key of the personal vault as the core shows it.
struct SshKey: Decodable, Identifiable, Hashable {
    let id: String
    let name: String
    let algorithm: String
    let bits: Int?
    let fingerprint: String
    let publicKey: String
    let hasPassphrase: Bool

    var kind: String {
        switch algorithm {
        case "ed25519": "Ed25519"
        case "rsa": bits.map { "RSA \($0)" } ?? "RSA"
        case "ecdsa": "ECDSA"
        default: algorithm
        }
    }
}

struct VaultKeys: Decodable {
    let keys: [SshKey]
    let hosts: [HostAuth]

    struct HostAuth: Decodable {
        let id: String
        let label: String
        let auth: Auth
    }

    struct Auth: Decodable {
        let type: String
        let keyId: String?
    }

    func users(of key: SshKey) -> [String] {
        hosts.filter { $0.auth.type == "key" && $0.auth.keyId == key.id }.map(\.label)
    }
}
