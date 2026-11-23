package dev.detour.android

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.int
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** A tunnel of the vault or a team: a port forwarded through one of its hosts. */
data class Tunnel(
    val id: String,
    val vaultId: String,
    val name: String,
    /** `local` (ssh -L), `remote` (ssh -R) or `dynamic` (ssh -D, SOCKS5). */
    val kind: String,
    val bindPort: Int,
    val viaHostId: String,
    val target: String?,
) {
    val key get() = "$vaultId/$id"

    fun json(): Map<String, Any?> = mapOf("id" to id, "name" to name, "kind" to kind, "bindPort" to bindPort, "viaHostId" to viaHostId, "target" to target)

    companion object {
        fun from(m: Map<String, Any?>, vault: String) = Tunnel(
            m.str("id") ?: "", vault, m.str("name") ?: "", m.str("kind") ?: "local", m.int("bindPort") ?: 0, m.str("viaHostId") ?: "", m.str("target"),
        )
    }
}

/** How a tunnel is now, as the core says. */
data class TunnelStatus(val on: Boolean, val connections: Int, val error: CoreFailure?)

/**
 * The tunnels and which of them run. They run in the core for as long as the app does: the
 * keep-alive service keeps it running in the background while one is on.
 */
object Tunnels {
    var list by mutableStateOf<List<Tunnel>>(emptyList())
        private set
    var statuses by mutableStateOf<Map<String, TunnelStatus>>(emptyMap())
        private set
    /** Tunnels being started now. */
    var starting by mutableStateOf<Set<String>>(emptySet())
        private set

    val running: Int get() = statuses.values.count { it.on }

    /** The tunnel just deleted, as the vault had it, for Undo. */
    var deleted by mutableStateOf<Map<String, Any?>?>(null)

    /** Stops a tunnel and takes it out of the vault; Undo puts it back. */
    suspend fun remove(t: Tunnel) {
        stop(t)
        deleted = runCatching { dev.detour.android.core.PersonalVault.remove("tunnels", t.id) }.getOrNull()
        load()
    }

    suspend fun undo() {
        val d = deleted ?: return
        deleted = null
        runCatching { dev.detour.android.core.PersonalVault.save("tunnels", d) }
        load()
    }

    private var polling: Job? = null

    suspend fun load() {
        val out = ArrayList<Tunnel>()
        runCatching { Core.call("vault_get").obj() }.getOrNull()?.let { v -> out += v["tunnels"].list().map { Tunnel.from(it.obj(), "personal") } }
        runCatching { Core.call("teams_list").list() }.getOrNull()?.forEach { t ->
            val view = t.obj()["view"]?.obj() ?: return@forEach
            val id = view.str("id") ?: return@forEach
            out += view["data"].obj()["tunnels"].list().map { Tunnel.from(it.obj(), id) }
        }
        list = out
        refresh()
    }

    /** The core's statuses, now and every two seconds while any tunnel runs. */
    suspend fun refresh() {
        val raw = runCatching { Core.call("tunnels_status").list() }.getOrDefault(emptyList())
        statuses = raw.map { it.obj() }.associate { s ->
            (s.str("key") ?: "") to TunnelStatus(s.str("state") == "on", s.int("connections") ?: 0, s["error"]?.obj()?.let { CoreFailure(it.str("kind") ?: "internal", it) })
        }
        KeepAliveService.update()
        if (running > 0 && polling?.isActive != true) {
            polling = AppModel.scope.launch {
                while (running > 0) {
                    delay(2000)
                    refresh()
                }
            }
        }
    }

    /** Starts a tunnel; a password or a new server key comes back as the core's failure. */
    suspend fun start(t: Tunnel, secret: String? = null, trust: String? = null) {
        starting = starting + t.key
        try {
            Core.call("tunnel_start", mapOf("vaultId" to t.vaultId, "tunnelId" to t.id, "secret" to secret, "trustFingerprint" to trust))
        } finally {
            starting = starting - t.key
            refresh()
        }
    }

    suspend fun stop(t: Tunnel) {
        runCatching { Core.call("tunnel_stop", mapOf("vaultId" to t.vaultId, "tunnelId" to t.id)) }
        refresh()
    }
}
