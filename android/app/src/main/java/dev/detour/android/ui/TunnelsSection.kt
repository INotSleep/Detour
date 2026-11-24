package dev.detour.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.SwapHoriz
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import dev.detour.android.AppModel
import dev.detour.android.Tunnel
import dev.detour.android.Tunnels
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import dev.detour.android.terminal.TerminalSession
import kotlinx.coroutines.launch

/** What starting a tunnel waits for from the person. */
private sealed interface Ask {
    data class Password(val tunnel: Tunnel, val prompt: String) : Ask
    data class Key(val tunnel: Tunnel, val fingerprint: String) : Ask
}

/** The tunnels in the Sessions tab: a switch opens or closes each one. */
fun LazyListScope.tunnelsSection(edit: (Tunnel?) -> Unit) {
    if (Tunnels.list.isEmpty()) return
    section(t("Tunnels"), key = "tunnels") {
        val list = Tunnels.list
        list.forEachIndexed { i, tunnel -> TunnelRow(tunnel, i == list.lastIndex, edit) }
    }
}

@Composable
private fun TunnelRow(tunnel: Tunnel, last: Boolean, edit: (Tunnel?) -> Unit) {
    val scope = rememberCoroutineScope()
    var menu by remember { mutableStateOf(false) }
    var ask by remember { mutableStateOf<Ask?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val status = Tunnels.statuses[tunnel.key]
    val on = status?.on == true
    val busy = tunnel.key in Tunnels.starting
    val personal = tunnel.vaultId == "personal"

    fun start(secret: String? = null, trust: String? = null) = scope.launch {
        error = null
        try {
            Tunnels.start(tunnel, secret, trust)
        } catch (f: CoreFailure) {
            when (f.kind) {
                "needSecret" -> ask = Ask.Password(tunnel, f["prompt"] ?: "")
                "hostKeyUnknown" -> ask = Ask.Key(tunnel, f["fingerprint"] ?: "")
                else -> error = TerminalSession.describe(f)
            }
        } catch (e: Exception) {
            error = e.message
        }
    }

    fun remove() = scope.launch { Tunnels.remove(tunnel) }

    Box {
        SwipeActions(if (personal) listOf(deleteAction { remove() }, editAction { edit(tunnel) }) else emptyList()) {
            Cell(last = last, onClick = if (personal) ({ edit(tunnel) }) else null, onLongClick = { menu = true }, inset = 52) {
                CellIcon(Icons.Outlined.SwapHoriz, if (on) colors.green else colors.accent)
                val detail = error ?: status?.error?.let { TerminalSession.describe(it) }
                    ?: if (on) t("Connections: %@", status.connections) else route(tunnel)
                CellText(tunnel.name, detail, mono = error == null && status?.error == null && !on)
                if (busy) CircularProgressIndicator(Modifier.size(22.dp), color = colors.accent, strokeWidth = 2.dp)
                else IosSwitch(on, { want -> if (want) start() else scope.launch { Tunnels.stop(tunnel) } })
            }
        }
        DropdownMenu(menu, { menu = false }) {
            if (tunnel.kind != "remote") DropdownMenuItem(leadingIcon = menuIcon("Copy address"), text = { Text(t("Copy address")) }, onClick = { menu = false; Clipboard.copy("127.0.0.1:${tunnel.bindPort}") })
            if (personal) {
                DropdownMenuItem(leadingIcon = menuIcon("Edit"), text = { Text(t("Edit")) }, onClick = { menu = false; edit(tunnel) })
                DropdownMenuItem(leadingIcon = menuIcon("Delete"), text = { Text(t("Delete"), color = colors.red) }, onClick = {
                    menu = false
                    remove()
                })
            }
        }
    }

    when (val a = ask) {
        is Ask.Password -> {
            var password by remember { mutableStateOf("") }
            AlertDialog(
                onDismissRequest = { ask = null },
                title = { Text(a.prompt.ifEmpty { t("Password") }) },
                text = { Field(password, { password = it }, t("Password"), secure = true, ime = ImeAction.Go, onDone = { ask = null; start(secret = password) }) },
                confirmButton = { TextButton({ ask = null; start(secret = password) }, enabled = password.isNotEmpty()) { Text(t("Connect")) } },
                dismissButton = { TextButton({ ask = null }) { Text(t("Cancel")) } },
            )
        }
        is Ask.Key -> AlertDialog(
            onDismissRequest = { ask = null },
            title = { Text(t("New server key")) },
            text = { Text(a.fingerprint, fontFamily = FontFamily.Monospace, fontSize = 13.sp) },
            confirmButton = { TextButton({ ask = null; start(trust = a.fingerprint) }) { Text(t("Trust and connect")) } },
            dismissButton = { TextButton({ ask = null }) { Text(t("Cancel")) } },
        )
        null -> {}
    }
}

/** Where a tunnel goes, in short: "8080 → db:5432 · web-1". */
private fun route(t: Tunnel): String {
    val via = AppModel.sections.flatMap { it.hosts }.firstOrNull { it.id == t.viaHostId && it.vaultId == t.vaultId }?.label ?: "?"
    return when (t.kind) {
        "remote" -> "$via:${t.bindPort} → ${t.target ?: ""}"
        "dynamic" -> "SOCKS5 127.0.0.1:${t.bindPort} · $via"
        else -> "127.0.0.1:${t.bindPort} → ${t.target ?: ""} · $via"
    }
}

/** A new tunnel of the personal vault, or one changed. */
@Composable
fun TunnelEditor(tunnel: Tunnel?, close: () -> Unit) {
    var name by remember { mutableStateOf(tunnel?.name ?: "") }
    var kind by remember { mutableStateOf(tunnel?.kind ?: "local") }
    var port by remember { mutableStateOf(tunnel?.bindPort?.toString() ?: "") }
    var target by remember { mutableStateOf(tunnel?.target ?: "") }
    var via by remember { mutableStateOf(tunnel?.viaHostId ?: "") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val hosts = AppModel.sections.flatMap { it.hosts }.filter { it.vaultId == "personal" }
    LaunchedEffect(Unit) { if (via.isEmpty()) via = hosts.firstOrNull()?.id ?: "" }

    val portNumber = port.trim().toIntOrNull()?.takeIf { it in 1..65535 }
    val targetOk = kind == "dynamic" || Regex("^\\[?[^\\s\\]]+]?:\\d{1,5}$").matches(target.trim())
    val ready = portNumber != null && via.isNotEmpty() && targetOk && !busy

    fun save() {
        if (!ready) return
        busy = true
        val t = Tunnel(
            tunnel?.id ?: PersonalVault.newId(), "personal",
            name.trim().ifEmpty { if (kind == "dynamic") "SOCKS5 $portNumber" else target.trim() },
            kind, portNumber!!, via, if (kind == "dynamic") null else target.trim(),
        )
        scope.launch {
            try {
                PersonalVault.save("tunnels", t.json())
                Tunnels.load()
                close()
            } catch (f: CoreFailure) {
                error = f["message"] ?: f.kind
            }
            busy = false
        }
    }

    Sheet(close) {
        SheetBar(
            if (tunnel == null) t("New tunnel") else t("Tunnel"),
            left = { BarButton(t("Cancel"), onClick = close) },
            right = { if (busy) CircularProgressIndicator(Modifier.size(22.dp), color = colors.accent) else BarButton(t("Save"), bold = true, enabled = ready) { save() } },
        )
        LazyColumn(Modifier.fillMaxSize().imePadding()) {
            section {
                Box(Modifier.padding(12.dp)) {
                    Segmented(listOf("local" to t("To the server"), "remote" to t("From the server"), "dynamic" to "SOCKS5"), kind) { kind = it }
                }
            }
            section(footer = when (kind) {
                "remote" -> t("Expose my port on the server")
                "dynamic" -> t("Proxy through the server")
                else -> t("Open a server port here")
            }) {
                FieldCell(port, { port = it }, if (kind == "remote") t("Port on the server") else t("Port on the phone"), number = true)
                if (kind != "dynamic") FieldCell(target, { target = it }, if (kind == "remote") t("Address on the phone, host:port") else t("Address from the server, host:port"), plain = true, mono = true)
                FieldCell(name, { name = it }, t("Name"), last = true, ime = ImeAction.Done, onDone = { save() })
            }
            section(t("Server")) {
                hosts.forEachIndexed { i, h ->
                    Cell(last = i == hosts.lastIndex, onClick = { via = h.id }) {
                        CellText(h.label, "${h.login}@${h.address}", mono = true)
                        if (h.id == via) Text("✓", color = colors.accent, fontSize = 17.sp)
                    }
                }
            }
            error?.let { e -> item { Text(e, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(32.dp, 8.dp)) } }
        }
    }
}
