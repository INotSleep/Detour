package dev.detour.android

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.detour.android.core.Biometry
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.DriveFolderRef
import dev.detour.android.core.DriveSignInNeed
import dev.detour.android.core.Drives
import dev.detour.android.core.ErrorDetail
import dev.detour.android.core.PendingTeam
import dev.detour.android.core.Groups
import dev.detour.android.core.Host
import dev.detour.android.core.HostGroup
import dev.detour.android.core.MyGroup
import dev.detour.android.core.VaultData
import dev.detour.android.core.VaultHosts
import dev.detour.android.core.HostSection
import dev.detour.android.core.SyncStatus
import dev.detour.android.core.bool
import dev.detour.android.core.double
import dev.detour.android.core.t
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.terminal.TerminalSession
import dev.detour.android.ui.JoinLink
import dev.detour.android.files.FilesSession
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import java.util.UUID

/** An open session: a terminal or a host's files over SFTP. */
sealed interface OpenSession {
    val id: UUID
    val host: Host

    class Terminal(val session: TerminalSession) : OpenSession {
        override val id get() = session.id
        override val host get() = session.host
    }

    class Files(val files: FilesSession) : OpenSession {
        override val id get() = files.id
        override val host get() = files.host
    }
}

/** A team being opened from a drive folder: how far the download is, or why it failed. */
class OpeningTeam {
    var done by mutableStateOf(0)
    var total by mutableStateOf(0)
    var error by mutableStateOf<String?>(null)
}

/** The app's tabs, in the order of the tab bar. */
enum class MainTab { Hosts, Sessions, Keys, Snippets, Proxies, Teams, Settings }

/** What the app shows and holds: the vault's state, its hosts, the open sessions. */
object AppModel {
    enum class Phase { Loading, Setup, Locked, Ready }

    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    var phase by mutableStateOf(Phase.Loading)
    var sections by mutableStateOf<List<HostSection>>(emptyList())
    /** The personal vault and the teams, with their hosts, groups and this person's own grouping. */
    var vaults by mutableStateOf<List<VaultHosts>>(emptyList())
    /** The vault the hosts screen shows: new hosts go there. */
    var hostsVault by mutableStateOf(prefs?.getString("hostsVault", null) ?: VaultData.PERSONAL)
        private set
    /** Teams whose hosts show in this person's own groups, not the shared ones. */
    var mineViews by mutableStateOf(prefs?.getStringSet("mineViews", null)?.toSet() ?: emptySet())
        private set

    private val prefs get() = if (KeepAliveService.ready) KeepAliveService.context.getSharedPreferences("app", android.content.Context.MODE_PRIVATE) else null

    fun showVault(id: String) {
        hostsVault = id
        prefs?.edit()?.putString("hostsVault", id)?.apply()
    }

    /** Shows a team's hosts in this person's own groups (made from the shared ones the first time) or the shared ones. */
    fun setMineView(team: VaultHosts, on: Boolean) {
        mineViews = if (on) mineViews + team.id else mineViews - team.id
        prefs?.edit()?.putStringSet("mineViews", mineViews)?.apply()
        if (on && team.layout == null) scope.launch {
            runCatching { Groups.saveLayout(team.id, Groups.layoutFrom(team.hosts, team.groups)) }.onFailure { dev.detour.android.ui.Hud.error(describe(it)) }
            reload()
        }
    }
    var sync by mutableStateOf<SyncStatus?>(null)
    val sessions = mutableStateListOf<OpenSession>()
    var current by mutableStateOf<UUID?>(null)
    var showTerminals by mutableStateOf(false)
    var tab by mutableStateOf(MainTab.Hosts)
    /** Drives to sign in to again: the vault's sync or a team is kept there. */
    var driveSignIns by mutableStateOf<List<DriveSignInNeed>>(emptyList())
    /** Teams on this person's other devices kept in a folder: opened here through a drive. */
    var folderTeams by mutableStateOf<List<PendingTeam>>(emptyList())
    /** The sync's drive folder, when it syncs through a cloud drive. */
    var syncDrive by mutableStateOf<DriveFolderRef?>(null)
    /** Why the last sync failed, with the drive or the server key it was about. */
    var syncError by mutableStateOf<ErrorDetail?>(null)
    /** Offer the fingerprint once, after the first unlock with the password. */
    var offerBiometry by mutableStateOf(false)
    /** A `detour://join` link (the other device's QR) waiting for the join form. */
    var pendingJoin by mutableStateOf<JoinLink?>(null)
    private var lastPassword: String? = null
    /** Teams being opened from a drive folder, by team id: shown with their progress in the lists. */
    val openingTeams = mutableStateMapOf<String, OpeningTeam>()
    /** Bumped when a team joined here: the teams screen reads its list again. */
    var teamsVersion by mutableStateOf(0)

