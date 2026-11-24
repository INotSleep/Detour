package dev.detour.android.core

/** A host as the core shows it (no secrets). */
data class Host(
    val id: String,
    val group: String,
    val label: String,
    val address: String,
    val port: Int,
    val user: String,
    val own: OwnLogin?,
    /** Set by the app: the vault it is in. */
    val vaultId: String = "personal",
    /** Its place in its group; hosts without one come after, as they are. */
    val order: Double? = null,
    /** In a team: this person's access to it (`none`, `view`, `secret`, `edit`); null in the personal vault. */
    val access: String? = null,
    /** The user from this person's settings of its group (theirs, then the shared one's), which connecting uses first. */
    val groupUser: String? = null,
) {
    /** May this person change it: move it, edit it, delete it. */
    val editable: Boolean get() = access == null || access == "edit"

    data class OwnLogin(val user: String, val active: Boolean?)

    /** The login used to connect: this person's group settings, then their own login on a team host while it is on, then the host's. */
    val login: String get() = groupUser ?: own?.takeIf { it.active != false }?.user ?: user

    companion object {
        fun from(m: Map<String, Any?>, vault: String, access: String? = null) = Host(
            id = m.str("id") ?: "",
            group = m.str("group") ?: "",
            label = m.str("label") ?: "",
            address = m.str("address") ?: "",
            port = m.int("port") ?: 22,
            user = m.str("user") ?: "",
            own = m["own"]?.obj()?.let { OwnLogin(it.str("user") ?: "", it.bool("active")) },
            vaultId = vault,
            order = m.double("order"),
            access = access,
        )
    }
}

/** A vault's record of a group: its place among the others (its hosts name it). */
data class HostGroup(val id: String, val name: String, val order: Double) {
    companion object {
        fun from(m: Map<String, Any?>) = HostGroup(m.str("id") ?: "", m.str("name") ?: "", m.double("order") ?: 0.0)
    }
}

/** A group of this person's own grouping of a team's hosts ("My groups"): host ids in order. */
data class MyGroup(val id: String, val name: String, val hosts: List<String>) {
    val json: Map<String, Any?> get() = mapOf("id" to id, "name" to name, "hosts" to hosts)

    companion object {
        fun from(m: Map<String, Any?>) = MyGroup(m.str("id") ?: "", m.str("name") ?: "", m["hosts"].list().mapNotNull { it as? String })
    }
}

/** A vault's hosts as the hosts screen shows them: the personal vault or a team. */
data class VaultHosts(
    val id: String,
    /** The team's name; null for the personal vault. */
    val name: String?,
    /** In a team: `owner`, `admin` or `member`. */
    val role: String?,
    val hosts: List<Host>,
    val groups: List<HostGroup>,
    /** This person's own grouping of a team's hosts, once made. */
    val layout: List<MyGroup>?,
) {
    val team: Boolean get() = name != null
    /** The shared groups and their order are for those who manage the team. */
    val canManage: Boolean get() = !team || role == "owner" || role == "admin"
}

/** A list of hosts under a name: a vault's group, or a team. */
data class HostSection(val id: String, val title: String, val hosts: List<Host>)

data class CoreErrorInfo(val kind: String, val message: String?, val drive: String?, val account: String?) {
    companion object {
        fun from(v: Any?): CoreErrorInfo? = v?.obj()?.takeIf { it.isNotEmpty() }?.let {
            CoreErrorInfo(it.str("kind") ?: "internal", it.str("message"), it.str("provider") ?: it.str("drive"), it.str("account"))
        }
    }
}

data class SyncStatus(
    val enabled: Boolean,
    val location: String?,
    val git: Boolean,
    /** `folder`, `git`, `onedrive`, `gdrive` or `dropbox`. */
    val storage: String?,
    val pending: Boolean,
    val error: CoreErrorInfo?,
    val syncedAt: Double?,
) {
    companion object {
        fun from(v: Any?): SyncStatus? = v?.obj()?.let {
            SyncStatus(it.bool("enabled") ?: false, it.str("location"), it.bool("git") ?: false, it.str("storage"), it.bool("pending") ?: false, CoreErrorInfo.from(it["error"]), it.double("syncedAt"))
        }
    }
}
