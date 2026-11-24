package dev.detour.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.CreateNewFolder
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.core.Groups
import dev.detour.android.core.Host
import dev.detour.android.core.ListedGroup
import dev.detour.android.core.PendingTeam
import dev.detour.android.core.VaultData
import dev.detour.android.core.VaultHosts
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** What Undo brings back: a host, or a group with its hosts. */
private sealed interface Deleted {
    val label: String
    class HostItem(val vault: String, val host: Map<String, Any?>, override val label: String) : Deleted
    class SharedGroup(val vault: String, val name: String, val record: Map<String, Any?>?, val members: List<String>) : Deleted { override val label get() = name }
    class MineGroup(val vault: String, val layout: List<dev.detour.android.core.MyGroup>, override val label: String) : Deleted
}

/** A group being named: a new one (`from` null) or one renamed. */
private data class Naming(val from: ListedGroup?)

/**
 * The hosts of one vault: the personal one or a team's (chips at the top switch, and new hosts go
 * to the one shown), in its groups or, in a team, this person's own. A tap connects; a hold opens
 * the menu, and a hold that moves drags the host to another place or group of the same vault.
 */
@Composable
fun HostsScreen() {
    var query by remember { mutableStateOf("") }
    /** The host editor: a host's id, or "" for a new one. */
    var editing by remember { mutableStateOf<String?>(null) }
    var deleted by remember { mutableStateOf<Deleted?>(null) }
    var ownLogin by remember { mutableStateOf<Host?>(null) }
    var folderTeam by remember { mutableStateOf<PendingTeam?>(null) }
    var joining by remember { mutableStateOf(false) }
    var naming by remember { mutableStateOf<Naming?>(null) }
    var addMenu by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val list = rememberLazyListState()
    val drag = remember { HostDrag() }
    var box by remember { mutableStateOf<LayoutCoordinates?>(null) }
    val density = LocalDensity.current

    // A join link opened the app: the form, filled in from it.
    LaunchedEffect(AppModel.pendingJoin) {
        if (AppModel.pendingJoin == null) return@LaunchedEffect
        if (AppModel.sync?.enabled == true) AppModel.pendingJoin = null else joining = true
    }

    val vaults = AppModel.vaults
    val vault: VaultHosts? = vaults.firstOrNull { it.id == AppModel.hostsVault } ?: vaults.firstOrNull()
    val mine = vault != null && vault.team && vault.id in AppModel.mineViews && vault.layout != null
    // Everyone arranges their own grouping; the shared one is for those who manage it.
    val canArrange = vault != null && (mine || vault.canManage)
    // The bar's button is drawn by the pinned title bar: it reads the current state, not the one it was made with.
    val arrangeNow by androidx.compose.runtime.rememberUpdatedState(canArrange)
    val q = query.trim().lowercase()
    val hosts = vault?.hosts?.let { all -> if (q.isEmpty()) all else all.filter { "${it.label} ${it.address} ${it.login} ${it.group}".lowercase().contains(q) } } ?: emptyList()
    val groups: List<ListedGroup> = when {
        vault == null -> emptyList()
        mine -> Groups.listMine(hosts, vault.layout!!, vault.groups, q.isEmpty() && canArrange)
        else -> Groups.list(hosts, vault.groups, q.isEmpty() && canArrange)
    }
    // While dragging, hosts can also go out of every group.
    val noneKey = if (mine) Groups.MINE_NONE else Groups.sharedKey("")
    val shown = if (drag.active && groups.none { it.key == noneKey }) groups + ListedGroup(noneKey, "", hosts = emptyList()) else groups
    val named = shown.any { it.name.isNotEmpty() }

    fun deleteHost(h: Host) = scope.launch {
        val removed = runCatching { VaultData.remove(h.vaultId, "hosts", h.id) }.onFailure { Hud.error(AppModel.describe(it)) }.getOrNull() ?: return@launch
        AppModel.reload()
        deleted = Deleted.HostItem(h.vaultId, removed, h.label)
    }

    fun deleteGroup(g: ListedGroup) {
        val v = vault ?: return
        scope.launch {
            if (g.mine != null) {
                val before = v.layout ?: return@launch
                if (runCatching { Groups.saveLayout(v.id, Groups.mineRemove(before, g.mine.id)) }.onFailure { Hud.error(AppModel.describe(it)) }.isFailure) return@launch
                deleted = Deleted.MineGroup(v.id, before, g.name)
            } else {
                val (record, members) = runCatching { Groups.removeGroup(v.id, g.name) }.onFailure { Hud.error(AppModel.describe(it)) }.getOrNull() ?: return@launch
                deleted = Deleted.SharedGroup(v.id, g.name, record, members)
            }
            AppModel.reload()
        }
    }

    /** Puts the dragged host where it was dropped: shown at once, saved behind. */
    fun drop() {
        val v = vault ?: return
        val (h, to) = drag.end() ?: return
        fun show(next: VaultHosts) { AppModel.vaults = AppModel.vaults.map { if (it.id == v.id) next else it } }
        scope.launch {
            runCatching {
                if (mine) {
                    val layout = Groups.mineMoveHost(v.layout!!, groups, h.id, to.group, to.before)
                    show(v.copy(layout = layout))
                    Groups.saveLayout(v.id, layout)
                } else {
                    val changes = Groups.moved(v.hosts, h.id, shown.first { it.key == to.group }.name, to.before)
                    show(v.copy(hosts = Groups.applied(v.hosts, changes)))
                    Groups.moveHosts(v.id, changes)
                }
            }.onFailure { Hud.error(AppModel.describe(it)) }
            AppModel.reload()
        }
    }

    LaunchedEffect(deleted) {
        if (deleted != null) { delay(6000); deleted = null }
    }

    // Near the top or the bottom while dragging, the list scrolls by itself.
    LaunchedEffect(drag.active) {
        if (!drag.active) return@LaunchedEffect
        val edge = with(density) { 72.dp.toPx() }
        while (drag.active) {
            withFrameNanos { }
            val b = box ?: continue
            val top = b.positionInRoot().y
            val bottom = top + b.size.height
            val y = drag.pointer.y
            val dy = when {
                y < top + edge -> -(top + edge - y) / 5f
                y > bottom - edge -> (y - (bottom - edge)) / 5f
                else -> 0f
            }
            if (dy != 0f) {
                list.scrollBy(dy.coerceIn(-28f, 28f))
                drag.retarget()
            }
        }
    }

    Box(
        Modifier.fillMaxSize().onGloballyPositioned { box = it }
            .followDrag(drag, { box }, { drag.moveTo(it, vault?.id ?: "") }, { drop() })
    ) {
        LazyColumn(Modifier.fillMaxSize(), state = list) {
            item {
                LargeTitle(t("Hosts")) {
                    Box {
                        BarIcon(Icons.Outlined.Add, t("New host")) { if (arrangeNow) addMenu = true else editing = "" }
                        DropdownMenu(addMenu, { addMenu = false }) {
                            DropdownMenuItem({ Text(t("New host")) }, { addMenu = false; editing = "" }, leadingIcon = { Icon(Icons.Outlined.Dns, null) })
                            DropdownMenuItem({ Text(t("New group")) }, { addMenu = false; naming = Naming(null) }, leadingIcon = { Icon(Icons.Outlined.CreateNewFolder, null) })
                        }
                    }
                }
                SearchField(query) { query = it }
                if (vaults.size > 1 && vault != null) VaultChips(vaults, vault, drag)
                AnimatedVisibility(vault?.team == true, enter = expandVertically() + fadeIn(), exit = shrinkVertically() + fadeOut()) {
                    Box(Modifier.padding(horizontal = 16.dp).padding(top = 8.dp)) {
                        Segmented(listOf(false to t("Shared groups"), true to t("My groups")), mine) { on -> vault?.let { AppModel.setMineView(it, on) } }
                    }
                }
            }
            if (q.isEmpty()) driveNeedsSection { folderTeam = it }
            if (vault == null || (vault.hosts.isEmpty() && shown.isEmpty())) {
                if (AppModel.driveSignIns.isEmpty() && AppModel.folderTeams.isEmpty()) {
                    item { EmptyState(Icons.Outlined.Dns, t("No hosts"), t("New host")) { editing = "" } }
                }
            } else if (shown.isEmpty()) {
                item { EmptyState(Icons.Outlined.Dns, t("No results")) }
            }
            for (g in shown) {
                groupSection(
                    g, drag,
                    title = g.name.ifEmpty { if (named) t("No group") else null },
                    rename = if (canArrange && g.name.isNotEmpty()) ({ naming = Naming(g) }) else null,
                    delete = { deleteGroup(g) },
                ) {
                    if (g.hosts.isEmpty()) {
                        Box(Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(16.dp, 10.dp), contentAlignment = Alignment.CenterStart) {
                            Text(t("Drag hosts here"), color = colors.secondary, fontSize = 15.sp)
                        }
                    }
                    g.hosts.forEachIndexed { i, h ->
                        DropGap(drag, g.key, h.id)
                        HostRow(
                            h, g.key, drag, last = i == g.hosts.lastIndex,
                            canDrag = canArrange && (mine || h.editable) && q.isEmpty(),
                            edit = { editing = h.id }, delete = { deleteHost(h) }, ownLogin = { ownLogin = h },
                        )
                    }
                    DropGap(drag, g.key, null)
                }
            }
            item { Spacer(Modifier.height(80.dp)) }
        }
        TitleBar()

        // The row being dragged, under the finger.
        val origin = box?.positionInRoot() ?: Offset.Zero
        drag.host?.let { h ->
            val lift by animateFloatAsState(if (drag.active) 1.03f else 1f, label = "lift")
            Box(
                Modifier.offset { IntOffset((drag.left - origin.x).roundToInt(), (drag.pointer.y - drag.grab.y - origin.y).roundToInt()) }
                    .width(with(density) { drag.size.width.toDp() }).height(with(density) { drag.size.height.toDp() })
                    .graphicsLayer { scaleX = lift; scaleY = lift }
                    .shadow(10.dp, RoundedCornerShape(10.dp)).clip(RoundedCornerShape(10.dp)).background(colors.cell)
                    .alpha(if (drag.overVault != null) 0.6f else 1f)
            ) {
                Row(Modifier.fillMaxSize().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) { HostText(h) }
            }
        }

        // A host dragged onto another vault: it doesn't go there, said where the finger doesn't cover it.
        AnimatedVisibility(drag.overVault != null, Modifier.align(Alignment.BottomCenter), enter = slideInVertically { it } + fadeIn(), exit = slideOutVertically { it } + fadeOut()) {
            Row(
                Modifier.padding(16.dp).shadow(6.dp, RoundedCornerShape(14.dp)).clip(RoundedCornerShape(14.dp)).background(colors.cell).padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Outlined.Lock, null, tint = colors.secondary, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(10.dp))
                Text(t("Can't move hosts between vaults"), color = colors.label, fontSize = 15.sp)
            }
        }
        UndoBar(deleted?.let { t("Deleted %@", it.label) }, Modifier.align(Alignment.BottomCenter)) {
            val d = deleted ?: return@UndoBar
            deleted = null
            scope.launch {
                runCatching {
                    when (d) {
                        is Deleted.HostItem -> VaultData.save(d.vault, "hosts", d.host)
                        is Deleted.SharedGroup -> Groups.restoreGroup(d.vault, d.name, d.record, d.members)
                        is Deleted.MineGroup -> Groups.saveLayout(d.vault, d.layout)
                    }
                }.onFailure { Hud.error(AppModel.describe(it)) }
                AppModel.reload()
            }
        }
    }
    editing?.let { id -> HostEditor(id.ifEmpty { null }, vault?.id ?: VaultData.PERSONAL) { editing = null } }
    ownLogin?.let { h -> OwnLoginEditor(h) { ownLogin = null } }
    folderTeam?.let { t -> TeamFromDriveSheet(t) { folderTeam = null } }
    if (joining) SyncJoinSheet { joining = false }
    naming?.let { n ->
        GroupNameDialog(n.from?.name ?: "", close = { naming = null }) { name ->
            val v = vault ?: return@GroupNameDialog
            val from = n.from
            naming = null
            scope.launch {
                runCatching {
                    when {
                        mine && from?.mine != null -> Groups.saveLayout(v.id, Groups.mineRename(v.layout!!, from.mine.id, name))
                        mine -> Groups.saveLayout(v.id, Groups.mineAdd(v.layout!!, name))
                        from != null -> Groups.renameGroup(v.id, from.name, name)
                        else -> Groups.addGroup(v.id, name)
                    }
                }.onFailure { Hud.error(AppModel.describe(it)) }
                AppModel.reload()
            }
        }
    }
}

