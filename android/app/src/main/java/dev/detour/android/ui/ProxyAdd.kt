package dev.detour.android.ui

import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.HorizontalDivider
import androidx.compose.ui.Alignment
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ContentPaste
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import dev.detour.android.terminal.TerminalSession
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private enum class ProxyKind(val title: String) {
    Vless("VLESS"), Wireguard("WireGuard"), Socks5("SOCKS5"), Http("HTTP");

    /** The kind's `type` in the vault. */
    val raw: String get() = name.lowercase()
}

/**
 * A new proxy, by kind: VLESS from its link, WireGuard from its config (pasted or a .conf
 * file), SOCKS5 and HTTP from their address and login.
 */
@Composable
fun AddProxySheet(added: suspend () -> Unit, close: () -> Unit) {
    var kind by remember { mutableStateOf(ProxyKind.Vless) }
    var text by remember { mutableStateOf("") }
    var name by remember { mutableStateOf("") }
    var address by remember { mutableStateOf("") }
    var port by remember { mutableStateOf("") }
    var user by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current

    val portNumber = port.toIntOrNull()?.takeIf { it in 0..65535 }
    val ready = when (kind) {
        ProxyKind.Vless, ProxyKind.Wireguard -> text.isNotBlank()
        ProxyKind.Socks5, ProxyKind.Http -> address.isNotBlank() && portNumber != null
    }

    LaunchedEffect(kind) { error = null }

    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            val read = withContext(Dispatchers.IO) {
                runCatching {
                    val body = context.contentResolver.openInputStream(uri)!!.use { it.readBytes().toString(Charsets.UTF_8) }
                    val file = context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                        if (c.moveToFirst()) c.getString(0) else null
                    }
                    body to file
                }.getOrNull()
            } ?: return@launch
            text = read.first
            if (name.isEmpty()) read.second?.let { name = it.substringBeforeLast('.') }
        }
    }

    fun add() {
        if (!ready || busy) return
        busy = true
        error = null
        scope.launch {
            try {
                val proxy = mutableMapOf<String, Any?>("id" to PersonalVault.newId())
                when (kind) {
                    ProxyKind.Vless, ProxyKind.Wireguard -> {
                        val parsed = Core.call("proxy_parse", mapOf("text" to text.trim())).obj()
                        val given = name.trim()
                        proxy["name"] = given.ifEmpty { parsed.str("name") ?: kind.title }
                        proxy["kind"] = parsed["kind"] ?: emptyMap<String, Any?>()
                        proxy["newSecrets"] = parsed["secrets"] ?: emptyMap<String, Any?>()
                    }
                    ProxyKind.Socks5, ProxyKind.Http -> {
                        val host = address.trim()
                        val given = name.trim()
                        proxy["name"] = given.ifEmpty { host }
                        proxy["kind"] = mapOf("type" to kind.raw, "address" to host, "port" to (portNumber ?: 0), "user" to user.trim())
                        proxy["newSecrets"] = if (password.isEmpty()) emptyMap() else mapOf("password" to password)
                    }
                }
                PersonalVault.edit { v -> v["proxies"] = v["proxies"].list() + listOf(proxy) }
                added()
                close()
            } catch (f: CoreFailure) {
                error = if (f.kind == "proxy") reason(f["reason"] ?: "") else TerminalSession.describe(f)
            } catch (e: Exception) {
                error = e.message ?: e.toString()
            }
            busy = false
        }
    }

    Sheet(close) {
        SheetBar(
            t("Add proxy"),
            left = { BarButton(t("Cancel"), onClick = close) },
            right = { if (busy) CircularProgressIndicator(color = colors.accent) else BarButton(t("Add"), bold = true, enabled = ready) { add() } },
        )
        LazyColumn(Modifier.fillMaxSize().imePadding()) {
            section {
                Box(Modifier.padding(12.dp)) {
                    Segmented(ProxyKind.entries.map { it to it.title }, kind) { kind = it }
                }
            }
            when (kind) {
                ProxyKind.Vless -> section(footer = t("The vless:// link from your server's panel or provider"), key = "vless") {
                    TextCell(text, { text = it }, "vless://…", minHeight = 76, maxHeight = 180, ime = ImeAction.Done, onDone = { add() })
                    ActionCell(Icons.Outlined.ContentPaste, t("Paste"), last = true) { Clipboard.paste()?.let { text = it } }
                }
                ProxyKind.Wireguard -> section(footer = t("A WireGuard config: [Interface] and one [Peer]"), key = "wireguard") {
                    // Enter makes a new line here: a config has many.
                    TextCell(text, { text = it }, "", minHeight = 160, maxHeight = 320, ime = ImeAction.Default)
                    ActionCell(Icons.Outlined.ContentPaste, t("Paste")) { Clipboard.paste()?.let { text = it } }
                    ActionCell(Icons.Outlined.Description, t("From a file"), last = true) { picker.launch(arrayOf("*/*")) }
                }
                ProxyKind.Socks5, ProxyKind.Http -> {
                    section(key = "address") {
                        FieldCell(name, { name = it }, t("Name"))
                        FieldCell(address, { address = it }, t("Address"), plain = true)
                        FieldCell(port, { port = it }, if (kind == ProxyKind.Socks5) "1080" else "8080", number = true, last = true)
                    }
                    section(header = t("Login, if the proxy asks for one"), key = "login") {
                        FieldCell(user, { user = it }, t("User"), plain = true)
                        FieldCell(password, { password = it }, t("Password"), secure = true, last = true, ime = ImeAction.Done, onDone = { add() })
                    }
                }
            }
            error?.let { e -> item { Text(e, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(32.dp, 8.dp)) } }
        }
    }
}

/** A text of several lines as a row of a form, from its top: a link or a config. */
@Composable
private fun TextCell(value: String, onChange: (String) -> Unit, placeholder: String, minHeight: Int, maxHeight: Int, ime: ImeAction, onDone: () -> Unit = {}) {
    val style = TextStyle(color = colors.label, fontSize = 13.sp, fontFamily = FontFamily.Monospace)
    Column {
        BasicTextField(
            value, onChange,
            modifier = Modifier.fillMaxWidth().heightIn(min = minHeight.dp, max = maxHeight.dp).padding(horizontal = 16.dp, vertical = 11.dp),
            textStyle = style,
            cursorBrush = SolidColor(colors.accent),
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Uri, capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, imeAction = ime,
            ),
            keyboardActions = KeyboardActions(onDone = { onDone() }),
            decorationBox = { inner ->
                Box(contentAlignment = Alignment.TopStart) {
                    if (value.isEmpty()) Text(placeholder, style = style.copy(color = colors.tertiary))
                    inner()
                }
            },
        )
        HorizontalDivider(Modifier.padding(start = 16.dp), thickness = 0.5.dp, color = colors.separator)
    }
}

/** A button as a row of a form: its icon and title in the accent color. */
@Composable
private fun ActionCell(icon: ImageVector, title: String, last: Boolean = false, onClick: () -> Unit) {
    Cell(last = last, onClick = onClick, inset = 52) {
        CellIcon(icon)
        Text(title, color = colors.accent, fontSize = 17.sp)
    }
}

/** What the core found wrong with a link or a config. */
private fun reason(r: String): String = when {
    r == "protocol" -> t("This isn't a vless:// link or a WireGuard config")
    r.startsWith("unsupported:") -> t("Detour can't do this yet: %@", r.removePrefix("unsupported:"))
    else -> t("Detour can't read this proxy")
}
