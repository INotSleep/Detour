package dev.detour.android.keys

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardReturn
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
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
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.SnippetGroup
import dev.detour.android.core.t
import dev.detour.android.terminal.TerminalSession
import dev.detour.android.ui.colors
import kotlinx.coroutines.launch
import kotlinx.coroutines.yield

/** The snippets on the keyboard: a tap runs one. */
@Composable
fun SnippetsPanel(session: TerminalSession) {
    var groups by remember { mutableStateOf<List<SnippetGroup>?>(null) }
    LaunchedEffect(Unit) { groups = runCatching { SnippetGroup.load() }.getOrDefault(emptyList()) }
    val g = groups
    if (g == null) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) { CircularProgressIndicator(Modifier.padding(top = 40.dp), color = colors.accent) }
        return
    }
    if (g.all { it.snippets.isEmpty() }) {
        Text(t("No snippets"), color = colors.secondary, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(top = 40.dp))
        return
    }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        for (group in g) {
            group.name?.let { name -> item { Text(name, color = colors.secondary, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 4.dp)) } }
            items(group.snippets) { s -> PanelRow(s.name, s.command) { session.run(s.command) } }
        }
    }
}

/** The shell's history, newest first: a tap types a command at the prompt, the arrow runs it. */
@Composable
fun HistoryPanel(session: TerminalSession) {
    // Kept with the session: what was read shows at once, and is read again behind it.
    val history = session.history
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) { history.load(session) }
    val lines = history.lines
    val error = history.error
    when {
        lines != null -> {
            if (lines.isEmpty()) {
                Text(t("No history yet"), color = colors.secondary, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(top = 40.dp))
            } else {
                LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    itemsIndexed(lines) { _, line ->
                        PanelRow(line, null, mono = true, run = { session.run(line) }) { session.input(line.toByteArray()) }
                    }
                }
            }
        }
        error != null -> Column(Modifier.fillMaxWidth().padding(top = 30.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(error, color = colors.secondary, fontSize = 13.sp, textAlign = TextAlign.Center)
            Text(t("Try again"), color = colors.accent, fontSize = 15.sp, modifier = Modifier.clip(RoundedCornerShape(8.dp)).background(colors.fill).clickable { scope.launch { history.load(session) } }.padding(horizontal = 14.dp, vertical = 8.dp))
        }
        else -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) { CircularProgressIndicator(Modifier.padding(top = 40.dp), color = colors.accent) }
    }
}

/** A row of a panel: a tap does its thing; with `run`, an arrow on the right runs it. */
@Composable
private fun PanelRow(title: String, detail: String?, mono: Boolean = false, run: (() -> Unit)? = null, tap: () -> Unit) {
    val haptic = LocalHapticFeedback.current
    Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
        Column(
            Modifier.weight(1f).clip(RoundedCornerShape(8.dp)).background(colors.fill)
                .clickable { haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove); tap() }
                .padding(horizontal = 10.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                title, color = colors.label,
                fontSize = if (mono) 14.sp else 15.sp, fontWeight = if (mono) FontWeight.Normal else FontWeight.Medium,
                fontFamily = if (mono) FontFamily.Monospace else null, maxLines = if (mono) 2 else 1, overflow = TextOverflow.Ellipsis,
            )
            if (detail != null) Text(detail, color = colors.secondary, fontSize = 12.sp, fontFamily = FontFamily.Monospace, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (run != null) {
            Box(
                Modifier.width(44.dp).height(40.dp).clip(RoundedCornerShape(8.dp)).background(colors.fill)
                    .clickable { haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove); run() },
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.AutoMirrored.Outlined.KeyboardReturn, t("Run"), tint = colors.label, modifier = Modifier.size(20.dp)) }
        }
    }
}

/**
 * The commands run in this app's sessions to the host (newest first), then the history files of
 * the host's shells (bash, zsh, fish), read over a connection of its own with the session's
 * login. A shell writes its file when it exits: what the open sessions ran comes from the screen.
 */