/** The vaults to show hosts from: the personal one and the teams. While a host is dragged, the others dim and lock. */
@Composable
private fun VaultChips(vaults: List<VaultHosts>, current: VaultHosts, drag: HostDrag) {
    Column {
        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(top = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            for (v in vaults) {
                val selected = v.id == current.id
                val bg by animateColorAsState(if (selected) colors.accent else colors.fill, label = "chip")
                val dim by animateFloatAsState(if (drag.active && !selected) 0.35f else 1f, label = "dim")
                Row(
                    Modifier.onGloballyPositioned { drag.vault(v.id, it.boundsInRoot()) }
                        .alpha(dim).clip(RoundedCornerShape(50)).background(bg)
                        .clickable(remember { MutableInteractionSource() }, null, enabled = !drag.active) { AppModel.showVault(v.id) }
                        .padding(horizontal = 14.dp, vertical = 7.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (drag.active && !selected) {
                        Icon(Icons.Outlined.Lock, null, tint = colors.label, modifier = Modifier.size(14.dp))
                        Spacer(Modifier.width(5.dp))
                    }
                    Text(v.name ?: t("Personal"), color = if (selected) Color.White else colors.label, fontSize = 15.sp, fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Medium, maxLines = 1)
                }
            }
        }
    }
}

/** A group of hosts: its header (a hold opens its menu, when it can be renamed) and its rows. */
private fun LazyListScope.groupSection(g: ListedGroup, drag: HostDrag, title: String?, rename: (() -> Unit)?, delete: () -> Unit, rows: @Composable () -> Unit) {
    item(key = g.key) {
        var top by remember { mutableStateOf(0f) }
        var open by remember { mutableStateOf(false) }
        DisposableEffect(g.key) { onDispose { drag.sectionGone(g.key) } }
        Column(Modifier.animateItem().padding(horizontal = 16.dp).padding(top = if (title != null) 18.dp else 22.dp).onGloballyPositioned { top = it.positionInRoot().y }) {
            if (title != null) {
                Box {
                    val interaction = remember { MutableInteractionSource() }
                    Text(
                        title.uppercase(), color = colors.secondary, fontSize = 13.sp,
                        modifier = Modifier.fillMaxWidth()
                            .then(if (rename != null) Modifier.tapHoldDrag(interaction, onClick = {}, onHold = { open = true }, drag = null) else Modifier)
                            .padding(start = 16.dp, bottom = 6.dp, top = 4.dp),
                    )
                    if (rename != null) DropdownMenu(open, { open = false }) {
                        DropdownMenuItem({ Text(t("Rename")) }, { open = false; rename() }, leadingIcon = menuIcon("Rename"))
                        DropdownMenuItem({ Text(t("Delete"), color = colors.red) }, { open = false; delete() }, leadingIcon = menuIcon("Delete"))
                    }
                }
            }
            Column(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(colors.cell)
                    .onGloballyPositioned { drag.section(g.key, it.boundsInRoot(), g.hosts.map { h -> h.id }, top) }
            ) { rows() }
        }
    }
}

/** Where the dragged host would land opens up: before the row `before` (or at the end, null) of a group. */
@Composable
private fun DropGap(drag: HostDrag, group: String, before: String?) {
    val density = LocalDensity.current
    val to = drag.target
    val open = to != null && to.group == group && to.before == before
    val height by androidx.compose.animation.core.animateDpAsState(
        if (open) with(density) { drag.size.height.toDp() } else 0.dp,
        androidx.compose.animation.core.spring(stiffness = androidx.compose.animation.core.Spring.StiffnessMediumLow), label = "gap",
    )
    if (height > 0.dp) Spacer(Modifier.fillMaxWidth().height(height))
}

@Composable
private fun RowScope.HostText(host: Host) {
    CellText(host.label, "${host.login}@${host.address}${if (host.port == 22) "" else ":${host.port}"}", mono = true)
}

@Composable
private fun HostRow(host: Host, group: String, drag: HostDrag, last: Boolean, canDrag: Boolean, edit: () -> Unit, delete: () -> Unit, ownLogin: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    val interaction = remember { MutableInteractionSource() }
    val highlight = rememberTapHighlight(interaction)
    val dragged = drag.host?.id == host.id
    DisposableEffect(host.id) { onDispose { drag.rowGone(host.id) } }
    Box(Modifier.onGloballyPositioned { drag.row(host.id, it.boundsInRoot()) }.alpha(if (dragged) 0.3f else 1f)) {
        SwipeActions(if (host.editable) listOf(deleteAction(delete), editAction(edit)) else emptyList()) {
            Column(Modifier.background(colors.fill.copy(alpha = colors.fill.alpha * highlight.value))) {
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 44.dp)
                        .tapHoldDrag(
                            interaction,
                            onClick = { AppModel.open(host) },
                            onHold = { menu = true },
                            drag = if (canDrag) ({ at -> menu = false; drag.start(host, group, at) }) else null,
                        )
                        .padding(horizontal = 16.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) { HostText(host) }
                if (!last) HorizontalDivider(Modifier.padding(start = 16.dp), thickness = 0.5.dp, color = colors.separator)
            }
        }
        DropdownMenu(menu, { menu = false }) {
            DropdownMenuItem(leadingIcon = menuIcon("Connect"), text = { Text(t("Connect")) }, onClick = { menu = false; AppModel.open(host) })
            DropdownMenuItem(leadingIcon = menuIcon("Files"), text = { Text(t("Files")) }, onClick = { menu = false; AppModel.openFiles(host) })
            DropdownMenuItem(leadingIcon = menuIcon("Copy ssh command"), text = { Text(t("Copy ssh command")) }, onClick = {
                menu = false
                Clipboard.copy("ssh " + (if (host.port == 22) "" else "-p ${host.port} ") + "${host.login}@${host.address}")
            })
            if (host.vaultId != VaultData.PERSONAL) {
                DropdownMenuItem(leadingIcon = menuIcon("My login"), text = { Text(t("My login")) }, onClick = { menu = false; ownLogin() })
            }
            if (host.editable) {
                DropdownMenuItem(leadingIcon = menuIcon("Edit"), text = { Text(t("Edit")) }, onClick = { menu = false; edit() })
                DropdownMenuItem(leadingIcon = menuIcon("Delete"), text = { Text(t("Delete"), color = colors.red) }, onClick = { menu = false; delete() })
            }
        }
    }
}

/** Names a new group or renames one. */
@Composable
private fun GroupNameDialog(initial: String, close: () -> Unit, done: (String) -> Unit) {
    var name by remember { mutableStateOf(initial) }
    fun finish() {
        val n = name.trim()
        if (n.isEmpty() || n == initial) close() else done(n)
    }
    AlertDialog(
        onDismissRequest = close,
        title = { Text(if (initial.isEmpty()) t("New group") else t("Rename")) },
        text = { Field(name, { name = it }, t("Name"), onDone = { finish() }) },
        confirmButton = { TextButton({ finish() }) { Text(if (initial.isEmpty()) t("Create") else t("Save")) } },
        dismissButton = { TextButton(close) { Text(t("Cancel")) } },
        containerColor = colors.background,
        titleContentColor = colors.label,
    )
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
