package dev.detour.android.files

import android.net.Uri
import android.provider.OpenableColumns
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.detour.android.AppModel
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.Host
import dev.detour.android.core.double
import dev.detour.android.core.int
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.terminal.Clipboard
import dev.detour.android.terminal.TerminalSession
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.text.Collator
import java.util.UUID

/** A file on the server, as the core lists it. */
data class RemoteFile(val name: String, val kind: String, val size: Double, val modified: Double?, val permissions: Int?) {
    val isDir: Boolean get() = kind == "dir"
    val isLink: Boolean get() = kind == "link"

    companion object {
        fun from(m: Map<String, Any?>) = RemoteFile(
            name = m.str("name") ?: "",
            kind = m.str("kind") ?: "file",
            size = m.double("size") ?: 0.0,
            modified = m.double("modified"),
            permissions = m.int("permissions"),
        )
    }
}

/** An SFTP connection to a host, held by the core while the files session is open. */
class FilesSession(val host: Host) {
    sealed interface State {
        data object Connecting : State
        data object Ready : State
        data class NeedPassword(val prompt: String, val passphrase: Boolean) : State
        data class UnknownKey(val fingerprint: String) : State
        data class Failed(val message: String) : State
    }

    /** Something going on with files: a download, an upload or a delete. */
    data class Transfer(val id: String, val name: String, val kind: Kind, val done: Double = 0.0, val total: Double = 0.0) {
        enum class Kind { Down, Up, Delete }

        val fraction: Double? get() = if (total > 0) minOf(1.0, done / total) else null
    }

    /** Its place among the open sessions. */
    val id: UUID = UUID.randomUUID()
    var state by mutableStateOf<State>(State.Connecting)
    val transfers = mutableStateListOf<Transfer>()
    var home = "/"
        private set
    /** The folders shown, the home one first: kept here so the session comes back where it was. */
    val folders = mutableStateListOf<String>()

    private var filesId: String? = null

    /** The server key the person accepted: kept for the password asked next (the core pins it
     * only once a login succeeds). */
    private var trusted: String? = null

    suspend fun open(secret: String? = null, trust: String? = null) {
        if (trust != null) trusted = trust
        state = State.Connecting
        val args = mapOf("vaultId" to host.vaultId, "hostId" to host.id, "secret" to secret, "trustFingerprint" to (trust ?: trusted))
        try {
            val r = Core.call("files_open", args).obj()
            filesId = r.str("id")
            home = r.str("home") ?: "/"
            if (folders.firstOrNull() != home) {
                folders.clear()
                folders.add(home)
            }
            state = State.Ready
        } catch (f: CoreFailure) {
            state = when (f.kind) {
                "needSecret" -> State.NeedPassword(f["prompt"] ?: "", f["secret"] == "passphrase")
                "hostKeyUnknown" -> State.UnknownKey(f["fingerprint"] ?: "")
                else -> State.Failed(TerminalSession.describe(f))
            }
        } catch (e: Exception) {
            state = State.Failed(e.message ?: e.toString())
        }
    }

    fun close() {
        val id = filesId ?: return
        filesId = null
        AppModel.scope.launch { runCatching { Core.call("files_close", mapOf("id" to id)) } }
    }

    suspend fun list(path: String): List<RemoteFile> {
        val files = Core.call("files_list", mapOf("id" to (filesId ?: ""), "path" to path)).list().map { RemoteFile.from(it.obj()) }
        return files.sortedWith { a, b -> if (a.isDir != b.isDir) (if (a.isDir) -1 else 1) else natural(a.name, b.name) }
    }

    suspend fun mkdir(path: String) {
        Core.call("files_mkdir", mapOf("id" to (filesId ?: ""), "path" to path))
    }

    suspend fun rename(from: String, to: String) {
        Core.call("files_rename", mapOf("id" to (filesId ?: ""), "from" to from, "to" to to))
    }

    suspend fun remove(path: String) {
        transfer(path.substringAfterLast('/'), Transfer.Kind.Delete) { t, s ->
            Core.call("files_remove", mapOf("id" to (filesId ?: ""), "path" to path, "transfer" to t, "stream" to s))
        }
    }

