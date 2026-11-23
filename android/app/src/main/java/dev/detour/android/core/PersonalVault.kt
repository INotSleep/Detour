package dev.detour.android.core

import java.util.UUID

/**
 * Changes to the personal vault, the way the desktop app makes them: the vault as the core last
 * showed it, changed, and put back with what it was (`base`), so only these changes apply.
 */
object PersonalVault {
    suspend fun get(): Map<String, Any?> = Core.call("vault_get").obj()

    /** Reads the vault, lets `change` edit it, and saves the result. */
    suspend fun edit(change: (MutableMap<String, Any?>) -> Unit) {
        val base = get()
        val next = base.toMutableMap()
        change(next)
        Core.call("vault_put", mapOf("update" to pick(next), "base" to pick(base)))
    }

    private fun pick(d: Map<String, Any?>) = mapOf(
        "hosts" to (d["hosts"] ?: emptyList<Any>()), "keys" to (d["keys"] ?: emptyList<Any>()),
        "snippets" to (d["snippets"] ?: emptyList<Any>()), "tunnels" to (d["tunnels"] ?: emptyList<Any>()),
        "proxies" to (d["proxies"] ?: emptyList<Any>()),
    )

    fun newId(): String = UUID.randomUUID().toString().replace("-", "").lowercase()

    /** Adds `item` to the list `field`, or replaces the one with its id. */
    suspend fun save(field: String, item: Map<String, Any?>) = edit { v ->
        val list = v[field].list().map { it.obj() }.toMutableList()
        val i = list.indexOfFirst { it.str("id") == item.str("id") }
        if (i >= 0) list[i] = item else list.add(item)
        v[field] = list
    }

    /** Removes an item of the list `field`; returns it as it was, for Undo. */
    suspend fun remove(field: String, id: String): Map<String, Any?>? {
        var removed: Map<String, Any?>? = null
        edit { v ->
            val list = v[field].list().map { it.obj() }.toMutableList()
            val i = list.indexOfFirst { it.str("id") == id }
            if (i >= 0) removed = list.removeAt(i)
            v[field] = list
        }
        return removed
    }

    suspend fun saveHost(host: Map<String, Any?>) = save("hosts", host)
    suspend fun removeHost(id: String) = remove("hosts", id)
}

/** A key of the personal vault as the core shows it. */
data class SshKey(
    val id: String,
    val name: String,
    val algorithm: String,
    val bits: Int?,
    val fingerprint: String,
    val publicKey: String,
    val hasPassphrase: Boolean,
) {
    val kind: String
        get() = when (algorithm) {
            "ed25519" -> "Ed25519"
            "rsa" -> bits?.let { "RSA $it" } ?: "RSA"
            "ecdsa" -> "ECDSA"
            else -> algorithm
        }

    companion object {
        fun from(m: Map<String, Any?>) = SshKey(
            m.str("id") ?: "", m.str("name") ?: "", m.str("algorithm") ?: "", m.int("bits"),
            m.str("fingerprint") ?: "", m.str("publicKey") ?: "", m.bool("hasPassphrase") ?: false,
        )
    }
}