    val terminals: List<TerminalSession> get() = sessions.mapNotNull { (it as? OpenSession.Terminal)?.session }

    private var started = false

    fun start() {
        if (started) return
        started = true
        Core.onEvent { name, data ->
            when (name) {
                "locked" -> phase = Phase.Locked
                "data.changed", "unlocked" -> if (phase == Phase.Ready) scope.launch { reload() }
                "drive.progress" -> openingTeams.values.filter { it.error == null }.forEach {
                    it.done = data.double("done")?.toInt() ?: 0
                    it.total = data.double("total")?.toInt() ?: 0
                }
            }
        }
        scope.launch {
            try {
                val st = Core.call("vault_status").obj()
                phase = when {
                    st.bool("initialized") != true -> Phase.Setup
                    st.bool("unlocked") != true -> Phase.Locked
                    else -> { ready(); Phase.Ready }
                }
            } catch (_: Exception) {
                phase = Phase.Locked
            }
        }
    }

    suspend fun create(password: String) {
        Core.call("vault_create", mapOf("masterPassword" to password))
        lastPassword = password
        ready()
        offerBiometry = Biometry.available
    }

    /** `typed`: the person typed it (not the one saved behind the fingerprint). */
    suspend fun unlock(password: String, typed: Boolean = true) {
        Core.call("vault_unlock", mapOf("masterPassword" to password))
        if (typed) {
            lastPassword = password
            // A password changed on another device: the saved one is replaced on its next use.
            if (Biometry.isSaved) Biometry.save(password) else offerBiometry = Biometry.available
        }
        ready()
    }

    fun rememberWithBiometry(on: Boolean) {
        val pw = lastPassword
        if (on && pw != null) Biometry.save(pw) else if (!on) Biometry.delete()
        lastPassword = null
        offerBiometry = false
    }

    suspend fun lock() {
        runCatching { Core.call("vault_lock") }
        phase = Phase.Locked
    }

    private suspend fun ready() {
        phase = Phase.Ready
        reload()
    }

    /** Changes shown before the core has them: a reload begun before one is saved reads what was there before. */
    private var saving = 0
    private var shownAt = 0

    /** Shows a vault changed here at once, while `save` keeps it. */
    suspend fun showWhileSaving(next: VaultHosts, save: suspend () -> Unit) {
        vaults = vaults.map { if (it.id == next.id) next else it }
        shownAt++
        saving++
        try { save() } finally { saving-- }
    }

    private var reloading = false
    private var reloadAgain = false

    /** Reads everything again. One at a time: asked for meanwhile, it runs once more after. */
    suspend fun reload() {
        if (reloading) { reloadAgain = true; return }
        reloading = true
        try {
            do {
                reloadAgain = false
                load()
            } while (reloadAgain)
        } finally {
            reloading = false
        }
    }

    private suspend fun load() {
        val began = shownAt
        val all = ArrayList<VaultHosts>()
        runCatching { Core.call("vault_get").obj() }.getOrNull()?.let { v ->
            all += VaultHosts(VaultData.PERSONAL, null, null, v["hosts"].list().map { Host.from(it.obj(), VaultData.PERSONAL) }, v["groups"].list().map { HostGroup.from(it.obj()) }, null)
        }
        val signIns = ArrayList<ErrorDetail?>()
        runCatching { Core.call("teams_list").list() }.getOrNull()?.forEach { t ->
            val view = t.obj()["view"]?.obj() ?: return@forEach
            val id = view.str("id") ?: return@forEach
            val me = view["members"].list().map { it.obj() }.firstOrNull { it.bool("isYou") == true }
            val levels = view["access"].list().map { it.obj() }.filter { it.str("member") == me?.str("id") }.associate { (it.str("record") ?: "") to (it.str("level") ?: "none") }
            val data = view["data"].obj()
            all += VaultHosts(
                id, view.str("name") ?: "", me?.str("role"),
                data["hosts"].list().map { h -> h.obj().let { Host.from(it, id, levels[it.str("id")] ?: "none") } },
                data["groups"].list().map { HostGroup.from(it.obj()) },
                view["layout"]?.obj()?.let { l -> l["groups"].list().map { MyGroup.from(it.obj()) } },
            )
            signIns += ErrorDetail.from(view["remote"].obj()["error"])
        }
        // The user connecting goes with, as the core picks it: this person's settings of the host's
        // group of theirs, then of its shared group.
        val prefs = runCatching { Core.call("group_prefs").list().map { it.obj() } }.getOrDefault(emptyList())
            .associateBy { it.str("key") ?: "" }
        fun loginPrefs(key: String) = prefs[key]?.takeIf { it["user"] != null || it["auth"] != null }
        // Read before a change shown here was saved: the reload after saving it shows it.
        if (saving == 0 && began == shownAt) {
            vaults = all.map { v ->
                v.copy(hosts = v.hosts.map { h ->
                    val mine = v.layout?.firstOrNull { h.id in it.hosts }?.let { loginPrefs("${v.id}/mine:${it.id}") }
                    val login = mine ?: loginPrefs("${v.id}/team:${h.group}")
                    login?.str("user")?.takeIf { it.isNotEmpty() }?.let { h.copy(groupUser = it) } ?: h
                })
            }
            sections = vaults.flatMap { v ->
                Groups.list(v.hosts, v.groups, false).map { g ->
                    HostSection("${v.id}/${g.name}", listOfNotNull(v.name, g.name.ifEmpty { null }).joinToString(" / "), g.hosts)
                }
            }
        }
        val st = runCatching { Core.call("sync_status") }.getOrNull()
        sync = SyncStatus.from(st)
        syncDrive = st.obj()["drive"]?.obj()?.let { DriveFolderRef.from(it) }
        syncError = ErrorDetail.from(st.obj()["error"])
        signIns += syncError
        val needs = signIns.mapNotNull { e -> if (e?.kind == "driveSignIn") e.provider?.let { DriveSignInNeed(it, e.account) } else null }.toMutableList()
        val folders = ArrayList<PendingTeam>()
        if (sync?.enabled == true) {
            val pending = runCatching { Core.call("teams_pending").list().map { PendingTeam.from(it.obj()) } }.getOrDefault(emptyList())
            if (pending.isNotEmpty()) {
                val accounts = Drives.signedIn()
                for (t in pending) {
                    val d = t.drive
                    if (d != null) {
                        val here = accounts.any { it.provider == d.provider && (d.account == null || it.account == d.account) }
                        if (!here) needs += DriveSignInNeed(d.provider, d.account)
                    } else if (t.url == null) {
                        folders += t
                    }
                }
            }
        }
        driveSignIns = needs.distinct()
        folderTeams = folders
    }

