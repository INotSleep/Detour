package dev.detour.android.keys

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBackIos
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import dev.detour.android.ui.DropdownMenu
import dev.detour.android.ui.DropdownMenuItem
import dev.detour.android.ui.menuIcon
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.core.SnippetGroup
import dev.detour.android.core.t
import dev.detour.android.terminal.KeyCap
import dev.detour.android.ui.BarButton
import dev.detour.android.ui.Cell
import dev.detour.android.ui.CellText
import dev.detour.android.ui.FieldCell
import dev.detour.android.ui.Segmented
import dev.detour.android.ui.Sheet
import dev.detour.android.ui.SheetBar
import dev.detour.android.ui.ToggleCell
import dev.detour.android.ui.colors
import dev.detour.android.ui.section
import dev.detour.android.ui.SwipeActions
import dev.detour.android.ui.deleteAction

/** A place a key goes: in the bar (null: a new key at its end) or a row of the keyboard. */
private sealed interface Slot {
    data class Bar(val index: Int?) : Slot
    data class Block(val block: String, val index: Int) : Slot
}

/**
 * Settings of the keys: the bar over the keyboard as it looks, then the keys keyboard's rows of
 * eight. A tap on a key (or an empty place) picks what goes there. Full screen, with a back button.
 */
@Composable
fun KeyboardEditor(back: String = dev.detour.android.core.t("Settings"), close: () -> Unit) {
    var picking by remember { mutableStateOf<Slot?>(null) }
    var confirmReset by remember { mutableStateOf(false) }
    BackHandler(onBack = close)

    Column(Modifier.fillMaxSize().background(colors.background).windowInsetsPadding(WindowInsets.statusBars).windowInsetsPadding(WindowInsets.navigationBars)) {
        SheetBar(t("Keyboard"), left = { dev.detour.android.ui.BackButton(back, close) })
        LazyColumn(Modifier.fillMaxSize()) {
            section(t("Over the keyboard")) {
                ToggleCell(t("Show the key bar"), !KeyBarStore.hidden, last = KeyBarStore.hidden) { KeyBarStore.hide(!it) }
                if (!KeyBarStore.hidden) {
                    Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                        KeyBarStore.keys.forEachIndexed { i, key ->
                            KeyCap(key, modifier = Modifier.width(40.dp).height(34.dp)) { picking = Slot.Bar(i) }
                        }
                        AddCap(Modifier.width(40.dp).height(34.dp)) { picking = Slot.Bar(null) }
                    }
                }
            }
            section(t("Keys keyboard")) {
                val blocks = KeysKeyboardStore.blocks
                blocks.forEachIndexed { bi, block -> BlockRow(block, bi, blocks.size) { i -> picking = Slot.Block(block.id, i) } }
                Cell(last = true, onClick = { KeysKeyboardStore.set(KeysKeyboardStore.blocks + KeyBlock()) }) {
                    Icon(Icons.Outlined.Add, null, tint = colors.accent, modifier = Modifier.size(20.dp))
                    Spacer(Modifier.width(12.dp))
                    Text(t("Add row"), color = colors.accent, fontSize = 17.sp)
                }
            }
            section {
                Cell(last = true, onClick = { confirmReset = true }) { Text(t("Reset to default"), color = colors.red, fontSize = 17.sp) }
            }
            item { Spacer(Modifier.height(40.dp)) }
        }
    }

    if (confirmReset) {
        AlertDialog(
            onDismissRequest = { confirmReset = false },
            title = { Text(t("Put back the default keys?")) },
            confirmButton = { TextButton({ confirmReset = false; KeyBarStore.reset(); KeysKeyboardStore.reset() }) { Text(t("Reset"), color = colors.red) } },
            dismissButton = { TextButton({ confirmReset = false }) { Text(t("Cancel")) } },
        )
    }
    picking?.let { slot ->
        KeyPicker(current = key(slot), movable = slot is Slot.Bar && slot.index != null, close = { picking = null }) { change ->
            apply(change, slot)
            picking = null
        }
    }
}

