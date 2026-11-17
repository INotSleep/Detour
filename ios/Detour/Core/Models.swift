import Foundation

/// A host as the core shows it (no secrets).
struct Host: Decodable, Identifiable, Hashable {
    let id: String
    let group: String
    let label: String
    let address: String
    let port: Int
    let user: String
    var own: OwnLogin?
    /// Set by the app: the vault it is in.
    var vaultId: String = "personal"

    struct OwnLogin: Decodable, Hashable {
        let user: String
        let active: Bool?
    }

    enum CodingKeys: String, CodingKey { case id, group, label, address, port, user, own }

    /// The login used to connect: this person's own one on a team host, if set.
    var login: String { own.flatMap { $0.active == false ? nil : $0.user } ?? user }
}

struct VaultView: Decodable {
    let hosts: [Host]
}

struct TeamEntry: Decodable {
    let id: String
    let view: TeamView?

    struct TeamView: Decodable {
        let id: String
        let name: String
        let data: VaultView
        let remote: Remote?
    }

    /// Where the team is kept, and why reaching it failed.
    struct Remote: Decodable {
        let kind: String
        let error: CoreErrorInfo?
    }
}

/// A team joined on another device of this person's and not here yet.
struct PendingTeam: Decodable, Identifiable {
    let teamId: String
    let name: String
    /// Its repository: joined here by itself.
    let url: String?
    /// Its drive folder: joined here by itself once signed in to the drive.
    let drive: DriveFolderRef?
    let error: CoreErrorInfo?

    var id: String { teamId }
}

struct VaultStatus: Decodable {
    let initialized: Bool
    let unlocked: Bool
    let trusted: Bool?
}

struct SyncStatus: Decodable {
    let enabled: Bool
    let location: String?
    let git: Bool
    /// `folder`, `git`, `onedrive`, `gdrive` or `dropbox`.
    let storage: String?
    let pending: Bool
    let error: CoreErrorInfo?
    let syncedAt: Double?
}

/// A list of hosts under a name: a vault's group, or a team.
struct HostSection: Identifiable {
    let id: String
    let title: String
    let hosts: [Host]
}
