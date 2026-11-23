package dev.detour.android.terminal

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.termux.terminal.TerminalSessionClient
import com.termux.view.TerminalView
import dev.detour.android.AppModel
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.Host
import dev.detour.android.core.bool
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.core.t
import dev.detour.android.keys.TypedHistory
import kotlinx.coroutines.launch
import java.util.UUID
import com.termux.terminal.TerminalSession as Emulated

/**
 * One terminal tab: an SSH session the core holds, drawn by the terminal emulator. The emulator
 * (and its scrollback) lives here for as long as the session is open, whatever the screen does.
 */
class TerminalSession(val host: Host) {
    sealed interface State {
        data object Connecting : State
        data object Connected : State
        /** The session ended: by itself (`exit`) or because the connection dropped. */
        data class Closed(val dropped: Boolean) : State
        data class NeedPassword(val prompt: String) : State
        data class UnknownKey(val fingerprint: String) : State
        data class Failed(val message: String) : State
    }

    val id: UUID = UUID.randomUUID()
    var state by mutableStateOf<State>(State.Connecting)
    var title by mutableStateOf(host.label)
    /** Ctrl and Alt from the key bar, for the next key typed. */
    var ctrl by mutableStateOf(false)
    var alt by mutableStateOf(false)
    /** The keys keyboard is chosen in place of the system one. */
    var keysKeyboard by mutableStateOf(false)
    /** The keys keyboard is up (it hides as the system one does). */
    var keysShown by mutableStateOf(false)
    /** The system keyboard is coming back in place of the keys one. */
    var returning by mutableStateOf(false)
    /** The keys keyboard asked for its settings. */
    var customizing by mutableStateOf(false)
    /** The password or passphrase typed for this session, kept in memory while it is open. */
    var typedSecret: String? = null
        private set

    private var sessionId: String? = null
    private var stream: ULong? = null
    private var userClosed = false
    private var reconnectWhenEnded = false
    private var cols = 80
    private var rows = 24
    /** The size the server's terminal has: the shell wraps lines by it, so it must be the screen's. */
    private var sentCols = 0
    private var sentRows = 0
    private var resizing = false

    /** The shell's history for the keys keyboard's panel, read once and kept fresh. */
    val history = dev.detour.android.keys.ShellHistory()

    /** The view showing this session now, if any. */
    var view: TerminalView? = null

    /** The colours the terminal is drawn in; put on the emulator once it exists. */
    var theme: TerminalTheme? = null
        set(value) {
            field = value
            paint()
        }
    private var painted: TerminalTheme? = null

    private fun paint() {
        val e = emulated.emulator ?: return
        val t = theme ?: return
        if (painted === t) return
        t.applyTo(e)
        painted = t
        view?.invalidate()
    }

    val emulated = Emulated(5000, Client(), object : Emulated.Transport {
        override fun send(data: ByteArray) = this@TerminalSession.send(data)
        override fun resize(columns: Int, rows: Int) = this@TerminalSession.resize(columns, rows)
    })

    /** The server key the person accepted: kept for the tries after it (the password asked next). */
    private var trusted: String? = null

    fun connect(secret: String? = null, trust: String? = null) {
        if (trust != null) trusted = trust
        state = State.Connecting
        val s = Core.newStream()
        stream = s
        Core.watch(s, { output(it) }, { ended(it) })
        // The session opens at this size; a new one the screen takes meanwhile goes once it's open.
        sentCols = cols
        sentRows = rows
        val args = mapOf(
            "vaultId" to host.vaultId, "hostId" to host.id, "secret" to secret, "trustFingerprint" to (trust ?: trusted),
            "cols" to cols, "rows" to rows,
        ).filterValues { it != null }
        AppModel.scope.launch {
            try {
                val r = Core.call("session_open", mapOf("args" to args, "stream" to s.toLong())).obj()
                sessionId = r.str("sessionId")
                if (secret != null) typedSecret = secret
                state = State.Connected
                sendSize()
            } catch (f: CoreFailure) {
                Core.unwatch(s)
                state = when (f.kind) {
                    "needSecret" -> State.NeedPassword(f["prompt"] ?: "")
                    "hostKeyUnknown" -> State.UnknownKey(f["fingerprint"] ?: "")
                    else -> State.Failed(describe(f))
                }
            } catch (e: Exception) {
                state = State.Failed(e.message ?: e.toString())
            }
        }
    }

    private fun output(bytes: ByteArray) {
        emulated.feed(bytes, bytes.size)
    }

    private fun ended(info: Map<String, Any?>) {
        sessionId = null
        if (userClosed) return
        // An exit status means the shell ended; none means the connection went away.
        val dropped = info["exitStatus"] == null
        if (dropped && reconnectWhenEnded) {
            reconnectWhenEnded = false
            reconnect()
            return
        }
        state = State.Closed(dropped)
    }

    /** After the phone slept or changed networks: a dead connection is replaced at once. */
    fun checkConnection() {
        when (val s = state) {
            is State.Closed -> if (s.dropped) reconnect()
            State.Connected -> {
                val id = sessionId ?: return
                AppModel.scope.launch {
                    val r = runCatching { Core.call("session_check", mapOf("sessionId" to id, "timeoutMs" to 5000)).obj() }.getOrNull()
                    if (r?.bool("alive") != false || userClosed) return@launch
                    if ((state as? State.Closed)?.dropped == true) reconnect() else reconnectWhenEnded = true
                }
            }
            else -> {}
        }
    }

