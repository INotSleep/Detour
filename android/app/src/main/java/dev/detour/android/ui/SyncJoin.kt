package dev.detour.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.IosShare
import androidx.compose.material.icons.outlined.QrCodeScanner
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
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.core.Core
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.StorageModel
import dev.detour.android.core.coreErrorText
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.launch

enum class SyncJoinMode { Connect, Create }

/**
 * Sync for this phone: connects it to the vault the person already syncs (with a code from
 * another device, or the recovery key), or turns sync on with this phone's vault.
 */
@Composable
fun SyncJoinSheet(start: SyncJoinMode = SyncJoinMode.Connect, close: () -> Unit) {
    val storage = remember { StorageModel() }
    var mode by remember { mutableStateOf(start) }
    var code by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var scanning by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    /** Turning on found the person's vault there already. */
    var found by remember { mutableStateOf(false) }
    /** Turned on: the recovery key to keep. */
    var recovery by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val ready = storage.ready && (mode == SyncJoinMode.Create || code.isNotBlank())

    // Fills the form from the other device's QR.
    fun apply(link: JoinLink?) {
        if (link == null) return
        mode = SyncJoinMode.Connect
        code = link.code
        link.repo?.let {
            storage.url = it
            storage.kind = StorageModel.Kind.Git
        }
        link.drive?.let { d ->
            storage.kind = StorageModel.Kind.Drive
            storage.drive.provider = d.provider
            storage.drive.folder = d
            storage.drive.account = d.account
        }
        AppModel.pendingJoin = null
    }

    LaunchedEffect(AppModel.pendingJoin) { apply(AppModel.pendingJoin) }
    LaunchedEffect(storage.kind) { error = null; found = false }

    fun submit() {
        if (!ready || busy) return
        busy = true
        error = null
        val c = code.trim()
        scope.launch {
            try {
                val drive = storage.kind == StorageModel.Kind.Drive
                when (mode) {
                    SyncJoinMode.Connect ->
                        if (drive) Core.call("sync_join_drive", mapOf("drive" to storage.driveJson, "code" to c))
                        else Core.call("sync_join_git", storage.git + ("code" to c))
                    SyncJoinMode.Create ->
                        if (drive) Core.call("sync_enable_drive", mapOf("drive" to storage.driveJson))
                        else Core.call("sync_enable_git", storage.git)
                }
                AppModel.reload()
                if (mode == SyncJoinMode.Create) recovery = true else close()
            } catch (f: CoreFailure) {
                if (f.kind == "syncExists") {
                    // The person's vault is there already: this phone joins it.
                    mode = SyncJoinMode.Connect
                    found = true
                } else {
                    error = coreErrorText(f, folder = storage.kind == StorageModel.Kind.Drive)
                }
            } catch (e: Exception) {
                error = coreErrorText(e, folder = storage.kind == StorageModel.Kind.Drive)
            }
            busy = false
        }
    }

    fun scan() {
        scanning = true
        error = null
        scope.launch {
            when (val r = JoinScanner.scan(context)) {
                is JoinScanner.Result.Found -> JoinLink.parse(r.text)?.let { apply(it) } ?: run { error = t("This isn't a Detour code") }
                is JoinScanner.Result.Failed -> error = r.message
                JoinScanner.Result.Cancelled -> {}
            }
            scanning = false
        }
    }

    Sheet(close) {
        if (recovery) {
            SheetBar(t("Recovery key"), right = { BarButton(t("Done"), bold = true, onClick = close) })
            LazyColumn(Modifier.fillMaxSize()) { recoveryKeySections() }
            return@Sheet
        }
        SheetBar(
            if (mode == SyncJoinMode.Create) t("Turn on sync") else t("Connect to your vault"),
            left = { BarButton(t("Cancel"), onClick = close) },
            right = {
                if (busy) CircularProgressIndicator(Modifier.size(22.dp), color = colors.accent)
                else BarButton(if (mode == SyncJoinMode.Create) t("Turn on") else t("Connect"), bold = true, enabled = ready) { submit() }
            },
        )
        LazyColumn(Modifier.fillMaxSize()) {
            storageSections(storage, "sync")
            if (mode == SyncJoinMode.Connect) {
                val footer = if (found) (if (storage.kind == StorageModel.Kind.Drive) t("This folder already has your vault") else t("This repository already has your vault")) else null
                section(t("Code from another device or recovery key"), footer = footer, key = "code") {
                    Cell(last = true) {
                        Box(Modifier.weight(1f)) {
                            BasicTextField(
                                code,
                                { v ->
                                    // A pasted join link fills the form, as its QR does.
                                    val link = if (v.trim().startsWith("detour://", ignoreCase = true)) JoinLink.parse(v) else null
                                    if (link != null) apply(link) else code = v
                                },
                                singleLine = true,
                                textStyle = TextStyle(color = colors.label, fontSize = 17.sp, fontFamily = FontFamily.Monospace),
                                cursorBrush = SolidColor(colors.accent),
                                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false, imeAction = ImeAction.Done),
                                keyboardActions = KeyboardActions(onAny = { submit() }),
                                decorationBox = { inner ->
                                    if (code.isEmpty()) Text("XXXX-XXXX-XXXX-XXXX", color = colors.tertiary, fontSize = 17.sp, fontFamily = FontFamily.Monospace)
                                    inner()
                                },
                            )
                        }
                        if (JoinScanner.supported(context)) {
                            Spacer(Modifier.width(8.dp))
                            if (scanning) CircularProgressIndicator(Modifier.size(22.dp), color = colors.accent, strokeWidth = 2.dp)
                            else BarIcon(Icons.Outlined.QrCodeScanner, t("Scan the code")) { scan() }
                        }
                    }
                }
            }
            errorItem(error)
        }
    }
}

/** The recovery key of the synced vault: the way back in without another device. */
fun LazyListScope.recoveryKeySections() {
    item(key = "recovery") { RecoveryKey() }
}

@Composable
private fun RecoveryKey() {
    var key by remember { mutableStateOf("") }
    var copies by remember { mutableStateOf(0) }
    val context = LocalContext.current
    LaunchedEffect(Unit) { key = runCatching { Core.call("sync_recovery_key") as? String }.getOrNull() ?: "" }
    Column {
        GroupCard {
            Cell(last = true) {
                SelectionContainer {
                    Text(key.ifEmpty { "…" }, color = colors.label, fontSize = 16.sp, fontFamily = FontFamily.Monospace)
                }
            }
        }
        GroupCard {
            Cell(last = key.isEmpty(), onClick = if (key.isEmpty()) null else ({ Clipboard.copy(key); copies++ })) {
                CopyIcon(copies > 0)
                Spacer(Modifier.width(14.dp))
                Text(if (copies > 0) t("Copied") else t("Copy"), color = if (key.isEmpty()) colors.tertiary else colors.accent, fontSize = 17.sp)
            }
            if (key.isNotEmpty()) ButtonCell(t("Save or send"), icon = Icons.Outlined.IosShare, last = true) { share(context, key) }
        }
    }
}

/** A section's rounded card, for rows that live in one list item. */
@Composable
fun GroupCard(rows: @Composable () -> Unit) {
    Column(Modifier.padding(horizontal = 16.dp).padding(top = 22.dp).fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(colors.cell)) { rows() }
}
