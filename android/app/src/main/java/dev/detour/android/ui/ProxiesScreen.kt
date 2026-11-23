package dev.detour.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Shield
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.MainTab
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.int
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.core.t
import dev.detour.android.terminal.TerminalSession
import kotlinx.coroutines.launch

/** A saved proxy, as the core shows it (`ProxyView`): its kind stays JSON, there are several. */
private class ProxyItem(val vaultId: String, val json: Map<String, Any?>) {
    val id: String get() = "$vaultId/${json.str("id") ?: ""}"
    val name: String get() = json.str("name") ?: ""
    private val kind: Map<String, Any?> get() = json["kind"].obj()

    /** "VLESS · example.com:443" */
    val summary: String
        get() {
            val type = (kind.str("type") ?: "").uppercase()
            val shown = when (type) { "SOCKS5" -> "SOCKS5"; "WIREGUARD" -> "WireGuard"; else -> type }
            val place = kind.str("endpoint") ?: kind.str("address")?.let { "$it:${kind.int("port") ?: 0}" } ?: ""
            return listOf(shown, place).filter { it.isNotEmpty() }.joinToString(" · ")
        }
}

/** The proxies of one vault: the personal one (no name) or a team's. */
private class ProxyGroup(val id: String, val name: String?, val proxies: List<ProxyItem>)

/**
 * The proxies of the vault and its teams: checked with a tap, added from a link or a WireGuard
 * config, the personal vault's deleted with a long press.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ProxiesScreen() {
    var groups by remember { mutableStateOf<List<ProxyGroup>?>(null) }
    val checks = remember { mutableStateMapOf<String, String>() }
    val checking = remember { mutableStateMapOf<String, Boolean>() }
    var adding by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    suspend fun load() {
        val out = ArrayList<ProxyGroup>()
        runCatching { PersonalVault.get() }.getOrNull()?.get("proxies").list().takeIf { it.isNotEmpty() }?.let { list ->
            out.add(ProxyGroup("personal", null, list.map { ProxyItem("personal", it.obj()) }))
        }
        runCatching { Core.call("teams_list") }.getOrNull()?.list()?.forEach { t ->
            val view = t.obj()["view"].obj()
            val id = view.str("id") ?: return@forEach
            val list = view["data"].obj()["proxies"].list().takeIf { it.isNotEmpty() } ?: return@forEach
            out.add(ProxyGroup(id, view.str("name"), list.map { ProxyItem(id, it.obj()) }))
        }
        groups = out
    }

    /** Connects through the proxy once: how long it took, or what went wrong. */
    fun check(p: ProxyItem) = scope.launch {
        checking[p.id] = true
        try {
            val r = Core.call("proxy_check", mapOf("vaultId" to p.vaultId, "proxy" to p.json)).obj()
            checks[p.id] = "${r.int("ms") ?: 0} ms"
        } catch (f: CoreFailure) {
            checks[p.id] = TerminalSession.describe(f)
        } catch (e: Exception) {
            checks[p.id] = e.message ?: e.toString()
        } finally {
            checking.remove(p.id)
        }
    }

    fun delete(p: ProxyItem) = scope.launch {
        val id = p.json.str("id")
        try {
            PersonalVault.edit { v -> v["proxies"] = v["proxies"].list().filter { it.obj().str("id") != id } }
            error = null
        } catch (e: Exception) {
            error = errorText(e)
        }
        load()
    }

    // Loads each time the tab is shown, and again when the vault changes (the hosts reload then).
    val shown = AppModel.tab == MainTab.Proxies
    LaunchedEffect(shown, AppModel.sections) { if (shown) load() }

    PullToRefreshBox(
        refreshing,
        onRefresh = { scope.launch { refreshing = true; load(); refreshing = false } },
        modifier = Modifier.fillMaxSize(),
    ) {
        LazyColumn(Modifier.fillMaxSize()) {
            item { LargeTitle(t("Proxies")) { BarIcon(Icons.Outlined.Add, t("Add proxy")) { adding = true } } }
            val gs = groups
            if (gs == null) {
                item {
                    Box(Modifier.padding(top = 120.dp).fillMaxSize(), contentAlignment = Alignment.TopCenter) {
                        CircularProgressIndicator(color = colors.secondary)
                    }
                }
            } else if (gs.isEmpty()) {
                item { EmptyState(Icons.Outlined.Shield, t("No proxies"), t("Add proxy")) { adding = true } }
            }
            for (g in gs.orEmpty()) {
                val header = g.name ?: if (gs.orEmpty().size > 1) t("My proxies") else null
                section(header, key = g.id) {
                    g.proxies.forEachIndexed { i, p ->
                        ProxyRow(
                            p, last = i == g.proxies.lastIndex,
                            checking = checking[p.id] == true, result = checks[p.id],
                            check = { check(p) }, delete = if (p.vaultId == "personal") ({ delete(p) }) else null,
                        )
                    }
                }
            }
            error?.let { e -> item { Text(e, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(32.dp, 8.dp)) } }
            item { Spacer(Modifier.height(40.dp)) }
        }
        TitleBar()
    }
    if (adding) AddProxySheet(added = { load() }) { adding = false }
}

@Composable
private fun ProxyRow(p: ProxyItem, last: Boolean, checking: Boolean, result: String?, check: () -> Unit, delete: (() -> Unit)?) {
    var menu by remember { mutableStateOf(false) }
    Box {
        SwipeActions(listOfNotNull(delete?.let { deleteAction(it) })) {
            Cell(last = last, onClick = check, onLongClick = delete?.let { { menu = true } }) {
                Column(Modifier.weight(1f)) {
                    Text(p.name, color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(p.summary, color = colors.secondary, fontSize = 12.sp, fontFamily = FontFamily.Monospace, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (checking) {
                    CircularProgressIndicator(Modifier.padding(start = 8.dp).size(20.dp), color = colors.secondary, strokeWidth = 2.dp)
                } else if (result != null) {
                    Text(
                        result, color = if (result.endsWith("ms")) colors.green else colors.red, fontSize = 13.sp,
                        maxLines = 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.End,
                        modifier = Modifier.padding(start = 8.dp).widthIn(max = 200.dp),
                    )
                }
            }
        }
        if (delete != null) {
            DropdownMenu(menu, { menu = false }) {
                DropdownMenuItem(leadingIcon = menuIcon("Delete"), text = { Text(t("Delete"), color = colors.red) }, onClick = { menu = false; delete() })
            }
        }
    }
}

/** What went wrong saving the vault, in words. */
private fun errorText(e: Throwable): String {
    val f = e as? CoreFailure ?: return e.message ?: e.toString()
    return when (f.kind) {
        "io", "unreachable" -> t("No connection")
        else -> f["message"] ?: f.kind
    }
}
