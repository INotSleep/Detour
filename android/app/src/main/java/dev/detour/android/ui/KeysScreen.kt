package dev.detour.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBackIos
import androidx.compose.material.icons.automirrored.outlined.ArrowForwardIos
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.FileDownload
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.MainTab
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.SshKey
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.core.t
import kotlinx.coroutines.launch

/** The vault's keys and which hosts log in with each. */
internal class VaultKeys(val keys: List<SshKey>, private val hosts: List<Map<String, Any?>>) {
    fun users(of: SshKey): List<String> = hosts.filter {
        val auth = it["auth"].obj()
        auth.str("type") == "key" && auth.str("keyId") == of.id
    }.map { it.str("label") ?: "" }

    companion object {
        suspend fun load(): VaultKeys {
            val v = PersonalVault.get()
            return VaultKeys(v["keys"].list().map { SshKey.from(it.obj()) }, v["hosts"].list().map { it.obj() })
        }
    }
}

/** The personal vault's SSH keys: make one, import one, copy or share its public key, delete it. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeysScreen() {
    var vault by remember { mutableStateOf<VaultKeys?>(null) }
    var failed by remember { mutableStateOf<String?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    var making by remember { mutableStateOf(false) }
    var importing by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf(false) }
    /** The key shown in full, by id. */
    var open by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val title = MainTab.Keys.title

    suspend fun load() {
        try {
            vault = VaultKeys.load()
            failed = null
        } catch (f: CoreFailure) {
            if (vault == null) failed = f["message"] ?: f.kind
        } catch (e: Exception) {
            if (vault == null) failed = e.message
        }
    }

    // Again after every reload of the hosts: a sync may have brought keys, or hosts that use them.
    LaunchedEffect(AppModel.sections) { load() }

    val key = open?.let { id -> vault?.keys?.firstOrNull { it.id == id } }

    Box(Modifier.fillMaxSize()) {
        PullToRefreshBox(
            refreshing,
            { scope.launch { refreshing = true; load(); refreshing = false } },
            Modifier.fillMaxSize().then(if (key != null) Modifier.blockTouches() else Modifier),
        ) {
            LazyColumn(Modifier.fillMaxSize()) {
                item {
                    LargeTitle(title) {
                        Box {
                            BarIcon(Icons.Outlined.Add, t("Add")) { menu = true }
                            DropdownMenu(menu, { menu = false }) {
                                DropdownMenuItem(
                                    { Text(t("New key")) }, { menu = false; making = true },
                                    leadingIcon = { Icon(Icons.Outlined.Add, null) },
                                )
                                DropdownMenuItem(
                                    { Text(t("Import a key")) }, { menu = false; importing = true },
                                    leadingIcon = { Icon(Icons.Outlined.FileDownload, null) },
                                )
                            }
                        }
                    }
                }
                val v = vault
                when {
                    v == null && failed != null -> item {
                        Text(failed ?: "", color = colors.red, fontSize = 15.sp, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(32.dp, 120.dp))
                    }
                    v == null -> item {
                        Box(Modifier.fillMaxWidth().padding(top = 160.dp), contentAlignment = Alignment.Center) {
                            CircularProgressIndicator(color = colors.secondary, strokeWidth = 2.dp, modifier = Modifier.size(28.dp))
                        }
                    }
                    v.keys.isEmpty() -> item {
                        Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                            EmptyState(Icons.Outlined.Key, t("No keys"), t("New key")) { making = true }
                            Spacer(Modifier.height(8.dp))
                            BarButton(t("Import a key")) { importing = true }
                        }
                    }
                    else -> section {
                        v.keys.forEachIndexed { i, k ->
                            Cell(last = i == v.keys.lastIndex, onClick = { open = k.id }) {
                                CellText(k.name, k.kind + if (k.hasPassphrase) " · " + t("passphrase") else "")
                                Icon(Icons.AutoMirrored.Outlined.ArrowForwardIos, null, tint = colors.tertiary, modifier = Modifier.size(14.dp))
                            }
                        }
                    }
                }
                item { Spacer(Modifier.height(80.dp)) }
            }
            TitleBar()
        }

        // The key pushed over the list, as iOS's navigation does; it stays drawn while it slides out.
        val shown = remember { arrayOfNulls<SshKey>(1) }
        if (key != null) shown[0] = key
        AnimatedVisibility(key != null, enter = slideInHorizontally { it }, exit = slideOutHorizontally { it }) {
            shown[0]?.let { k ->
                KeyDetail(k, vault?.users(k) ?: emptyList(), back = title, close = { open = null }, changed = { load() })
            }
        }
    }

    if (making) NewKeySheet(close = { making = false }, changed = { load() })
    if (importing) ImportKeySheet(close = { importing = false }, changed = { load() })
}

@Composable
private fun KeyDetail(key: SshKey, users: List<String>, back: String, close: () -> Unit, changed: suspend () -> Unit) {
    var confirmDelete by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    BackHandler(onBack = close)

    fun delete() {
        if (busy) return
        busy = true
        error = null
        scope.launch {
            try {
                PersonalVault.remove("keys", key.id)
                changed()
                close()
            } catch (f: CoreFailure) {
                error = if (f.kind == "keyInUse") t("Hosts log in with this key: %@", users.joinToString(", ")) else f["message"] ?: f.kind
            } catch (e: Exception) {
                error = e.message
            }
            busy = false
        }
    }

    Column(Modifier.fillMaxSize().background(colors.background)) {
        Box(Modifier.windowInsetsPadding(WindowInsets.statusBars)) {
            SheetBar(key.name, left = { BackButton(back, close) })
        }
        LazyColumn(Modifier.fillMaxSize()) {
            section(t("Public key")) { PublicKeyRows(key.publicKey) }
            section {
                InfoRow(t("Type"), key.kind)
                InfoRow(t("Fingerprint"), key.fingerprint, mono = true, last = users.isEmpty())
                if (users.isNotEmpty()) InfoRow(t("Hosts"), users.joinToString(", "), last = true)
            }
            section {
                Cell(last = error == null, onClick = { confirmDelete = true }) {
                    Text(t("Delete the key"), color = colors.red, fontSize = 17.sp, modifier = Modifier.weight(1f))
                    if (busy) CircularProgressIndicator(color = colors.secondary, strokeWidth = 2.dp, modifier = Modifier.size(18.dp))
                }
                error?.let { Text(it, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(16.dp, 10.dp)) }
            }
            item { Spacer(Modifier.height(80.dp)) }
        }
    }

    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text(t("Delete %@?", key.name)) },
            confirmButton = { TextButton({ confirmDelete = false; delete() }) { Text(t("Delete"), color = colors.red) } },
            dismissButton = { TextButton({ confirmDelete = false }) { Text(t("Cancel"), color = colors.accent) } },
            containerColor = colors.cell,
            titleContentColor = colors.label,
        )
    }
}


/** A title and its value, which may run over several lines. */
@Composable
private fun InfoRow(title: String, value: String, mono: Boolean = false, last: Boolean = false) {
    Cell(last = last) {
        Text(title, color = colors.label, fontSize = 17.sp)
        Spacer(Modifier.width(16.dp))
        Text(
            value, color = colors.secondary, textAlign = TextAlign.End, modifier = Modifier.weight(1f),
            fontSize = if (mono) 12.sp else 17.sp, fontFamily = if (mono) FontFamily.Monospace else null,
        )
    }
}
