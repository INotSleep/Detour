package dev.detour.android.ui

import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.ContentPaste
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.SshKey
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer

/** A public key, then Copy and Share: the rows of its section. */
@Composable
internal fun PublicKeyRows(publicKey: String) {
    var copied by remember { mutableStateOf(false) }
    val haptic = LocalHapticFeedback.current
    val context = LocalContext.current
    Column {
        SelectionContainer(Modifier.padding(horizontal = 16.dp, vertical = 11.dp)) {
            Text(publicKey, color = colors.label, fontSize = 12.sp, fontFamily = FontFamily.Monospace, maxLines = 6, overflow = TextOverflow.Ellipsis)
        }
        HorizontalDivider(Modifier.padding(start = 16.dp), thickness = 0.5.dp, color = colors.separator)
    }
    Cell(onClick = {
        Clipboard.copy(publicKey)
        copied = true
        haptic.performHapticFeedback(HapticFeedbackType.Confirm)
    }) {
        CellIcon(if (copied) Icons.Outlined.Check else Icons.Outlined.ContentCopy)
        Text(if (copied) t("Copied") else t("Copy the public key"), color = colors.accent, fontSize = 17.sp)
    }
    Cell(last = true, onClick = {
        val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, publicKey + "\n")
        context.startActivity(Intent.createChooser(send, null))
    }) {
        CellIcon(Icons.Outlined.Share)
        Text(t("Share"), color = colors.accent, fontSize = 17.sp)
    }
}