/** A row of eight: a tap on a key picks it, a long press moves or deletes the row. */
@Composable
private fun BlockRow(block: KeyBlock, index: Int, count: Int, pick: (Int) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Box {
        SwipeActions(listOf(deleteAction { KeysKeyboardStore.set(KeysKeyboardStore.blocks.filter { it.id != block.id }) })) {
        Column {
            Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                for (i in 0 until KeyBlock.SIZE) KeyCap(block.keys.getOrNull(i), modifier = Modifier.weight(1f).height(40.dp)) { pick(i) }
                Box(Modifier.size(28.dp, 40.dp).clickable { menu = true }, contentAlignment = Alignment.Center) {
                    Text("⋮", color = colors.secondary, fontSize = 20.sp)
                }
            }
            androidx.compose.material3.HorizontalDivider(Modifier.padding(start = 12.dp), thickness = 0.5.dp, color = colors.separator)
        }
        }
        DropdownMenu(menu, { menu = false }, modifier = Modifier.align(Alignment.TopEnd)) {
            fun swap(to: Int) {
                val list = KeysKeyboardStore.blocks.toMutableList()
                java.util.Collections.swap(list, index, to)
                KeysKeyboardStore.set(list)
            }
            if (index > 0) DropdownMenuItem(leadingIcon = menuIcon("Move up"), text = { Text(t("Move up")) }, onClick = { menu = false; swap(index - 1) })
            if (index < count - 1) DropdownMenuItem(leadingIcon = menuIcon("Move down"), text = { Text(t("Move down")) }, onClick = { menu = false; swap(index + 1) })
            DropdownMenuItem(leadingIcon = menuIcon("Delete"), text = { Text(t("Delete"), color = colors.red) }, onClick = { menu = false; KeysKeyboardStore.set(KeysKeyboardStore.blocks.filter { it.id != block.id }) })
        }
    }
}

@Composable
private fun AddCap(modifier: Modifier, onClick: () -> Unit) {
    Box(modifier.clip(RoundedCornerShape(7.dp)).background(colors.accent.copy(alpha = 0.12f)).clickable(onClick = onClick), contentAlignment = Alignment.Center) {
        Icon(Icons.Outlined.Add, t("Add key"), tint = colors.accent, modifier = Modifier.size(18.dp))
    }
}

private fun key(slot: Slot): BarKey? = when (slot) {
    is Slot.Bar -> slot.index?.let { KeyBarStore.keys.getOrNull(it) }
    is Slot.Block -> KeysKeyboardStore.blocks.firstOrNull { it.id == slot.block }?.keys?.getOrNull(slot.index)
}

private fun apply(change: KeyChange, slot: Slot) {
    when (slot) {
        is Slot.Bar -> {
            val list = KeyBarStore.keys.toMutableList()
            val i = slot.index
            when (change) {
                is KeyChange.Set -> if (i != null && i in list.indices) list[i] = change.key.copy(id = list[i].id) else list.add(change.key)
                KeyChange.Remove -> if (i != null && i in list.indices) list.removeAt(i)
                is KeyChange.Move -> if (i != null && i in list.indices && (i + change.by) in list.indices) java.util.Collections.swap(list, i, i + change.by)
            }
            KeyBarStore.set(list)
        }
        is Slot.Block -> {
            val blocks = KeysKeyboardStore.blocks.toMutableList()
            val bi = blocks.indexOfFirst { it.id == slot.block }.takeIf { it >= 0 } ?: return
            val keys = blocks[bi].keys.toMutableList()
            while (keys.size < KeyBlock.SIZE) keys.add(null)
            when (change) {
                is KeyChange.Set -> keys[slot.index] = change.key
                KeyChange.Remove -> keys[slot.index] = null
                is KeyChange.Move -> {}
            }
            blocks[bi] = blocks[bi].copy(keys = keys)
            KeysKeyboardStore.set(blocks)
        }
    }
}

sealed interface KeyChange {
    data class Set(val key: BarKey) : KeyChange
    data object Remove : KeyChange
    /** Swap with the key before (-1) or after (1). */
    data class Move(val by: Int) : KeyChange
}

private enum class PickerTab { Keys, Combination, Snippet }

