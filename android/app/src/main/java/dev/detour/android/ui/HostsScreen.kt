package dev.detour.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.core.Host
import dev.detour.android.core.HostSection
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** The hosts of the vault and its teams; tapping one opens it in a terminal. */
@Composable
fun HostsScreen() {
    var query by remember { mutableStateOf("") }
    /** The host editor: a host's id, or "" for a new one. */
    var editing by remember { mutableStateOf<String?>(null) }
    var deleted by remember { mutableStateOf<Pair<Map<String, Any?>, String>?>(null) }
    var ownLogin by remember { mutableStateOf<Host?>(null) }
    var folderTeam by remember { mutableStateOf<dev.detour.android.core.PendingTeam?>(null) }
    var joining by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    // A join link opened the app: the form, filled in from it.
    LaunchedEffect(AppModel.pendingJoin) {
        if (AppModel.pendingJoin == null) return@LaunchedEffect
        if (AppModel.sync?.enabled == true) AppModel.pendingJoin = null else joining = true
    }

    val q = query.trim().lowercase()
    val filtered = if (q.isEmpty()) AppModel.sections else AppModel.sections.mapNotNull { s ->
        val hosts = s.hosts.filter { "${it.label} ${it.address} ${it.login} ${s.title}".lowercase().contains(q) }
        if (hosts.isEmpty()) null else HostSection(s.id, s.title, hosts)
    }

    fun delete(h: Host) = scope.launch {
        val removed = runCatching { PersonalVault.removeHost(h.id) }.getOrNull() ?: return@launch
        AppModel.reload()
        deleted = removed to h.label
    }

    LaunchedEffect(deleted) {
        if (deleted != null) { delay(6000); deleted = null }
    }

    Box(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.fillMaxSize()) {
            item {
                LargeTitle(t("Hosts")) { BarIcon(Icons.Outlined.Add, t("New host")) { editing = "" } }
                SearchField(query) { query = it }
            }
            if (q.isEmpty()) driveNeedsSection { folderTeam = it }
            if (AppModel.sections.isEmpty() && AppModel.driveSignIns.isEmpty() && AppModel.folderTeams.isEmpty()) {
                item { EmptyState(Icons.Outlined.Dns, t("No hosts"), t("New host")) { editing = "" } }
            } else if (filtered.isEmpty()) {
                item { EmptyState(Icons.Outlined.Dns, t("No results")) }
            }
            for (s in filtered) {
                section(s.title.ifEmpty { null }, key = s.id) {
                    s.hosts.forEachIndexed { i, h ->
                        HostRow(h, last = i == s.hosts.lastIndex, edit = { editing = h.id }, delete = { delete(h) }, ownLogin = { ownLogin = h })
                    }
                }
            }
            item { Spacer(Modifier.height(80.dp)) }
        }
        TitleBar()
        UndoBar(deleted?.let { t("Deleted %@", it.second) }, Modifier.align(Alignment.BottomCenter)) {
            val d = deleted ?: return@UndoBar
            deleted = null
            scope.launch { runCatching { PersonalVault.saveHost(d.first) }; AppModel.reload() }
        }
    }
    editing?.let { id -> HostEditor(id.ifEmpty { null }) { editing = null } }
    ownLogin?.let { h -> OwnLoginEditor(h) { ownLogin = null } }
    folderTeam?.let { t -> TeamFromDriveSheet(t) { folderTeam = null } }
    if (joining) SyncJoinSheet { joining = false }
}

@Composable
private fun HostRow(host: Host, last: Boolean, edit: () -> Unit, delete: () -> Unit, ownLogin: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Box {
        SwipeActions(if (host.vaultId == "personal") listOf(deleteAction(delete), editAction(edit)) else emptyList()) {
            Cell(last = last, onClick = { AppModel.open(host) }, onLongClick = { menu = true }) {
                CellText(host.label, "${host.login}@${host.address}${if (host.port == 22) "" else ":${host.port}"}", mono = true)
            }
        }
        DropdownMenu(menu, { menu = false }) {
            DropdownMenuItem(leadingIcon = menuIcon("Connect"), text = { Text(t("Connect")) }, onClick = { menu = false; AppModel.open(host) })
            DropdownMenuItem(leadingIcon = menuIcon("Files"), text = { Text(t("Files")) }, onClick = { menu = false; AppModel.openFiles(host) })
            DropdownMenuItem(leadingIcon = menuIcon("Copy ssh command"), text = { Text(t("Copy ssh command")) }, onClick = {
                menu = false
                Clipboard.copy("ssh " + (if (host.port == 22) "" else "-p ${host.port} ") + "${host.login}@${host.address}")
            })
            if (host.vaultId != "personal") {
                DropdownMenuItem(leadingIcon = menuIcon("My login"), text = { Text(t("My login")) }, onClick = { menu = false; ownLogin() })
            }
            if (host.vaultId == "personal") {
                DropdownMenuItem(leadingIcon = menuIcon("Edit"), text = { Text(t("Edit")) }, onClick = { menu = false; edit() })
                DropdownMenuItem(leadingIcon = menuIcon("Delete"), text = { Text(t("Delete"), color = colors.red) }, onClick = { menu = false; delete() })
            }
        }
    }
}

/** iOS's search field under a large title. */
@Composable
fun SearchField(query: String, onChange: (String) -> Unit) {
    Box(Modifier.padding(horizontal = 16.dp).padding(bottom = 4.dp)) {
        Field(query, onChange, t("Search"), plain = true)
    }
}

/** "Deleted X" with Undo, over the bottom of a screen for a few seconds. */
@Composable
fun UndoBar(text: String?, modifier: Modifier = Modifier, undo: () -> Unit) {
    AnimatedVisibility(text != null, modifier, enter = slideInVertically { it } + fadeIn(), exit = slideOutVertically { it } + fadeOut()) {
        Row(
            Modifier.padding(16.dp).fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(colors.cell).padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(text ?: "", color = colors.label, fontSize = 15.sp, maxLines = 1, modifier = Modifier.weight(1f))
            BarButton(t("Undo"), bold = true, onClick = undo)
        }
    }
}
