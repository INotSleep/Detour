package dev.detour.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Groups
import androidx.compose.material.icons.outlined.IosShare
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.MainTab
import dev.detour.android.core.Core
import dev.detour.android.core.DriveSignInNeed
import dev.detour.android.core.ErrorDetail
import dev.detour.android.core.PendingTeam
import dev.detour.android.core.StorageModel
import dev.detour.android.core.bool
import dev.detour.android.core.coreErrorText
import dev.detour.android.core.double
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.launch

/** A team on this device as `teams_list` shows it: opened (`view`), or why it couldn't be. */
data class TeamEntry(val id: String, val path: String, val view: TeamView?, val error: String?) {
    data class TeamView(
        val id: String,
        val name: String,
        /** Added by an admin: until then the team's contents can't be read. */
        val joined: Boolean,
        val members: List<Member>,
        val hosts: Int,
        val remote: Remote?,
    ) {
        companion object {
            fun from(m: Map<String, Any?>) = TeamView(
                m.str("id") ?: "", m.str("name") ?: "", m.bool("joined") ?: false,
                m["members"].list().map { it.obj() }.map { Member(it.str("id") ?: "", it.str("name") ?: "", it.str("role") ?: "member", it.str("fingerprint") ?: "", it.bool("isYou") ?: false) },
                m["data"].obj()["hosts"].list().size,
                m["remote"]?.obj()?.let { Remote(it.str("url") ?: "", it.str("kind") ?: "", it.bool("pending") ?: false, ErrorDetail.from(it["error"]), it.double("syncedAt")) },
            )
        }
    }

    /** `role`: `owner`, `admin` or `member`. */
    data class Member(val id: String, val name: String, val role: String, val fingerprint: String, val isYou: Boolean)

    /** Where the team is kept, how syncing it goes, and why reaching it failed. */
    data class Remote(val url: String, val kind: String, val pending: Boolean, val error: ErrorDetail?, val syncedAt: Double?)

    companion object {
        fun from(m: Map<String, Any?>) = TeamEntry(m.str("id") ?: "", m.str("path") ?: "", m["view"]?.obj()?.let { TeamView.from(it) }, m.str("error"))
    }
}

/** This person's identity: what an admin adds to a team. */
data class MyId(val detourId: String, val fingerprint: String)

private suspend fun loadTeams(): List<TeamEntry> = runCatching { Core.call("teams_list").list().map { TeamEntry.from(it.obj()) } }.getOrDefault(emptyList())

