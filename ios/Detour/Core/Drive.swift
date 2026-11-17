import AuthenticationServices
import Foundation

/// A cloud drive a vault or a team can be kept in.
enum DriveProvider: String, CaseIterable, Identifiable, Codable {
    case onedrive, gdrive, dropbox

    var id: String { rawValue }

    var name: String {
        switch self {
        case .onedrive: "OneDrive"
        case .gdrive: "Google Drive"
        case .dropbox: "Dropbox"
        }
    }
}

/// A folder in a drive, as the core names it (`detour_core::drive::DriveRemote`).
struct DriveFolderRef: Codable, Hashable {
    let provider: DriveProvider
    let id: String
    var drive: String?
    /// Its path or name, to show.
    var label: String
    var link: String?

    /// As the core takes it.
    var json: [String: Any] {
        var o: [String: Any] = ["provider": provider.rawValue, "id": id, "label": label]
        if let drive { o["drive"] = drive }
        if let link { o["link"] = link }
        return o
    }

    /// What the person sees: "OneDrive: Detour/Team".
    var title: String { label.isEmpty ? provider.name : label }
}

/// A folder's subfolders.
struct DriveListing: Decodable {
    let folder: DriveFolderRef
    let folders: [Entry]

    struct Entry: Decodable, Identifiable, Hashable {
        let id: String
        let drive: String?
        let name: String
        let shared: Bool

        var key: String { "\(drive ?? "")/\(id)" }
    }
}

/// This device's sign-in to a drive.
struct DriveAccount: Decodable {
    let provider: DriveProvider
    let account: String
    let signedIn: Bool
}

/// A drive's refusal, as the core reports it (`driveSignIn`, `driveNoAccess`, `drive`).
struct CoreErrorInfo: Decodable, Hashable {
    let kind: String
    let provider: String?
    let message: String?

    var drive: DriveProvider? { provider.flatMap(DriveProvider.init(rawValue:)) }
}

/// What starting a sign-in gave: a page to open, and whether it ends with a code to paste.
struct DriveStart {
    let url: URL
    let paste: Bool
}

enum Drives {
    /// The drives this device is signed in to, with their accounts.
    static func accounts() async -> [DriveProvider: String] {
        let list = (try? await Core.shared.call("drive_accounts", as: [DriveAccount].self)) ?? []
        return Dictionary(list.filter(\.signedIn).map { ($0.provider, $0.account) }, uniquingKeysWith: { a, _ in a })
    }

    /// Starts signing in: the page sends the browser back to the app once the drive has answered.
    static func start(_ p: DriveProvider) async throws -> DriveStart {
        let r = try await Core.shared.call("drive_sign_in", ["provider": p.rawValue, "back": "detour://drive-signed-in"]) as? [String: Any] ?? [:]
        guard let s = r["url"] as? String, let url = URL(string: s) else { throw CoreFailure(json: #"{"kind":"internal"}"#) }
        return DriveStart(url: url, paste: r["paste"] as? Bool ?? false)
    }

    /// Waits for the core to finish the sign-in it got back from the drive; the account.
    static func finish(_ p: DriveProvider) async throws -> String? {
        for _ in 0..<120 {
            let r = try await Core.shared.call("drive_sign_in_poll", ["provider": p.rawValue]) as? [String: Any] ?? [:]
            switch r["status"] as? String {
            case "done": return r["account"] as? String
            case "expired", "denied": return nil
            default: try await Task.sleep(for: .milliseconds(500))
            }
        }
        return nil
    }

    /// Finishes a sign-in with the code the drive's page showed.
    static func finish(_ p: DriveProvider, code: String) async throws -> String? {
        try await Core.shared.call("drive_sign_in_code", ["provider": p.rawValue, "code": code]) as? String
    }

    /// What went wrong, in words.
    static func errorText(_ e: Error) -> String {
        guard let f = e as? CoreFailure else { return e.localizedDescription }
        let name = f["provider"].flatMap(DriveProvider.init(rawValue:))?.name ?? ""
        switch f.kind {
        case "driveSignIn": return String(localized: "Sign in to \(name) again")
        case "driveNoAccess": return String(localized: "The \(name) account has no access to the folder")
        case "folderNotEmpty": return String(localized: "This folder is not empty")
        case "notATeamVault": return String(localized: "This folder has a different team")
        case "notASyncVault": return String(localized: "This folder has no synced vault")
        case "unreachable": return String(localized: "No connection")
        default: return f["message"] ?? f.kind
        }
    }
}

extension ASWebAuthenticationSessionError {
    static func isCancel(_ e: Error) -> Bool {
        (e as? ASWebAuthenticationSessionError)?.code == .canceledLogin
    }
}