/** Picks what a key is: one of the usual keys, a combination, or a snippet. */
@Composable
private fun KeyPicker(current: BarKey?, movable: Boolean, close: () -> Unit, done: (KeyChange) -> Unit) {
    var tab by remember {
        mutableStateOf(
            when (current?.kind) {
                BarKey.Kind.Combo -> PickerTab.Combination
                BarKey.Kind.Snippet, BarKey.Kind.Text -> PickerTab.Snippet
                else -> PickerTab.Keys
            }
        )
    }
    val pick = { k: BarKey -> done(KeyChange.Set(k)) }
    Sheet(close) {
        SheetBar(current?.title ?: t("Add key"), left = { BarButton(t("Cancel"), onClick = close) })
        LazyColumn(Modifier.fillMaxSize().imePadding()) {
            item {
                Box(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    Segmented(listOf(PickerTab.Keys to t("Keys"), PickerTab.Combination to t("Combination"), PickerTab.Snippet to t("Snippet")), tab) { tab = it }
                }
            }
            when (tab) {
                PickerTab.Keys -> keyTable(pick)
                PickerTab.Combination -> item { ComboBuilder(current?.takeIf { it.kind == BarKey.Kind.Combo }, pick) }
                PickerTab.Snippet -> item { SnippetChooser(pick) }
            }
            if (current != null) {
                section {
                    if (movable) {
                        Cell(onClick = { done(KeyChange.Move(-1)) }) { Text(t("Move left"), color = colors.accent, fontSize = 17.sp) }
                        Cell(onClick = { done(KeyChange.Move(1)) }) { Text(t("Move right"), color = colors.accent, fontSize = 17.sp) }
                    }
                    Cell(last = true, onClick = { done(KeyChange.Remove) }) { Text(t("Remove"), color = colors.red, fontSize = 17.sp) }
                }
            }
            item { Spacer(Modifier.height(40.dp)) }
        }
    }
}

/** The usual keys, by kind: everything a terminal on a phone may need. */
private fun LazyListScope.keyTable(pick: (BarKey) -> Unit) {
    fun key(v: String) = BarKey(BarKey.Kind.Key, value = v)
    val groups = listOf(
        "Modifiers" to listOf(BarKey(BarKey.Kind.Ctrl), BarKey(BarKey.Kind.Alt)),
        "Keys" to listOf("esc", "tab", "enter", "backspace", "delete", "insert", "space").map(::key),
        "Arrows and pages" to listOf("left", "down", "up", "right", "home", "end", "pageup", "pagedown").map(::key),
        "Function keys" to (1..24).map { key("f$it") },
        "Symbols" to "|~/\\-_=+<>{}[]()!@#$%^&*'\"`,.?;:".map { key(it.toString()) },
        "Ctrl and a letter" to "abcdefghijklmnopqrstuvwxyz".map { BarKey(BarKey.Kind.Combo, value = "ctrl+$it") },
    )
    for ((title, keys) in groups) {
        section(t(title)) { KeyGrid(keys, null, pick) }
    }
}

@Composable
private fun KeyGrid(keys: List<BarKey>, chosen: String?, pick: (BarKey) -> Unit) {
    Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        keys.chunked(6).forEach { row ->
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                row.forEach { k -> KeyCap(k, on = chosen != null && k.value == chosen, modifier = Modifier.weight(1f).height(40.dp)) { pick(k) } }
                repeat(6 - row.size) { Spacer(Modifier.weight(1f)) }
            }
        }
    }
}