    /** Downloads a file into a folder of its own in the app's cache; returns where. */
    suspend fun download(remote: String): File {
        val name = remote.substringAfterLast('/')
        val dir = File(context.cacheDir, "files/${UUID.randomUUID()}")
        withContext(Dispatchers.IO) { dir.mkdirs() }
        val local = File(dir, name)
        transfer(name, Transfer.Kind.Down) { t, s ->
            Core.call("files_download", mapOf("id" to (filesId ?: ""), "remote" to remote, "local" to local.path, "transfer" to t, "stream" to s))
        }
        return local
    }

    /** Downloads a file into a document the person picked on the phone. */
    suspend fun save(remote: String, to: Uri) {
        val local = download(remote)
        try {
            withContext(Dispatchers.IO) {
                val out = context.contentResolver.openOutputStream(to, "wt") ?: error("Can't write to $to")
                out.use { o -> local.inputStream().use { it.copyTo(o) } }
            }
        } finally {
            withContext(Dispatchers.IO) { local.parentFile?.deleteRecursively() }
        }
    }

    /** Uploads a file picked on the phone into `dir`. Fails with `exists` unless `replace`. */
    suspend fun upload(file: Uri, into: String, replace: Boolean) {
        val name = nameOf(file)
        // The core reads it from the app's own files.
        val copy = File(context.cacheDir, "uploads/${UUID.randomUUID()}")
        val local = File(copy, name)
        try {
            withContext(Dispatchers.IO) {
                copy.mkdirs()
                val input = context.contentResolver.openInputStream(file) ?: error("Can't read $name")
                input.use { i -> local.outputStream().use { i.copyTo(it) } }
            }
            transfer(name, Transfer.Kind.Up) { t, s ->
                Core.call("files_upload", mapOf("id" to (filesId ?: ""), "local" to local.path, "remote" to join(into, name), "replace" to replace, "transfer" to t, "stream" to s))
            }
        } finally {
            withContext(Dispatchers.IO) { copy.deleteRecursively() }
        }
    }

    /** A picked file's name, as the phone shows it. */
    fun nameOf(file: Uri): String {
        val shown = runCatching {
            context.contentResolver.query(file, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
        }.getOrNull()
        return (shown ?: file.lastPathSegment?.substringAfterLast('/') ?: "file").replace('/', '_')
    }

    fun cancel(t: Transfer) {
        AppModel.scope.launch { runCatching { Core.call("files_cancel", mapOf("transfer" to t.id)) } }
    }

    private suspend fun transfer(name: String, kind: Transfer.Kind, run: suspend (String, Long) -> Unit) {
        val tid = UUID.randomUUID().toString()
        val stream = Core.newStream()
        transfers.add(Transfer(tid, name, kind))
        Core.watchProgress(stream) { done, total ->
            val i = transfers.indexOfFirst { it.id == tid }
            if (i >= 0) transfers[i] = transfers[i].copy(done = done, total = total)
        }
        try {
            run(tid, stream.toLong())
        } finally {
            Core.unwatch(stream)
            transfers.removeAll { it.id == tid }
        }
    }

    companion object {
        /** The app's context (set when the app starts). */
        private val context get() = Clipboard.context

        fun join(dir: String, name: String): String = if (dir.endsWith("/")) dir + name else "$dir/$name"

        private val collator = Collator.getInstance().apply { strength = Collator.SECONDARY }
        private val runs = Regex("\\d+|\\D+")

        /** Names as Finder sorts them: "file2" before "file10". */
        fun natural(a: String, b: String): Int {
            val x = runs.findAll(a).map { it.value }.toList()
            val y = runs.findAll(b).map { it.value }.toList()
            for (i in 0 until minOf(x.size, y.size)) {
                val p = x[i]
                val q = y[i]
                val c = if (p[0].isDigit() && q[0].isDigit()) {
                    val pt = p.trimStart('0')
                    val qt = q.trimStart('0')
                    if (pt.length != qt.length) pt.length - qt.length else pt.compareTo(qt)
                } else collator.compare(p, q)
                if (c != 0) return c
            }
            return if (x.size != y.size) x.size - y.size else collator.compare(a, b)
        }
    }
}
