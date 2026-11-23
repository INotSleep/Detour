package dev.detour.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Keyboard
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.MainTab
import dev.detour.android.core.Biometry
import dev.detour.android.core.Core
import dev.detour.android.core.t
import dev.detour.android.keys.KeyboardEditor
import dev.detour.android.terminal.AppAppearance
import dev.detour.android.terminal.TerminalSettings
import kotlinx.coroutines.launch

private enum class SettingsPage { AppTheme, TerminalTheme, Keyboard, Sync }

/** Settings: the app's and the terminal's look, the keyboard, sync, the fingerprint and locking. */
@Composable
fun SettingsScreen() {
    var page by remember { mutableStateOf<SettingsPage?>(null) }
    var biometry by remember { mutableStateOf(Biometry.isSaved) }
    var askPassword by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val active = AppModel.tab == MainTab.Settings
    val version = remember { runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "" }
    val asSystem = t("As the system")
    val asApp = t("As the app")

    // Turned on from the offer after unlocking, meanwhile.
    LaunchedEffect(active, AppModel.offerBiometry) { biometry = Biometry.isSaved }

    fun turnOn(pw: String) = scope.launch {
        try {
            // Checked by the core before it is kept.
            Core.call("vault_unlock", mapOf("masterPassword" to pw))
            biometry = Biometry.save(pw)
            error = if (biometry) null else t("Couldn't save it on this device")
        } catch (_: Exception) {
            error = t("Wrong master password")
        }
    }

    Box(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.fillMaxSize()) {
            item { LargeTitle(t("Settings")) }
            section(t("Appearance"), key = "appearance") {
                ChevronCell(t("App theme"), themeName(AppAppearance.themeId, asSystem)) { page = SettingsPage.AppTheme }
                ToggleCell(t("Haptic feedback"), dev.detour.android.core.Haptics.enabled, last = true) { dev.detour.android.core.Haptics.set(it) }
            }
            section(t("Terminal"), key = "terminal") {
                ChevronCell(t("Theme"), themeName(TerminalSettings.themeId, asApp)) { page = SettingsPage.TerminalTheme }
                Cell {
                    Text(t("Font size"), color = colors.label, fontSize = 17.sp, modifier = Modifier.weight(1f))
                    Text("${TerminalSettings.fontSize.toInt()}", color = colors.secondary, fontSize = 17.sp)
                    Spacer(Modifier.width(12.dp))
                    Stepper(
                        TerminalSettings.fontSize > TerminalSettings.MIN_FONT, TerminalSettings.fontSize < TerminalSettings.MAX_FONT,
                        { TerminalSettings.fontSize -= 1 }, { TerminalSettings.fontSize += 1 },
                    )
                }
                ChevronCell(t("Keyboard"), icon = Icons.Outlined.Keyboard, last = true) { page = SettingsPage.Keyboard }
            }
            section(key = "sync") {
                ChevronCell(t("Sync"), if (AppModel.sync?.enabled == true) t("On") else t("Off"), icon = Icons.Outlined.Sync, last = true) { page = SettingsPage.Sync }
            }
            section(key = "lock") {
                if (Biometry.available) {
                    ToggleCell(t("Unlock with %@", Biometry.name), biometry, last = false) { on ->
                        if (on) askPassword = true else { Biometry.delete(); biometry = false }
                    }
                    error?.let { ErrorCell(it, last = false) }
                }
                ButtonCell(t("Lock"), icon = Icons.Outlined.Lock, last = true) { scope.launch { AppModel.lock() } }
            }
            section(key = "version") { ValueCell(t("Version"), version, last = true) }
            item { Spacer(Modifier.height(40.dp)) }
        }
        TitleBar()

        BackHandler(active && page != null) { page = null }
        Pushed(page) { p ->
            when (p) {
                SettingsPage.AppTheme -> ThemePickerPage(t("App theme"), AppAppearance.themeId, asSystem, t("Settings"), { page = null }) { AppAppearance.themeId = it }
                SettingsPage.TerminalTheme -> ThemePickerPage(t("Terminal theme"), TerminalSettings.themeId, asApp, t("Settings"), { page = null }) { TerminalSettings.themeId = it }
                SettingsPage.Keyboard -> Box(Modifier.fillMaxSize().background(colors.background)) { KeyboardEditor { page = null } }
                SettingsPage.Sync -> SyncPage(t("Settings")) { page = null }
            }
        }
    }

    if (askPassword) PasswordDialog({ askPassword = false }) { askPassword = false; turnOn(it) }
}

/** The master password asked again, to keep it behind the fingerprint. */
@Composable
private fun PasswordDialog(dismiss: () -> Unit, done: (String) -> Unit) {
    var password by remember { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text(t("Master password")) },
        text = {
            Field(password, { password = it }, t("Master password"), secure = true, modifier = Modifier.focusRequester(focus), ime = ImeAction.Go, onDone = { if (password.isNotEmpty()) done(password) })
        },
        confirmButton = { TextButton({ done(password) }, enabled = password.isNotEmpty()) { Text(t("Turn on")) } },
        dismissButton = { TextButton(dismiss) { Text(t("Cancel")) } },
        // The field is drawn on the cell colour.
        containerColor = colors.background,
        titleContentColor = colors.label,
    )
}