/** A new Ed25519 key, its public key shown at once to put on a server. */
@Composable
internal fun NewKeySheet(close: () -> Unit, changed: suspend () -> Unit) {
    var name by remember { mutableStateOf("android") }
    var made by remember { mutableStateOf<SshKey?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    fun make() {
        val n = name.trim()
        if (n.isEmpty() || busy) return
        busy = true
        error = null
        scope.launch {
            try {
                Core.call("key_generate", mapOf("vaultId" to "personal", "name" to n, "algorithm" to "ed25519", "remember" to true))
                val keys = VaultKeys.load().keys
                made = keys.firstOrNull { it.name == n } ?: keys.lastOrNull()
                changed()
            } catch (f: CoreFailure) {
                error = f["message"] ?: f.kind
            } catch (e: Exception) {
                error = e.message
            }
            busy = false
        }
    }

    Sheet(close) {
        val m = made
        SheetBar(
            m?.name ?: t("New key"),
            left = if (m == null) ({ BarButton(t("Cancel"), onClick = close) }) else null,
            right = {
                when {
                    m != null -> BarButton(t("Done"), bold = true, onClick = close)
                    busy -> CircularProgressIndicator(color = colors.accent, strokeWidth = 2.dp, modifier = Modifier.size(22.dp))
                    else -> BarButton(t("Create"), bold = true, enabled = name.isNotBlank()) { make() }
                }
            },
        )
        LazyColumn(Modifier.fillMaxSize().imePadding()) {
            if (m != null) {
                section(t("Public key")) { PublicKeyRows(m.publicKey) }
            } else {
                section(footer = "Ed25519") {
                    FieldCell(name, { name = it }, t("Name"), last = true, plain = true, ime = ImeAction.Done, onDone = { make() })
                }
                error?.let { e -> item { Text(e, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(32.dp, 8.dp)) } }
            }
        }
    }
}

/** A private key pasted or picked from the device's files. */
@Composable
internal fun ImportKeySheet(close: () -> Unit, changed: suspend () -> Unit) {
    var name by remember { mutableStateOf("") }
    var text by remember { mutableStateOf("") }
    var passphrase by remember { mutableStateOf("") }
    var needPassphrase by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val passphraseFocus = remember { FocusRequester() }

    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            val (s, file) = withContext(Dispatchers.IO) { readKeyFile(context, uri) }
            if (s != null) {
                text = s
                if (name.isEmpty() && file != null) name = file.substringBeforeLast('.').ifEmpty { file }
            }
        }
    }

    LaunchedEffect(needPassphrase) {
        if (needPassphrase) runCatching { passphraseFocus.requestFocus() }
    }

    fun save() {
        if (text.isEmpty() || busy) return
        busy = true
        error = null
        val n = name.trim()
        scope.launch {
            try {
                Core.call(
                    "key_import",
                    mapOf(
                        "vaultId" to "personal", "name" to n.ifEmpty { "imported" }, "text" to text,
                        "passphrase" to if (needPassphrase && passphrase.isNotEmpty()) passphrase else null, "remember" to true,
                    ),
                )
                changed()
                close()
            } catch (f: CoreFailure) {
                when (f.kind) {
                    "passphraseRequired" -> needPassphrase = true
                    "wrongPassphrase" -> { needPassphrase = true; error = t("Wrong passphrase") }
                    "invalidKey" -> error = t("This isn't a private key")
                    else -> error = f["message"] ?: f.kind
                }
            } catch (e: Exception) {
                error = e.message
            }
            busy = false
        }
    }

    Sheet(close) {
        SheetBar(
            t("Key import"),
            left = { BarButton(t("Cancel"), onClick = close) },
            right = {
                if (busy) CircularProgressIndicator(color = colors.accent, strokeWidth = 2.dp, modifier = Modifier.size(22.dp))
                else BarButton(t("Add"), bold = true, enabled = text.isNotEmpty()) { save() }
            },
        )
        LazyColumn(Modifier.fillMaxSize().imePadding()) {
            section {
                FieldCell(name, { name = it }, t("Name"), last = true, plain = true)
            }
            section(t("Private key")) {
                BasicTextField(
                    text, { text = it },
                    modifier = Modifier.fillMaxWidth().heightIn(min = 140.dp).padding(horizontal = 16.dp, vertical = 11.dp),
                    textStyle = TextStyle(color = colors.label, fontSize = 12.sp, fontFamily = FontFamily.Monospace),
                    cursorBrush = SolidColor(colors.accent),
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Ascii, capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false,
                    ),
                )
                HorizontalDivider(Modifier.padding(start = 16.dp), thickness = 0.5.dp, color = colors.separator)
                Row(Modifier.fillMaxWidth()) {
                    Box(Modifier.weight(1f)) {
                        Cell(last = true, onClick = { Clipboard.paste()?.let { text = it } }) {
                            CellIcon(Icons.Outlined.ContentPaste)
                            Text(t("Paste"), color = colors.accent, fontSize = 17.sp)
                        }
                    }
                    Box(Modifier.weight(1f)) {
                        Cell(last = true, onClick = { pick.launch(arrayOf("*/*")) }) {
                            Row(Modifier.weight(1f), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
                                CellIcon(Icons.Outlined.Folder)
                                Text(t("From Files"), color = colors.accent, fontSize = 17.sp)
                            }
                        }
                    }
                }
            }
            if (needPassphrase) {
                section {
                    FieldCell(
                        passphrase, { passphrase = it }, t("Passphrase"), last = true, secure = true,
                        modifier = Modifier.focusRequester(passphraseFocus), ime = ImeAction.Done, onDone = { save() },
                    )
                }
            }
            error?.let { e -> item { Text(e, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(32.dp, 8.dp)) } }
        }
    }
}

/** A picked file's text, if it is UTF-8 and small enough to be a key, and its name. */
private fun readKeyFile(context: android.content.Context, uri: Uri): Pair<String?, String?> {
    val file = runCatching {
        context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    }.getOrNull()
    val text = runCatching {
        val bytes = context.contentResolver.openInputStream(uri)?.use { input ->
            val out = java.io.ByteArrayOutputStream()
            val buf = ByteArray(8192)
            while (out.size() <= MAX_KEY_FILE) {
                val n = input.read(buf)
                if (n < 0) break
                out.write(buf, 0, n)
            }
            out.toByteArray()
        } ?: return@runCatching null
        if (bytes.size > MAX_KEY_FILE) null else Charsets.UTF_8.newDecoder().decode(ByteBuffer.wrap(bytes)).toString()
    }.getOrNull()
    return text to file
}

// A key file is a few kilobytes; anything far bigger is the wrong file.
private const val MAX_KEY_FILE = 1 shl 20