/** A combination: modifiers and a key (a named one, or any character typed). */
@Composable
private fun ComboBuilder(start: BarKey?, pick: (BarKey) -> Unit) {
    val parts = start?.comboParts
    var ctrl by remember { mutableStateOf(parts?.ctrl ?: true) }
    var alt by remember { mutableStateOf(parts?.alt ?: false) }
    var shift by remember { mutableStateOf(parts?.shift ?: false) }
    var key by remember { mutableStateOf(parts?.key ?: "") }
    val value = (listOfNotNull(if (ctrl) "ctrl" else null, if (alt) "alt" else null, if (shift) "shift" else null) + key).joinToString("+")
    val made = BarKey(BarKey.Kind.Combo, value = value)
    val valid = key.isNotEmpty() && (ctrl || alt || shift) && made.bytes() != null

    Column(Modifier.padding(horizontal = 16.dp).padding(top = 22.dp)) {
        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(colors.cell)) {
            Row(Modifier.padding(12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                ModifierToggle("Ctrl", ctrl) { ctrl = !ctrl }
                ModifierToggle("Alt", alt) { alt = !alt }
                ModifierToggle("Shift", shift) { shift = !shift }
                Spacer(Modifier.weight(1f))
                KeyCap(if (key.isEmpty()) null else made, on = valid, modifier = Modifier.width(56.dp).height(40.dp))
            }
            FieldCell(if (key.length == 1) key else "", { key = it.takeLast(1) }, t("Letter, digit or symbol"), mono = true, plain = true)
            Cell(last = true, onClick = if (valid) ({ pick(made) }) else null) { Text(t("Use"), color = if (valid) colors.accent else colors.tertiary, fontSize = 17.sp) }
        }
        Text(t("Or a key").uppercase(), color = colors.secondary, fontSize = 13.sp, modifier = Modifier.padding(start = 16.dp, top = 18.dp, bottom = 6.dp))
        Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(colors.cell)) {
            KeyGrid(KeyCodes.named.map { BarKey(BarKey.Kind.Key, value = it) }, key) { key = it.value }
        }
    }
}

@Composable
private fun ModifierToggle(name: String, on: Boolean, toggle: () -> Unit) {
    Text(
        name, color = if (on) Color.White else colors.label, fontSize = 15.sp, fontWeight = FontWeight.Medium,
        modifier = Modifier.clip(RoundedCornerShape(7.dp)).background(if (on) colors.accent else colors.fill).clickable(onClick = toggle).padding(horizontal = 10.dp, vertical = 7.dp),
    )
}

/** A snippet of the vault or a team, run with a tap of the key; or any text to type. */
@Composable
private fun SnippetChooser(pick: (BarKey) -> Unit) {
    var groups by remember { mutableStateOf<List<SnippetGroup>?>(null) }
    var text by remember { mutableStateOf("") }
    var label by remember { mutableStateOf("") }
    LaunchedEffect(Unit) { groups = runCatching { SnippetGroup.load() }.getOrDefault(emptyList()) }
    Column(Modifier.padding(horizontal = 16.dp)) {
        val g = groups
        if (g == null) {
            CircularProgressIndicator(Modifier.padding(24.dp).align(Alignment.CenterHorizontally), color = colors.accent)
        } else if (g.all { it.snippets.isEmpty() }) {
            Text(t("No snippets"), color = colors.secondary, fontSize = 15.sp, modifier = Modifier.padding(16.dp, 22.dp))
        } else {
            for (group in g) {
                if (group.snippets.isEmpty()) continue
                Text((group.name ?: t("My snippets")).uppercase(), color = colors.secondary, fontSize = 13.sp, modifier = Modifier.padding(start = 16.dp, top = 18.dp, bottom = 6.dp))
                Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(colors.cell)) {
                    group.snippets.forEachIndexed { i, s ->
                        Cell(last = i == group.snippets.lastIndex, onClick = { pick(BarKey(BarKey.Kind.Snippet, label = s.name, value = s.command)) }) {
                            CellText(s.name, s.command, mono = true)
                        }
                    }
                }
            }
        }
        Text(t("Text").uppercase(), color = colors.secondary, fontSize = 13.sp, modifier = Modifier.padding(start = 16.dp, top = 18.dp, bottom = 6.dp))
        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(colors.cell)) {
            FieldCell(text, { text = it }, t("Text to type"), mono = true, plain = true)
            FieldCell(label, { label = it }, t("Label on the key"))
            Cell(last = true, onClick = if (text.isNotEmpty()) ({ pick(BarKey(BarKey.Kind.Text, label = label, value = text)) }) else null) {
                Text(t("Use"), color = if (text.isNotEmpty()) colors.accent else colors.tertiary, fontSize = 17.sp)
            }
        }
        Text(t("\\n in the text presses Enter"), color = colors.secondary, fontSize = 13.sp, modifier = Modifier.padding(start = 16.dp, top = 6.dp))
    }
}
