package dev.detour.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.SwapHoriz
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import dev.detour.android.AppModel
import kotlinx.coroutines.launch
import dev.detour.android.OpenSession
import dev.detour.android.core.t
import dev.detour.android.terminal.TerminalSession

/** The open sessions: a tap brings one forward, a long press closes it. */
@Composable
fun SessionsScreen() {
    var picking by remember { mutableStateOf<NewKind?>(null) }
    var menu by remember { mutableStateOf(false) }
    var tunnel by remember { mutableStateOf<dev.detour.android.Tunnel?>(null) }
    var newTunnel by remember { mutableStateOf(false) }
    androidx.compose.runtime.LaunchedEffect(AppModel.sections) { dev.detour.android.Tunnels.load() }
    Box(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.fillMaxSize()) {
            item {
                LargeTitle(t("Sessions")) {
                    Box {
                        BarIcon(Icons.Outlined.Add, t("New session")) { menu = true }
                        NewSessionMenu(menu, { menu = false }, tunnel = { newTunnel = true }) { picking = it }
                    }
                }
            }
            if (AppModel.sessions.isEmpty() && dev.detour.android.Tunnels.list.isEmpty()) {
                item { EmptyState(Icons.Outlined.Terminal, t("No sessions"), t("New session")) { picking = NewKind.Terminal } }
            } else if (AppModel.sessions.isNotEmpty()) {
                section {
                    val list = AppModel.sessions.toList()
                    list.forEachIndexed { i, s -> SessionRow(s, i == list.lastIndex) }
                }
            }
            tunnelsSection { tunnel = it }
            item { Spacer(Modifier.height(40.dp)) }
        }
        TitleBar()
        val scope = androidx.compose.runtime.rememberCoroutineScope()
        UndoBar(dev.detour.android.Tunnels.deleted?.let { t("Deleted %@", it["name"] as? String ?: "") }, Modifier.align(androidx.compose.ui.Alignment.BottomCenter)) {
            scope.launch { dev.detour.android.Tunnels.undo() }
        }
    }
    // Undo for a tunnel deleted, for a few seconds.
    androidx.compose.runtime.LaunchedEffect(dev.detour.android.Tunnels.deleted) {
        if (dev.detour.android.Tunnels.deleted != null) { kotlinx.coroutines.delay(6000); dev.detour.android.Tunnels.deleted = null }
    }
    if (newTunnel || tunnel != null) TunnelEditor(tunnel) { newTunnel = false; tunnel = null }
    picking?.let { kind -> HostPicker(t("New session"), { picking = null }) { if (kind == NewKind.Files) AppModel.openFiles(it) else AppModel.open(it) } }
}

@Composable
private fun SessionRow(s: OpenSession, last: Boolean) {
    var menu by remember { mutableStateOf(false) }
    val live = when (s) {
        is OpenSession.Terminal -> s.session.state == TerminalSession.State.Connected
        is OpenSession.Files -> s.files.state == dev.detour.android.files.FilesSession.State.Ready
    }
    val title = when (s) {
        is OpenSession.Terminal -> s.session.title
        is OpenSession.Files -> s.host.label
    }
    Box {
        SwipeActions(listOf(closeAction { AppModel.close(s) })) {
            Cell(last = last, onClick = { AppModel.show(s) }, onLongClick = { menu = true }, inset = 52) {
                CellIcon(if (s is OpenSession.Files) Icons.Outlined.Folder else Icons.Outlined.Terminal)
                CellText(title, "${s.host.login}@${s.host.address}", mono = true)
                Box(Modifier.size(8.dp).clip(CircleShape).background(if (live) colors.green else colors.orange))
            }
        }
        DropdownMenu(menu, { menu = false }) {
            DropdownMenuItem(leadingIcon = menuIcon("Close"), text = { Text(t("Close"), color = colors.red) }, onClick = { menu = false; AppModel.close(s) })
        }
    }
}

/** The hosts to pick one from, for a new session. */
@Composable
fun HostPicker(title: String, close: () -> Unit, pick: (dev.detour.android.core.Host) -> Unit) {
    var query by remember { mutableStateOf("") }
    val q = query.trim().lowercase()
    val sections = if (q.isEmpty()) AppModel.sections else AppModel.sections.mapNotNull { s ->
        val hosts = s.hosts.filter { "${it.label} ${it.address} ${it.login}".lowercase().contains(q) }
        if (hosts.isEmpty()) null else dev.detour.android.core.HostSection(s.id, s.title, hosts)
    }
    Sheet(close) {
        SheetBar(title, left = { BarButton(t("Cancel"), onClick = close) })
        SearchField(query) { query = it }
        LazyColumn(Modifier.fillMaxSize()) {
            for (s in sections) {
                section(s.title.ifEmpty { null }, key = s.id) {
                    s.hosts.forEachIndexed { i, h ->
                        Cell(last = i == s.hosts.lastIndex, onClick = { close(); pick(h) }) {
                            CellText(h.label, "${h.login}@${h.address}", mono = true)
                        }
                    }
                }
            }
        }
    }
}

/** What a new session is: a terminal or a host's files. */
enum class NewKind { Terminal, Files }

/** The "+" of the sessions: Host or SFTP. */
@Composable
fun NewSessionMenu(open: Boolean, dismiss: () -> Unit, tunnel: (() -> Unit)? = null, pick: (NewKind) -> Unit) {
    DropdownMenu(open, dismiss) {
        DropdownMenuItem(text = { Text(t("Host")) }, onClick = { dismiss(); pick(NewKind.Terminal) }, leadingIcon = { androidx.compose.material3.Icon(Icons.Outlined.Terminal, null) })
        DropdownMenuItem({ Text("SFTP") }, { dismiss(); pick(NewKind.Files) }, leadingIcon = { androidx.compose.material3.Icon(Icons.Outlined.Folder, null) })
        if (tunnel != null) DropdownMenuItem(text = { Text(t("Tunnel")) }, onClick = { dismiss(); tunnel() }, leadingIcon = { androidx.compose.material3.Icon(androidx.compose.material.icons.Icons.Outlined.SwapHoriz, null) })
    }
}
