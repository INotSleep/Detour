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
) {
    data class OwnLogin(val user: String, val active: Boolean?)

    /** The login used to connect: this person's own one on a team host, if set. */
    val login: String get() = own?.takeIf { it.active != false }?.user ?: user

    companion object {
        fun from(m: Map<String, Any?>, vault: String) = Host(
            id = m.str("id") ?: "",
            group = m.str("group") ?: "",
            label = m.str("label") ?: "",
            address = m.str("address") ?: "",
            port = m.int("port") ?: 22,
            user = m.str("user") ?: "",
            own = m["own"]?.obj()?.let { OwnLogin(it.str("user") ?: "", it.bool("active")) },
            vaultId = vault,
        )
    }
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