/** This person's teams on this phone, their Detour ID for an admin to add, and joining one. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TeamsScreen() {
    var teams by remember { mutableStateOf<List<TeamEntry>>(emptyList()) }
    var me by remember { mutableStateOf<MyId?>(null) }
    var joining by remember { mutableStateOf(false) }
    var copies by remember { mutableStateOf(0) }
    var folderTeam by remember { mutableStateOf<PendingTeam?>(null) }
    var opened by remember { mutableStateOf<String?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val active = AppModel.tab == MainTab.Teams

    suspend fun load() {
        me = runCatching { Core.call("my_id").obj() }.getOrNull()?.let { MyId(it.str("detourId") ?: "", it.str("fingerprint") ?: "") }
        teams = loadTeams()
    }

    LaunchedEffect(active, AppModel.teamsVersion) { if (active) load() }

    Box(Modifier.fillMaxSize()) {
        PullToRefreshBox(refreshing, { scope.launch { refreshing = true; AppModel.syncNow(); load(); refreshing = false } }, Modifier.fillMaxSize()) {
            LazyColumn(Modifier.fillMaxSize()) {
                item { LargeTitle(t("Teams")) { BarIcon(Icons.Outlined.Add, t("Join a team")) { joining = true } } }
                me?.let { id ->
                    section(t("My Detour ID"), key = "me") {
                        Cell(onClick = { Clipboard.copy(id.detourId); copies++ }) {
                            Text(id.detourId, color = colors.label, fontSize = 13.sp, fontFamily = FontFamily.Monospace, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                            Spacer(Modifier.width(10.dp))
                            CopyIcon(copies > 0)
                        }
                        Cell {
                            Text(t("Fingerprint"), color = colors.label, fontSize = 13.sp, fontFamily = FontFamily.Monospace)
                            Spacer(Modifier.width(12.dp))
                            Text(id.fingerprint, color = colors.secondary, fontSize = 13.sp, fontFamily = FontFamily.Monospace, maxLines = 1, overflow = TextOverflow.MiddleEllipsis, modifier = Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.End)
                        }
                        ButtonCell(t("Send"), icon = Icons.Outlined.IosShare, last = true) { share(context, id.detourId) }
                    }
                }
                val folders = AppModel.folderTeams
                if (teams.isNotEmpty() || folders.isNotEmpty()) {
                    section(t("Teams"), key = "teams") {
                        teams.forEachIndexed { i, entry ->
                            val last = i == teams.lastIndex && folders.isEmpty()
                            val v = entry.view
                            if (v != null) {
                                Cell(last = last, onClick = { opened = v.id }) { TeamRow(v) }
                            } else {
                                Cell(last = last) {
                                    Column(Modifier.weight(1f)) {
                                        Text(entry.path, color = colors.label, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
                                        entry.error?.let { Text(it, color = colors.red, fontSize = 13.sp) }
                                    }
                                }
                            }
                        }
                        folders.forEachIndexed { i, t -> FolderTeamRow(t, i == folders.lastIndex) { folderTeam = t } }
                    }
                }
                if (teams.isEmpty() && folders.isEmpty() && me != null) {
                    item { EmptyState(Icons.Outlined.Groups, t("No teams"), t("Join a team")) { joining = true } }
                }
                item { Spacer(Modifier.height(40.dp)) }
            }
            TitleBar()
        }
        BackHandler(active && opened != null) { opened = null }
        Pushed(opened) { id ->
            teams.firstNotNullOfOrNull { it.view?.takeIf { v -> v.id == id } }?.let { v ->
                TeamDetailPage(v, reload = { teams = loadTeams() }) { opened = null }
            }
        }
    }
    if (joining) JoinTeamSheet { joining = false; scope.launch { load() } }
    folderTeam?.let { t -> TeamFromDriveSheet(t) { folderTeam = null; scope.launch { load() } } }
}

@Composable
private fun androidx.compose.foundation.layout.RowScope.TeamRow(team: TeamEntry.TeamView) {
    Column(Modifier.weight(1f)) {
        Text(team.name, color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (team.joined) Text(tCount("%lld members · %lld hosts", team.members.size, team.hosts), color = colors.secondary, fontSize = 13.sp)
        else Text(t("Waiting for an admin to add your Detour ID"), color = colors.orange, fontSize = 13.sp)
    }
    androidx.compose.material3.Icon(Icons.Outlined.ChevronRight, null, tint = colors.tertiary, modifier = Modifier.size(20.dp))
}

private fun roleName(role: String) = when (role) {
    "owner" -> t("Owner")
    "admin" -> t("Admin")
    else -> t("Member")
}

/** A team: where it is kept, how syncing it goes, its members, removing it from the devices. */
@Composable
private fun TeamDetailPage(team: TeamEntry.TeamView, reload: suspend () -> Unit, close: () -> Unit) {
    var current by remember { mutableStateOf<TeamEntry.TeamView?>(null) }
    var syncing by remember { mutableStateOf(false) }
    var confirmRemove by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val tv = current ?: team

    suspend fun sync() {
        syncing = true
        error = null
        try {
            current = TeamEntry.TeamView.from(Core.call("team_sync", mapOf("id" to tv.id)).obj())
        } catch (e: Exception) {
            error = coreErrorText(e)
        }
        syncing = false
        AppModel.reload()
        reload()
    }

    fun remove() = scope.launch {
        try {
            Core.call("team_leave", mapOf("id" to tv.id))
            AppModel.reload()
            reload()
            close()
        } catch (e: Exception) {
            error = coreErrorText(e)
        }
    }

    PushedPage(tv.name, t("Teams"), close, refresh = { sync() }) {
        tv.remote?.let { r ->
            section(key = "remote") {
                Cell {
                    Column(Modifier.weight(1f)) {
                        Text(r.url, color = colors.label, fontSize = 15.sp, fontFamily = FontFamily.Monospace, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        Text(
                            syncStatusText(r.pending, r.syncedAt, r.error, syncing), fontSize = 13.sp,
                            color = if (r.error != null) colors.red else if (r.pending) colors.orange else colors.secondary,
                        )
                    }
                }
                val e = r.error
                if (e?.kind == "driveSignIn" && e.provider != null) {
                    DriveSignInAgainRows(DriveSignInNeed(e.provider, e.account), last = true) { scope.launch { sync() } }
                } else {
                    ButtonCell(t("Sync now"), busy = syncing, last = true) { scope.launch { sync() } }
                }
            }
        }
        if (!tv.joined) {
            section(key = "waiting") {
                Cell(last = true) { Text(t("Waiting for an admin to add your Detour ID"), color = colors.orange, fontSize = 17.sp) }
            }
        }
        section(t("Members"), key = "members") {
            tv.members.forEachIndexed { i, m ->
                Cell(last = i == tv.members.lastIndex) {
                    Column(Modifier.weight(1f)) {
                        Text(if (m.isYou) t("%@ (you)", m.name) else m.name, color = colors.label, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(m.fingerprint, color = colors.secondary, fontSize = 12.sp, fontFamily = FontFamily.Monospace, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
                    }
                    Spacer(Modifier.width(8.dp))
                    Text(roleName(m.role), color = colors.secondary, fontSize = 13.sp)
                }
            }
        }
        section(key = "remove") {
            ButtonCell(t("Remove from all devices"), icon = Icons.Outlined.Delete, destructive = true, last = true) { confirmRemove = true }
        }
        errorItem(error)
    }
    if (confirmRemove) {
        Confirm(
            t("Remove \"%@\" from all your devices?", tv.name),
            t("The team vault and your membership stay, you can reconnect anytime."),
            t("Remove"), { confirmRemove = false },
        ) { remove() }
    }
}

/**
 * Joining a team an admin added this person to: its repository or drive folder, the creator's
 * fingerprint to compare, then joined.
 */
@Composable
fun JoinTeamSheet(close: () -> Unit) {
    data class Found(val root: String, val rootFingerprint: String, val clone: String)

    val storage = remember { StorageModel() }
    var found by remember { mutableStateOf<Found?>(null) }
    var me by remember { mutableStateOf<MyId?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        me = runCatching { Core.call("my_id").obj() }.getOrNull()?.let { MyId(it.str("detourId") ?: "", it.str("fingerprint") ?: "") }
    }
    LaunchedEffect(storage.kind) { found = null; error = null }
    LaunchedEffect(storage.url, storage.drive.folder) { found = null }

    fun submit() {
        if (!storage.ready || busy) return
        busy = true
        error = null
        val drive = storage.kind == StorageModel.Kind.Drive
        scope.launch {
            try {
                val f = found
                if (f != null) {
                    if (drive) Core.call("team_join_drive", mapOf("clone" to f.clone, "root" to f.root, "drive" to storage.driveJson))
                    else Core.call("team_join_git", storage.git + mapOf("clone" to f.clone, "root" to f.root))
                    AppModel.reload()
                    close()
                } else {
                    val r = (if (drive) Core.call("team_inspect_drive", mapOf("drive" to storage.driveJson)) else Core.call("team_inspect_git", storage.git)).obj()
                    val info = r["info"].obj()
                    found = Found(info.str("root") ?: "", info.str("rootFingerprint") ?: "", r.str("clone") ?: "")
                }
            } catch (e: Exception) {
                error = coreErrorText(e, folder = drive)
            }
            busy = false
        }
    }

    Sheet(close) {
        SheetBar(
            t("Join team"),
            left = { BarButton(t("Cancel"), onClick = close) },
            right = {
                if (busy) CircularProgressIndicator(Modifier.size(22.dp), color = colors.accent)
                else BarButton(if (found == null) t("Next") else t("Trust and join"), bold = true, enabled = storage.ready) { submit() }
            },
        )
        LazyColumn(Modifier.fillMaxSize()) {
            storageSections(storage, "team")
            found?.let { f ->
                section(footer = t("Compare the fingerprint with the team's creator"), key = "found") {
                    Cell(last = me == null) {
                        Text(t("Vault creator"), color = colors.label, fontSize = 15.sp)
                        Spacer(Modifier.width(12.dp))
                        Text(f.rootFingerprint, color = colors.secondary, fontSize = 15.sp, fontFamily = FontFamily.Monospace, modifier = Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.End)
                    }
                    me?.let { id ->
                        Cell(last = true) {
                            Text(t("Your fingerprint"), color = colors.label, fontSize = 13.sp)
                            Spacer(Modifier.width(12.dp))
                            Text(id.fingerprint, color = colors.secondary, fontSize = 13.sp, fontFamily = FontFamily.Monospace, modifier = Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.End)
                        }
                    }
                }
            }
            errorItem(error)
            item { Spacer(Modifier.padding(bottom = 40.dp)) }
        }
    }
}