    /** Signed in to a drive again: what is kept there syncs now, and teams in it join here. */
    suspend fun driveSignedIn() {
        if (sync?.enabled == true) runCatching { Core.call("sync_now") }
        runCatching { Core.call("teams_list").list() }.getOrNull()?.forEach { t ->
            val m = t.obj()
            if (m["view"].obj()["remote"].obj()["error"].obj().str("kind") == "driveSignIn") {
                runCatching { Core.call("team_sync", mapOf("id" to m.str("id"))) }
            }
        }
        reload()
    }

    /**
     * Opens here a team kept in a drive folder. It runs on its own: the list of teams shows how
     * far it is, and the sheet that started it is gone.
     */
    fun openTeamFromDrive(team: PendingTeam, folder: DriveFolderRef) {
        if (openingTeams[team.teamId]?.error == null && team.teamId in openingTeams) return
        val o = OpeningTeam()
        openingTeams[team.teamId] = o
        tab = MainTab.Teams
        scope.launch {
            try {
                KeepAliveService.whileBusy(t("Opening %@", team.name)) {
                    Core.call("team_join_linked_drive", mapOf("teamId" to team.teamId, "drive" to folder.json))
                }
                reload()
                openingTeams.remove(team.teamId)
                teamsVersion++
            } catch (e: Exception) {
                o.error = Drives.errorText(e)
            }
        }
    }

    /** Pulls what changed on other devices. */
    suspend fun syncNow() {
        if (sync?.enabled == true) runCatching { Core.call("sync_now") }
        reload()
    }

    // Sessions

    fun open(host: Host) {
        val t = TerminalSession(host)
        sessions.add(OpenSession.Terminal(t))
        current = t.id
        showTerminals = true
        t.connect()
        KeepAliveService.update()
    }

    /** A host's files, as a session of its own beside the terminals. */
    fun openFiles(host: Host) {
        val f = FilesSession(host)
        sessions.add(OpenSession.Files(f))
        current = f.id
        showTerminals = true
        scope.launch { f.open() }
        KeepAliveService.update()
    }

    fun show(s: OpenSession) {
        current = s.id
        showTerminals = true
    }

    fun close(s: OpenSession) {
        when (s) {
            is OpenSession.Terminal -> s.session.close()
            is OpenSession.Files -> s.files.close()
        }
        sessions.removeAll { it.id == s.id }
        if (current == s.id) current = sessions.lastOrNull()?.id
        if (sessions.isEmpty()) showTerminals = false
        KeepAliveService.update()
    }

    /** Back in front, or on another network: sessions whose connection went connect again. */
    fun resumed() {
        if (phase != Phase.Ready) return
        terminals.forEach { it.checkConnection() }
    }

    /** The app was opened with a `detour://join` link. */
    fun openLink(url: String) {
        JoinLink.parse(url)?.let { pendingJoin = it }
    }

    fun describe(e: Throwable): String = when (e) {
        is CoreFailure -> e["message"] ?: e.kind
        else -> e.message ?: e.toString()
    }
}
