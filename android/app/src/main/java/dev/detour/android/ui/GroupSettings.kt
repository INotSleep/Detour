package dev.detour.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.core.Core
import dev.detour.android.core.ListedGroup
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.SshKey
import dev.detour.android.core.VaultData
import dev.detour.android.core.VaultHosts
import dev.detour.android.core.bool
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.core.t
import kotlinx.coroutines.launch

/** A saved proxy as the pickers list it: `vault|id`, and its name. */
data class ProxyChoice(val vault: String, val id: String, val name: String) {
    val ref: String get() = "$vault|$id"
}

/** The saved proxies of a vault. */
suspend fun proxiesOf(vault: String): List<ProxyChoice> =
    runCatching { VaultData.get(vault)["proxies"].list().map { it.obj() } }.getOrDefault(emptyList())
        .map { ProxyChoice(vault, it.str("id") ?: "", it.str("name") ?: "") }

/**
 * A group's settings: this person's own proxy and login for its hosts (kept in their personal
 * vault, used before the hosts' own), and for those who manage a team, the proxy of the whole team.
 */
@Composable
fun GroupSettingsSheet(vault: VaultHosts, group: ListedGroup, close: () -> Unit) {
    val key = "${vault.id}/${group.mine?.let { "mine:${it.id}" } ?: "team:${group.name}"}"
    // The team's proxy of a shared group, for those who manage the team.
    val teamProxyShown = vault.team && group.mine == null && vault.canManage
    var proxies by remember { mutableStateOf<List<ProxyChoice>>(emptyList()) }
    var teamProxies by remember { mutableStateOf<List<ProxyChoice>>(emptyList()) }
    var keys by remember { mutableStateOf<List<SshKey>>(emptyList()) }
    var proxy by remember { mutableStateOf("") }
    var teamProxy by remember { mutableStateOf("") }
    var teamProxyBefore by remember { mutableStateOf("") }
    var user by remember { mutableStateOf("") }
    var auth by remember { mutableStateOf("none") }
    var keyId by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var hasPassword by remember { mutableStateOf(false) }
    var loaded by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        keys = runCatching { PersonalVault.get()["keys"].list().map { SshKey.from(it.obj()) } }.getOrDefault(emptyList())
        proxies = proxiesOf(VaultData.PERSONAL)
        if (vault.team) teamProxies = proxiesOf(vault.id)
        val saved = runCatching { Core.call("group_prefs").list().map { it.obj() } }.getOrDefault(emptyList()).firstOrNull { it.str("key") == key }
        if (saved != null) {
            saved["proxy"]?.obj()?.let { p -> proxy = "${p.str("vault")}|${p.str("id")}" }
            user = saved.str("user") ?: ""
            val a = saved["auth"]?.obj()
            auth = a?.str("type") ?: "none"
            keyId = a?.str("keyId") ?: ""
            hasPassword = saved.bool("hasPassword") ?: false
        }
        if (keyId.isEmpty()) keyId = keys.firstOrNull()?.id ?: ""
        if (teamProxyShown) {
            teamProxy = VaultData.get(vault.id)["groups"].list().map { it.obj() }.firstOrNull { it.str("name") == group.name }?.str("proxyId") ?: ""
            teamProxyBefore = teamProxy
        }
        loaded = true
    }

    val ready = loaded && !(auth == "key" && keyId.isEmpty())

    fun save() {
        if (!ready || busy) return
        busy = true
        error = null
        val prefs = HashMap<String, Any?>()
        prefs["key"] = key
        proxy.takeIf { it.isNotEmpty() }?.split("|")?.let { (v, id) -> prefs["proxy"] = mapOf("vault" to v, "id" to id) }
        user.trim().takeIf { it.isNotEmpty() }?.let { prefs["user"] = it }
        when (auth) {
            "key" -> prefs["auth"] = mapOf("type" to "key", "keyId" to keyId)
            "none" -> {}
            else -> prefs["auth"] = mapOf("type" to auth)
        }
        prefs["hasPassword"] = auth == "password" && (password.isNotEmpty() || hasPassword)
        if (auth == "password" && password.isNotEmpty()) prefs["newPassword"] = password
        val empty = proxy.isEmpty() && user.isBlank() && auth == "none"
        scope.launch {
            try {
                Core.call("group_prefs_set", mapOf("prefs" to prefs, "drop" to empty))
                if (teamProxyShown && teamProxy != teamProxyBefore) {
                    VaultData.edit(vault.id) { v ->
                        val groups = v["groups"].list().map { it.obj().toMutableMap() }.toMutableList()
                        val g = groups.firstOrNull { it.str("name") == group.name }
                            ?: mutableMapOf<String, Any?>("id" to PersonalVault.newId(), "name" to group.name, "order" to ((groups.maxOfOrNull { (it["order"] as? Number)?.toDouble() ?: 0.0 } ?: 0.0) + 1)).also { groups += it }
                        if (teamProxy.isEmpty()) g.remove("proxyId") else g["proxyId"] = teamProxy
                        v["groups"] = groups
                    }
                }
                AppModel.reload()
                Hud.show(t("Saved"))
                close()
            } catch (e: Exception) {
                error = AppModel.describe(e)
            }
            busy = false
        }
    }

    val notSet = "" to t("Not set")
    Sheet(close) {
        SheetBar(
            group.name,
            left = { BarButton(t("Cancel"), onClick = close) },
            right = { if (busy) CircularProgressIndicator(color = colors.accent) else BarButton(t("Save"), bold = true, enabled = ready) { save() } },
        )
        LazyColumn(Modifier.fillMaxSize().imePadding()) {
            section(key = "proxy") {
                PickCell(t("Proxy"), proxy, listOf(notSet) + (proxies + teamProxies).map { it.ref to it.name }, last = !teamProxyShown) { proxy = it }
                if (teamProxyShown) {
                    PickCell(t("Proxy for the whole team"), teamProxy, listOf(notSet) + teamProxies.map { it.id to it.name }, last = true) { teamProxy = it }
                }
            }
            section(key = "user") {
                FieldCell(user, { user = it }, t("User"), mono = true, last = true)
            }
            section(t("Login"), key = "login") {
                Box(Modifier.padding(12.dp)) {
                    Segmented(
                        buildList {
                            add("none" to t("Not set"))
                            if (keys.isNotEmpty()) add("key" to t("Key"))
                            add("password" to t("Password"))
                            if (auth == "agent") add("agent" to "ssh-agent")
                        },
                        auth,
                    ) { auth = it }
                }
                if (auth == "password") {
                    FieldCell(password, { password = it }, if (hasPassword) t("Saved; type to change") else t("Ask when connecting"), secure = true, last = true)
                } else if (auth == "key") {
                    keys.forEachIndexed { i, k ->
                        Cell(last = i == keys.lastIndex, onClick = { keyId = k.id }) {
                            CellText(k.name, k.kind)
                            if (k.id == keyId) Text("✓", color = colors.accent, fontSize = 17.sp)
                        }
                    }
                }
            }
            errorItem(error)
        }
    }
}
