package dev.detour.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.Snippet
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.launch

/** Adds a snippet to the personal vault (`snippet` null), or changes one. */
@Composable
internal fun SnippetEditor(snippet: Snippet?, saved: suspend () -> Unit, close: () -> Unit) {
    var name by remember { mutableStateOf(snippet?.name ?: "") }
    var command by remember { mutableStateOf(snippet?.command ?: "") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val nameFocus = remember { FocusRequester() }
    val commandFocus = remember { FocusRequester() }

    val ready = name.isNotBlank() && command.isNotBlank()

    LaunchedEffect(Unit) {
        if (snippet == null) runCatching { nameFocus.requestFocus() }
    }

    fun save() {
        if (!ready || busy) return
        busy = true
        error = null
        val item = mapOf("id" to (snippet?.id ?: PersonalVault.newId()), "name" to name.trim(), "command" to command)
        scope.launch {
            try {
                PersonalVault.save("snippets", item)
                saved()
                close()
            } catch (f: CoreFailure) {
                error = f["message"] ?: f.kind
            } catch (e: Exception) {
                error = e.message
            }
            busy = false
        }
    }

    Sheet(close) {
        SheetBar(
            if (snippet == null) t("New snippet") else t("Edit snippet"),
            left = { BarButton(t("Cancel"), onClick = close) },
            right = { if (busy) CircularProgressIndicator(color = colors.accent) else BarButton(t("Save"), bold = true, enabled = ready) { save() } },
        )
        LazyColumn(Modifier.fillMaxSize().imePadding()) {
            section {
                FieldCell(name, { name = it }, t("Name"), last = true, modifier = Modifier.focusRequester(nameFocus), onDone = {
                    runCatching { commandFocus.requestFocus() }
                })
            }
            section(t("Command")) {
                BasicTextField(
                    command, { command = it },
                    modifier = Modifier.fillMaxWidth().heightIn(min = 120.dp).focusRequester(commandFocus).padding(horizontal = 16.dp, vertical = 11.dp),
                    textStyle = TextStyle(color = colors.label, fontSize = 17.sp, fontFamily = FontFamily.Monospace),
                    cursorBrush = SolidColor(colors.accent),
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
                )
            }
            error?.let { e -> item { Text(e, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(32.dp, 8.dp)) } }
        }
    }
}

/** A team's snippet: read it, copy it. */
@Composable
internal fun SnippetViewer(snippet: Snippet, close: () -> Unit) {
    var copies by remember { mutableIntStateOf(0) }
    val haptic = LocalHapticFeedback.current

    Sheet(close, full = false) {
        // The name can be long: it stops short of Done instead of running under it.
        Box(Modifier.fillMaxWidth().height(52.dp).padding(horizontal = 16.dp)) {
            Text(
                snippet.name, color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, maxLines = 1,
                overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center,
                modifier = Modifier.align(Alignment.Center).padding(horizontal = 64.dp),
            )
            Box(Modifier.align(Alignment.CenterEnd)) { BarButton(t("Done"), bold = true, onClick = close) }
        }
        LazyColumn(Modifier.fillMaxWidth()) {
            section {
                SelectionContainer {
                    Text(
                        snippet.command, color = colors.label, fontSize = 17.sp, fontFamily = FontFamily.Monospace,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 11.dp),
                    )
                }
            }
            section {
                Cell(last = true, onClick = {
                    Clipboard.copy(snippet.command)
                    copies++
                    haptic.performHapticFeedback(HapticFeedbackType.Confirm)
                }) {
                    Icon(if (copies > 0) Icons.Outlined.Check else Icons.Outlined.ContentCopy, null, tint = colors.accent, modifier = Modifier.size(20.dp))
                    Spacer(Modifier.width(12.dp))
                    Text(if (copies > 0) t("Copied") else t("Copy"), color = colors.accent, fontSize = 17.sp)
                }
            }
            item { Spacer(Modifier.windowInsetsPadding(WindowInsets.navigationBars).height(24.dp)) }
        }
    }
}
