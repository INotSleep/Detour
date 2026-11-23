package dev.detour.android

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
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
import dev.detour.android.core.Host
import dev.detour.android.core.HostSection
import dev.detour.android.core.SyncStatus
import dev.detour.android.core.bool
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
import java.text.Collator
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

/** The app's tabs, in the order of the tab bar. */
enum class MainTab { Hosts, Sessions, Keys, Snippets, Proxies, Teams, Settings }

/** What the app shows and holds: the vault's state, its hosts, the open sessions. */
object AppModel {
    enum class Phase { Loading, Setup, Locked, Ready }

    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    var phase by mutableStateOf(Phase.Loading)
    var sections by mutableStateOf<List<HostSection>>(emptyList())
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

    val terminals: List<TerminalSession> get() = sessions.mapNotNull { (it as? OpenSession.Terminal)?.session }

    private var started = false

    fun start() {
        if (started) return
        started = true
        Core.onEvent { name, _ ->
            when (name) {
                "locked" -> phase = Phase.Locked
                "data.changed", "unlocked" -> if (phase == Phase.Ready) scope.launch { reload() }
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

    suspend fun reload() {
        val out = ArrayList<HostSection>()
        runCatching { Core.call("vault_get").obj() }.getOrNull()?.let { v ->
            out += grouped(v["hosts"].list().map { Host.from(it.obj(), "personal") }, "personal", null)
        }
        val signIns = ArrayList<ErrorDetail?>()
        runCatching { Core.call("teams_list").list() }.getOrNull()?.forEach { t ->
            val view = t.obj()["view"]?.obj() ?: return@forEach
            val id = view.str("id") ?: return@forEach
            val hosts = view["data"].obj()["hosts"].list().map { Host.from(it.obj(), id) }
            out += grouped(hosts, id, view.str("name"))
            signIns += ErrorDetail.from(view["remote"].obj()["error"])
        }
        sections = out
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

    /** Pulls what changed on other devices. */
    suspend fun syncNow() {
        if (sync?.enabled == true) runCatching { Core.call("sync_now") }
        reload()
    }

    private val collator = Collator.getInstance().apply { strength = Collator.SECONDARY }

    /** As iOS's `localizedStandardCompare`: numbers within names by their value ("web-2" before "web-10"). */
    private val natural = Comparator<String> { a, b ->
        val parts = Regex("\\d+|\\D+")
        val x = parts.findAll(a).map { it.value }.toList()
        val y = parts.findAll(b).map { it.value }.toList()
        for (i in 0 until minOf(x.size, y.size)) {
            val p = x[i]
            val q = y[i]
            val c = if (p[0].isDigit() && q[0].isDigit()) p.trimStart('0').length.compareTo(q.trimStart('0').length).takeIf { it != 0 } ?: p.trimStart('0').compareTo(q.trimStart('0'))
            else collator.compare(p, q)
            if (c != 0) return@Comparator c
        }
        x.size.compareTo(y.size)
    }

    private fun grouped(hosts: List<Host>, vault: String, prefix: String?): List<HostSection> {
        val groups = hosts.groupBy { it.group }
        val names = groups.keys.sortedWith { a, b ->
            if (a.isEmpty() != b.isEmpty()) (if (a.isEmpty()) 1 else -1) else natural.compare(a, b)
        }
        return names.map { g ->
            val title = listOfNotNull(prefix, g.ifEmpty { null }).joinToString(" / ")
            HostSection("$vault/$g", title, groups.getValue(g).sortedWith { a, b -> natural.compare(a.label, b.label) })
        }
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