    fun reconnect() {
        output("\r\n".toByteArray())
        connect()
    }

    /**
     * The command on the screen's line where the cursor is (wrapped lines joined), the prompt left
     * out; null in a full-screen program or with no prompt in sight (a password asked).
     */
    fun commandLine(): String? {
        val e = emulated.emulator ?: return null
        if (e.isAlternateBufferActive) return null
        val screen = e.screen
        val end = e.cursorRow
        var start = end
        while (start > 0 && screen.getLineWrap(start - 1)) start--
        return TypedHistory.command(screen.getSelectedText(0, start, e.mColumns, end, true))
    }

    /** What the person typed (or a key of the bar), with Ctrl or Alt if armed. */
    fun send(bytes: ByteArray) {
        // Enter: what is on the line is a command the person ran (the server hasn't answered yet).
        if (bytes.contains(13.toByte()) && !ctrl && !alt) commandLine()?.let { TypedHistory.add(it, host.id) }
        var out = bytes
        if (ctrl && out.size == 1) control(out[0])?.let { out = byteArrayOf(it) }
        if (alt) out = byteArrayOf(0x1b) + out
        ctrl = false
        alt = false
        input(out)
    }

    /** Types a snippet's command and presses Enter (each line of a longer one too). */
    fun run(command: String) {
        TypedHistory.add(command.trim(), host.id)
        val text = command.trim('\n', '\r').replace("\r\n", "\r").replace("\n", "\r") + "\r"
        input(text.toByteArray())
    }

    fun input(bytes: ByteArray) {
        val id = sessionId
        if (id == null) {
            if (state is State.Closed && bytes.contentEquals(byteArrayOf(13))) reconnect()
            return
        }
        val text = String(bytes, Charsets.UTF_8)
        AppModel.scope.launch { runCatching { Core.call("session_input", mapOf("sessionId" to id, "data" to text)) } }
    }

    fun resize(cols: Int, rows: Int) {
        this.cols = cols
        this.rows = rows
        sendSize()
    }

    /** Tells the server the screen's size, one call at a time so the last size is the one it keeps. */
    private fun sendSize() {
        val id = sessionId ?: return
        if (resizing || (cols == sentCols && rows == sentRows)) return
        val c = cols
        val r = rows
        resizing = true
        AppModel.scope.launch {
            runCatching { Core.call("session_resize", mapOf("sessionId" to id, "cols" to c, "rows" to r)) }
            sentCols = c
            sentRows = r
            resizing = false
            sendSize()
        }
    }

    fun close() {
        userClosed = true
        typedSecret = null
        view = null
        stream?.let { Core.unwatch(it) }
        val id = sessionId ?: return
        AppModel.scope.launch { runCatching { Core.call("session_close", mapOf("sessionId" to id)) } }
    }

    /** The emulator's callbacks: a redraw of the view showing it, the title, the clipboard. */
    private inner class Client : TerminalSessionClient {
        override fun onTextChanged(changedSession: Emulated) {
            paint()
            view?.onScreenUpdated()
        }
        override fun onTitleChanged(changedSession: Emulated) {
            changedSession.title?.takeIf { it.isNotBlank() }?.let { title = it }
        }
        override fun onSessionFinished(finishedSession: Emulated) {}
        override fun onCopyTextToClipboard(session: Emulated, text: String) { Clipboard.copy(text) }
        override fun onPasteTextFromClipboard(session: Emulated) {
            Clipboard.paste()?.let { session.emulator?.paste(it) }
        }
        override fun onBell(session: Emulated) {}
        // A reset puts the emulator's own colours back: ours go on again.
        override fun onColorsChanged(session: Emulated) {
            painted = null
            paint()
            view?.invalidate()
        }
        override fun onTerminalCursorStateChange(state: Boolean) {}
        override fun getTerminalCursorStyle(): Int? = null
        override fun logError(tag: String?, message: String?) {}
        override fun logWarn(tag: String?, message: String?) {}
        override fun logInfo(tag: String?, message: String?) {}
        override fun logDebug(tag: String?, message: String?) {}
        override fun logVerbose(tag: String?, message: String?) {}
        override fun logStackTraceWithMessage(tag: String?, message: String?, e: Exception?) {}
        override fun logStackTrace(tag: String?, e: Exception?) {}
    }

    companion object {
        /** Ctrl with a key: its control character. */
        fun control(b: Byte): Byte? {
            val c = b.toInt()
            return when (c) {
                in 'a'.code..'z'.code -> (c - 96).toByte()
                in 'A'.code..'Z'.code -> (c - 64).toByte()
                in '@'.code..'_'.code -> (c - 64).toByte()
                ' '.code -> 0
                '?'.code -> 127
                else -> null
            }
        }

        fun describe(f: CoreFailure): String = when (f.kind) {
            "unreachable" -> t("Can't reach %@", f["endpoint"] ?: t("the host"))
            "authFailed" -> t("The server didn't accept the login")
            "hostKeyChanged" -> t("The server's key changed. Check it on a computer before connecting.")
            "proxy" -> t("The proxy failed: %@", f["reason"] ?: "")
            "locked" -> t("Detour is locked")
            else -> f["message"] ?: f.kind
        }
    }
}

/** The program asked for application cursor keys (vim, less): arrows go as `Esc O A`. */
val TerminalSession.appCursor: Boolean
    get() = emulated.emulator?.isCursorKeysApplicationMode ?: false
