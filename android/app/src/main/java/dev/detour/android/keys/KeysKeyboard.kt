package dev.detour.android.keys

import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import dev.detour.android.core.t
import dev.detour.android.terminal.KeyCap
import dev.detour.android.terminal.TerminalSession
import dev.detour.android.terminal.appCursor
import dev.detour.android.ui.Segmented
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** A row of the keys keyboard: eight places, each a key or empty. */
data class KeyBlock(val keys: List<BarKey?> = List(SIZE) { null }, val id: String = UUID.randomUUID().toString()) {
    companion object { const val SIZE = 8 }
}

/** The keys keyboard's rows, kept on this device. */
object KeysKeyboardStore {
    private lateinit var context: Context
    var blocks by mutableStateOf(defaults)
        private set

    fun load(context: Context) {
        this.context = context
        val text = context.getSharedPreferences("keys", Context.MODE_PRIVATE).getString("keysKeyboard", null) ?: return
        runCatching {
            val a = JSONArray(text)
            blocks = (0 until a.length()).map { i ->
                val o = a.getJSONObject(i)
                val keys = o.getJSONArray("keys")
                KeyBlock((0 until KeyBlock.SIZE).map { j -> if (j < keys.length() && !keys.isNull(j)) BarKey.from(keys.getJSONObject(j)) else null }, o.optString("id"))
            }
        }
    }

    fun set(blocks: List<KeyBlock>) {
        this.blocks = blocks
        val a = JSONArray(blocks.map { b ->
            JSONObject().put("id", b.id).put("keys", JSONArray(b.keys.map { it?.json() ?: JSONObject.NULL }))
        })
        context.getSharedPreferences("keys", Context.MODE_PRIVATE).edit().putString("keysKeyboard", a.toString()).apply()
    }

    fun reset() = set(defaults)

    val defaults: List<KeyBlock>
        get() {
            fun key(v: String) = BarKey(BarKey.Kind.Key, value = v)
            fun combo(v: String) = BarKey(BarKey.Kind.Combo, value = v)
            fun row(keys: List<BarKey>) = KeyBlock(keys)
            fun chars(s: String) = row(s.map { key(it.toString()) })
            return listOf(
                row(listOf(key("esc"), key("tab"), BarKey(BarKey.Kind.Ctrl), BarKey(BarKey.Kind.Alt), key("home"), key("end"), key("pageup"), key("pagedown"))),
                row(listOf(key("left"), key("down"), key("up"), key("right"), key("space"), key("backspace"), key("delete"), key("enter"))),
                row((1..8).map { key("f$it") }),
                row((9..12).map { key("f$it") } + listOf(combo("ctrl+c"), combo("ctrl+d"), combo("ctrl+z"), combo("ctrl+r"))),
                chars("|~/\\-_=+"),
                chars("<>{}[]()"),
                chars("!@#$%^&*"),
                chars("'\"`,.?;:"),
            )
        }
}

/** What the keys keyboard shows: its keys, the snippets, or the shell's history. */
enum class KeysPanel { Keys, Snippets, History }

/**
 * The keys keyboard, shown in place of the system one: tabs at the bottom for its keys (rows of
 * eight; two rows side by side where there is room), the snippets, and the shell's history.
 */
@Composable
fun KeysKeyboardView(session: TerminalSession, modifier: Modifier = Modifier) {
    var panel by rememberSaveable { mutableStateOf(KeysPanel.Keys) }
    Column(modifier.padding(horizontal = 6.dp).padding(top = 6.dp, bottom = 4.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
            when (panel) {
                KeysPanel.Keys -> KeysGrid(session)
                KeysPanel.Snippets -> SnippetsPanel(session)
                KeysPanel.History -> HistoryPanel(session)
            }
        }
        Segmented(listOf(KeysPanel.Keys to t("Keys"), KeysPanel.Snippets to t("Snippets"), KeysPanel.History to t("History")), panel) { panel = it }
    }
}

@Composable
private fun KeysGrid(session: TerminalSession) {
    val haptic = LocalHapticFeedback.current
    BoxWithConstraints(Modifier.fillMaxSize()) {
        LazyVerticalGrid(
            GridCells.Fixed(if (maxWidth >= 700.dp) 2 else 1),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            items(KeysKeyboardStore.blocks, key = { it.id }) { block ->
                Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                    for (i in 0 until KeyBlock.SIZE) {
                        val key = block.keys.getOrNull(i)
                        if (key == null) {
                            Box(Modifier.weight(1f).height(40.dp))
                        } else {
                            val on = (key.kind == BarKey.Kind.Ctrl && session.ctrl) || (key.kind == BarKey.Kind.Alt && session.alt)
                            KeyCap(key, on, Modifier.weight(1f).height(40.dp)) {
                                haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                when (key.kind) {
                                    BarKey.Kind.Ctrl -> session.ctrl = !session.ctrl
                                    BarKey.Kind.Alt -> session.alt = !session.alt
                                    else -> key.bytes(session.appCursor)?.let { session.send(it) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
