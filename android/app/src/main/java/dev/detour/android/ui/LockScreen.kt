package dev.detour.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Terminal
import androidx.compose.material.icons.outlined.Fingerprint
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.core.Biometry
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.t
import kotlinx.coroutines.launch

/** The vault's first password, or unlocking it (the fingerprint when it is on). */
@Composable
fun LockScreen(setup: Boolean) {
    var password by remember { mutableStateOf("") }
    var repeat by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val focus = remember { FocusRequester() }

    fun go() {
        if (password.isEmpty() || busy) return
        if (setup) {
            if (password.length < 8) { error = t("At least 8 characters"); return }
            if (password != repeat) { error = t("The passwords differ"); return }
        }
        busy = true
        error = null
        scope.launch {
            try {
                if (setup) AppModel.create(password) else AppModel.unlock(password)
            } catch (f: CoreFailure) {
                error = if (f.kind == "wrongMasterPassword") t("Wrong master password") else f["message"] ?: f.kind
                password = ""
            } catch (e: Exception) {
                error = e.message
            }
            busy = false
        }
    }

    val context = LocalContext.current

    fun biometry() {
        val activity = context.fragmentActivity() ?: return
        scope.launch {
            val saved = Biometry.load(activity, t("Unlock Detour"))
            if (saved == null) {
                runCatching { focus.requestFocus() }
                return@launch
            }
            busy = true
            try {
                AppModel.unlock(saved, typed = false)
            } catch (_: Exception) {
                // Changed on another device: the password once more.
                Biometry.delete()
                error = t("Enter the master password")
                runCatching { focus.requestFocus() }
            }
            busy = false
        }
    }

    LaunchedEffect(Unit) {
        if (!setup && Biometry.isSaved) biometry() else runCatching { focus.requestFocus() }
    }

    Column(
        Modifier.fillMaxSize().imePadding().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(if (setup) Icons.Filled.Terminal else Icons.Filled.Lock, null, tint = colors.accent, modifier = Modifier.size(40.dp))
        Spacer(Modifier.height(18.dp))
        Text(if (setup) t("Create your vault") else t("Detour is locked"), color = colors.label, fontSize = 22.sp, fontWeight = FontWeight.Bold)
        Spacer(Modifier.height(18.dp))
        Column(Modifier.widthIn(max = 360.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Field(password, { password = it }, if (setup) t("New master password") else t("Master password"),
                secure = true, modifier = Modifier.focusRequester(focus),
                ime = if (setup) ImeAction.Next else ImeAction.Go, onDone = { if (!setup) go() })
            if (setup) Field(repeat, { repeat = it }, t("Repeat it"), secure = true, ime = ImeAction.Go, onDone = { go() })
        }
        error?.let {
            Spacer(Modifier.height(12.dp))
            Text(it, color = colors.red, fontSize = 13.sp)
        }
        Spacer(Modifier.height(18.dp))
        ProminentButton(
            if (setup) t("Create") else t("Unlock"),
            Modifier.widthIn(max = 360.dp).fillMaxWidth(),
            enabled = password.isNotEmpty(), busy = busy, onClick = { go() },
        )
        if (!setup && Biometry.isSaved) {
            Spacer(Modifier.height(8.dp))
            TextButton({ biometry() }, enabled = !busy) {
                Icon(Icons.Outlined.Fingerprint, null, tint = colors.accent, modifier = Modifier.size(20.dp))
                Spacer(Modifier.width(8.dp))
                Text(t("Unlock with %@", Biometry.name), color = colors.accent, fontSize = 17.sp)
            }
        }
        Spacer(Modifier.height(120.dp))
    }
}

/** After the first unlock with the password: keep it behind the fingerprint? Asked once. */
@Composable
fun BiometryOffer() {
    if (!AppModel.offerBiometry) return
    val name = Biometry.name
    AlertDialog(
        onDismissRequest = { AppModel.rememberWithBiometry(false) },
        title = { Text(t("Unlock with %@ next time?", name), fontSize = 17.sp, fontWeight = FontWeight.SemiBold) },
        confirmButton = { TextButton({ AppModel.rememberWithBiometry(true) }) { Text(t("Use %@", name)) } },
        dismissButton = { TextButton({ AppModel.rememberWithBiometry(false) }) { Text(t("Not now")) } },
        containerColor = colors.cell,
        titleContentColor = colors.label,
    )
}
