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
    /// Its place in its group; hosts without one come after, as they are.
    var order: Double?
    /// Set by the app: the vault it is in.
    var vaultId: String = "personal"
    /// Set by the app, in a team: this person's access to it (`none`, `view`, `secret`, `edit`).
    var access: String?
    /// Set by the app: the user from this person's settings of its group (theirs, then the
    /// shared one's), which connecting uses first.
    var groupUser: String?

    /// May this person change it: move it, edit it, delete it.
    var editable: Bool { access == nil || access == "edit" }

    struct OwnLogin: Decodable, Hashable {
        let user: String
        let active: Bool?
    }

    enum CodingKeys: String, CodingKey { case id, group, label, address, port, user, own, order }

    /// The login used to connect: this person's group settings, then their own login on a team
    /// host while it is on, then the host's.
    var login: String { groupUser ?? own.flatMap { $0.active == false ? nil : $0.user } ?? user }
}

struct VaultView: Decodable {
    let hosts: [Host]
    var groups: [HostGroup]?
}

/// A vault's record of a group: its place among the others (its hosts name it).
struct HostGroup: Decodable, Hashable {
    let id: String
    let name: String
    var order: Double?
}

/// A group of this person's own grouping of a team's hosts ("My groups"): host ids in order.
struct MyGroup: Codable, Hashable {
    let id: String
    var name: String
    var hosts: [String]
}

/// This person's own grouping of a team's hosts.
struct Layout: Codable {
    var groups: [MyGroup]
}

/// A vault's hosts as the hosts screen shows them: the personal vault or a team.
struct VaultHosts: Identifiable {
    let id: String
    /// The team's name; nil for the personal vault.
    let name: String?
    /// In a team: `owner`, `admin` or `member`.
    let role: String?
    var hosts: [Host]
    let groups: [HostGroup]
    /// This person's own grouping of a team's hosts, once made.
    var layout: [MyGroup]?

    var team: Bool { name != nil }
    /// The shared groups and their order are for those who manage the team.
    var canManage: Bool { !team || role == "owner" || role == "admin" }
}

struct TeamEntry: Decodable {
    let id: String
    let path: String
    let view: TeamView?
    /// Why the team could not be opened.
    let error: String?

    struct TeamView: Decodable {
        let id: String
        let name: String
        /// Added by an admin: until then the team's contents can't be read.
        let joined: Bool
        let members: [Member]
        let data: VaultView
        let remote: Remote?
        let layout: Layout?
        /// Who has which access to the records this person can see.
        let access: [Access]?
    }

    struct Access: Decodable {
        let record: String
        let member: String
        let level: String
    }

    struct Member: Decodable {
        let id: String
        let name: String
        /// `owner`, `admin` or `member`.
        let role: String
        let fingerprint: String
        let isYou: Bool
    }

    /// Where the team is kept, how syncing it goes, and why reaching it failed.
    struct Remote: Decodable {
        let url: String
        let kind: String
        let pending: Bool
        let error: CoreErrorInfo?
        let syncedAt: Double?
    }
}

/// This person's identity: what an admin adds to a team.
struct MyId: Decodable {
    let detourId: String
    let fingerprint: String
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
    /// The drive folder, when it syncs through a cloud drive.
    let drive: DriveFolderRef?
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
