package dev.detour.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.VaultData
import dev.detour.android.core.SshKey
import dev.detour.android.core.bool
import dev.detour.android.core.int
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.core.t
import kotlinx.coroutines.launch

/**
 * Adds a host to a vault (the personal one or a team's), or changes one. Fields the phone doesn't show (proxy, tags,
 * agent forwarding...) are kept as they are.
 */
@Composable
fun HostEditor(hostId: String?, vault: String = VaultData.PERSONAL, close: () -> Unit) {
    var original by remember { mutableStateOf<Map<String, Any?>?>(null) }
    var label by remember { mutableStateOf("") }
    var address by remember { mutableStateOf("") }
    var port by remember { mutableStateOf("22") }
    var user by remember { mutableStateOf("") }
    var group by remember { mutableStateOf("") }
    var auth by remember { mutableStateOf("password") }
    var keyId by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var hasPassword by remember { mutableStateOf(false) }
    var keys by remember { mutableStateOf<List<SshKey>>(emptyList()) }
    var loaded by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val focus = remember { FocusRequester() }

    val portNumber = port.trim().toIntOrNull()?.takeIf { it in 1..65535 }
    val ready = loaded && address.isNotBlank() && user.isNotBlank() && portNumber != null && (auth != "key" || keyId.isNotEmpty())

    LaunchedEffect(Unit) {
        val v = runCatching { VaultData.get(vault) }.getOrDefault(emptyMap())
        keys = v["keys"].list().map { SshKey.from(it.obj()) }
        val h = hostId?.let { id -> v["hosts"].list().map { it.obj() }.firstOrNull { it.str("id") == id } }
        if (h != null) {
            original = h
            label = h.str("label") ?: ""
            address = h.str("address") ?: ""
            port = (h.int("port") ?: 22).toString()
            user = h.str("user") ?: ""
            group = h.str("group") ?: ""
            hasPassword = h.bool("hasPassword") ?: false
            val a = h["auth"].obj()
            auth = a.str("type") ?: "password"
            keyId = a.str("keyId") ?: ""
        } else {
            runCatching { focus.requestFocus() }
        }
        if (keyId.isEmpty()) keyId = keys.firstOrNull()?.id ?: ""
        loaded = true
    }

    fun save() {
        if (!ready || busy) return
        busy = true
        error = null
        val a = address.trim()
        val h = (original ?: mapOf("id" to PersonalVault.newId(), "tags" to emptyList<String>())).toMutableMap()
        h["address"] = a
        h["label"] = label.trim().ifEmpty { a }
        h["port"] = portNumber
        h["user"] = user.trim()
        h["group"] = group.trim()
        when (auth) {
            "key" -> h["auth"] = mapOf("type" to "key", "keyId" to keyId)
            "agent" -> h["auth"] = mapOf("type" to "agent")
            else -> {
                h["auth"] = mapOf("type" to "password")
                if (password.isNotEmpty()) h["newPassword"] = password
            }
        }
        scope.launch {
            try {
                VaultData.save(vault, "hosts", h)
                AppModel.reload()
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
            if (hostId == null) t("New host") else t("Edit host"),
            left = { BarButton(t("Cancel"), onClick = close) },
            right = { if (busy) CircularProgressIndicator(color = colors.accent) else BarButton(t("Save"), bold = true, enabled = ready) { save() } },
        )
        LazyColumn(Modifier.fillMaxSize().imePadding()) {
            section {
                FieldCell(address, { address = it }, t("Address"), plain = true, modifier = Modifier.focusRequester(focus))
                FieldCell(user, { user = it }, t("User"), plain = true)
                FieldCell(port, { port = it }, t("Port"), number = true)
                FieldCell(label, { label = it }, address.trim().ifEmpty { t("Name") }, last = true)
            }
            section {
                Box(Modifier.padding(12.dp)) {
                    Segmented(
                        buildList {
                            add("password" to t("Password"))
                            if (keys.isNotEmpty()) add("key" to t("Key"))
                            if (auth == "agent") add("agent" to "ssh-agent")
                        },
                        auth,
                    ) { auth = it }
                }
                if (auth == "password") {
                    FieldCell(password, { password = it }, if (hasPassword) t("Saved; type to change") else t("Ask when connecting"), secure = true, last = true)
                } else if (auth == "key") {
                    keys.forEachIndexed { i, k ->
                        Cell(last = i == keys.lastIndex, onClick = { keyId = k.id }) {
                            CellText(k.name, k.kind)
                            if (k.id == keyId) Text("✓", color = colors.accent, fontSize = 17.sp)
                        }
                    }
                }
            }
            section {
                FieldCell(group, { group = it }, t("Group"), last = true, ime = androidx.compose.ui.text.input.ImeAction.Done, onDone = { save() })
            }
            error?.let { e -> item { Text(e, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(32.dp, 8.dp)) } }
        }
    }
}