class ShellHistory {
    var lines by mutableStateOf<List<String>?>(null)
    var error by mutableStateOf<String?>(null)

    private var loading = false

    suspend fun load(session: TerminalSession) {
        if (loading) return
        loading = true
        val host = session.host
        val typed = TypedHistory.list(host.id)
        // What was read before stays up while it is read again.
        val before = lines
        lines = if (before != null) (typed + before).distinct() else typed.ifEmpty { null }
        if (before == null) error = null
        val stream = Core.newStream()
        val out = java.io.ByteArrayOutputStream()
        val ended = kotlinx.coroutines.CompletableDeferred<Unit>()
        Core.watch(stream, { synchronized(out) { out.write(it) } }, { ended.complete(Unit) })
        try {
            Core.call("host_exec", mapOf("vaultId" to host.vaultId, "hostId" to host.id, "command" to COMMAND, "stream" to stream.toLong(), "secret" to session.typedSecret))
            // The output comes before its end, but on the main thread after the call returns:
            // wait for the end (not long) so none of it is missed.
            kotlinx.coroutines.withTimeoutOrNull(3000) { ended.await() }
            yield()
            val text = synchronized(out) { String(out.toByteArray(), Charsets.UTF_8) }
            val seen = typed.toMutableSet()
            lines = typed + parse(text).filter { seen.add(it) }
            error = null
        } catch (f: CoreFailure) {
            // What was run here still shows when the server's files can't be read.
            if (lines.isNullOrEmpty()) error = if (f.kind == "needSecret") t("The history needs the host's password") else TerminalSession.describe(f)
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (e: Exception) {
            if (lines.isNullOrEmpty()) error = e.message
        } finally {
            Core.unwatch(stream)
            loading = false
        }
    }

    companion object {
        /** Runs under sh whatever the login shell is (fish can't read a POSIX loop). */
        private const val COMMAND = "sh -c 'for f in \"\$HOME/.bash_history\" \"\$HOME/.zsh_history\" \"\$HOME/.local/share/fish/fish_history\"; do [ -f \"\$f\" ] && tail -n 1000 \"\$f\"; done'"

        /** Commands, newest first, each once. zsh writes ": <time>:0;command", fish "- cmd: command". */
        fun parse(text: String): List<String> {
            val out = ArrayList<String>()
            val seen = HashSet<String>()
            for (raw in text.lines().asReversed()) {
                var line = raw
                if (line.startsWith(": ") && line.contains(';')) line = line.substringAfter(';')
                else if (line.startsWith("- cmd: ")) line = line.removePrefix("- cmd: ")
                else if (line.startsWith("  when:") || line.startsWith("  paths:") || (line.startsWith("#") && line.drop(1).toLongOrNull() != null)) continue
                line = line.trim()
                if (line.isEmpty() || !seen.add(line)) continue
                out.add(line)
                if (out.size >= 300) break
            }
            return out
        }
    }
}

/**
 * Commands run in this app's sessions, by host, newest first. Kept in memory only, for as long
 * as the app runs: what people type may hold secrets.
 */
object TypedHistory {
    private val byHost = HashMap<String, List<String>>()

    fun add(command: String, host: String) {
        if (command.isEmpty()) return
        byHost[host] = (listOf(command) + (byHost[host] ?: emptyList()).filter { it != command }).take(300)
    }

    fun list(host: String): List<String> = byHost[host] ?: emptyList()

    /** A line of the screen without its prompt: what follows the first "$ ", "# ", "% ", "❯ " or "> ". */
    fun command(afterPrompt: String): String? {
        val found = listOf("$ ", "# ", "% ", "❯ ", "> ").mapNotNull { e -> afterPrompt.indexOf(e).takeIf { it >= 0 }?.let { it to e.length } }.minByOrNull { it.first }
            ?: return null
        return afterPrompt.substring(found.first + found.second).trim().ifEmpty { null }
    }
}
