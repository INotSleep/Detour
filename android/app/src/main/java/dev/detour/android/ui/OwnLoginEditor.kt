package dev.detour.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
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
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.Host
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.SshKey
import dev.detour.android.core.bool
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.core.t
import kotlinx.coroutines.launch

/**
 * This person's own login for a team host: kept in their personal vault and used instead of the
 * team's shared one while it is on.
 */
@Composable
fun OwnLoginEditor(host: Host, close: () -> Unit) {
    var exists by remember { mutableStateOf(false) }
    var active by remember { mutableStateOf(true) }
    var user by remember { mutableStateOf("") }
    var auth by remember { mutableStateOf("password") }
    var keyId by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var hasPassword by remember { mutableStateOf(false) }
    var keys by remember { mutableStateOf<List<SshKey>>(emptyList()) }
    var loaded by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val ready = loaded && user.isNotBlank() && (auth != "key" || keyId.isNotEmpty())

    LaunchedEffect(Unit) {
        keys = runCatching { PersonalVault.get()["keys"].list().map { SshKey.from(it.obj()) } }.getOrDefault(emptyList())
        val teams = runCatching { Core.call("teams_list").list() }.getOrDefault(emptyList())
        val view = teams.map { it.obj()["view"].obj() }.firstOrNull { it.str("id") == host.vaultId }
        val own = view?.get("data").obj()["hosts"].list().map { it.obj() }.firstOrNull { it.str("id") == host.id }?.get("own")?.obj()
        if (own != null) {
            exists = true
            active = own.bool("active") ?: true
            user = own.str("user") ?: ""
            hasPassword = own.bool("hasPassword") ?: false
            val a = own["auth"].obj()
            auth = if (a.str("type") == "key") "key" else "password"
            keyId = a.str("keyId") ?: ""
        }
        if (keyId.isEmpty()) keyId = keys.firstOrNull()?.id ?: ""
        loaded = true
    }

    fun set(login: Map<String, Any?>?) {
        busy = true
        error = null
        scope.launch {
            try {
                // `login` null removes it: sent as JSON null, which Core.call would leave out.
                Core.call("team_own_login_set", mapOf("id" to host.vaultId, "hostId" to host.id, "login" to (login ?: org.json.JSONObject.NULL)))
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

    fun save() {
        if (!ready || busy) return
        val login = mutableMapOf<String, Any?>(
            "user" to user.trim(),
            "auth" to if (auth == "key") mapOf("type" to "key", "keyId" to keyId) else mapOf("type" to "password"),
            "active" to if (exists) active else true,
        )
        if (auth == "password" && password.isNotEmpty()) login["newPassword"] = password
        set(login)
    }

    Sheet(close) {
        SheetBar(
            host.label,
            left = { BarButton(t("Cancel"), onClick = close) },
            right = { if (busy) CircularProgressIndicator(Modifier.size(22.dp), color = colors.accent) else BarButton(t("Save"), bold = true, enabled = ready) { save() } },
        )
        LazyColumn(Modifier.fillMaxSize().imePadding()) {
            if (exists) {
                section(footer = if (!active) t("The team's login is used: %@", host.user) else null, key = "active") {
                    ToggleCell(t("Log in with my own login"), active, last = true) { active = it }
                }
            }
            section(key = "login") {
                FieldCell(user, { user = it }, t("User"), plain = true)
                Box(Modifier.padding(12.dp)) {
                    Segmented(
                        buildList {
                            add("password" to t("Password"))
                            if (keys.isNotEmpty()) add("key" to t("Key"))
                        },
                        auth,
                    ) { auth = it }
                }
                if (auth == "password") {
                    FieldCell(
                        password, { password = it }, if (hasPassword) t("Saved; type to change") else t("Ask when connecting"),
                        secure = true, last = true, ime = ImeAction.Done, onDone = { save() },
                    )
                } else {
                    keys.forEachIndexed { i, k ->
                        Cell(last = i == keys.lastIndex, onClick = { keyId = k.id }) {
                            CellText(k.name, k.kind)
                            if (k.id == keyId) Text("✓", color = colors.accent, fontSize = 17.sp)
                        }
                    }
                }
            }
            if (exists) {
                section(key = "remove") {
                    ButtonCell(t("Remove my login"), destructive = true, last = true) { set(null) }
                }
            }
            errorItem(error)
        }
    }
}
