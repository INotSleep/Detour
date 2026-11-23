package dev.detour.android.ui

import android.net.Uri
import android.text.format.DateFormat
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.AddCircleOutline
import androidx.compose.material.icons.outlined.Cancel
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import dev.detour.android.AppModel
import dev.detour.android.core.Core
import dev.detour.android.core.DriveSignInNeed
import dev.detour.android.core.ErrorDetail
import dev.detour.android.core.Json
import dev.detour.android.core.StorageModel
import dev.detour.android.core.SyncStatus
import dev.detour.android.core.coreErrorText
import dev.detour.android.core.double
import dev.detour.android.core.obj
import dev.detour.android.core.str
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.launch
import java.util.Date

/**
 * Syncing the personal vault between this person's devices: where, how it goes, another device,
 * the recovery key, turning it off here.
 */
@Composable
fun SyncPage(back: String, onBack: () -> Unit) {
    var setup by remember { mutableStateOf<SyncJoinMode?>(null) }
    var addingDevice by remember { mutableStateOf(false) }
    var showingKey by remember { mutableStateOf(false) }
    var newLogin by remember { mutableStateOf(false) }
    var confirmOff by remember { mutableStateOf(false) }
    var replaceKey by remember { mutableStateOf(false) }
    var syncing by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    suspend fun syncNow() {
        syncing = true
        error = null
        AppModel.syncNow()
        syncing = false
    }

    fun turnOff() = scope.launch {
        try {
            Core.call("sync_disable")
            AppModel.reload()
        } catch (e: Exception) {
            error = coreErrorText(e)
        }
    }

    fun replaceServerKey() = scope.launch {
        val actual = AppModel.syncError?.actual ?: return@launch
        try {
            Core.call("sync_replace_server_key", mapOf("fingerprint" to actual))
            AppModel.reload()
        } catch (e: Exception) {
            error = coreErrorText(e)
        }
    }

    PushedPage(t("Sync"), back, onBack, refresh = { syncNow() }) {
        val s = AppModel.sync
        if (s != null && s.enabled) {
            section(key = "status") {
                val e = AppModel.syncError
                Cell(inset = 52) {
                    CellIcon(if (s.git) Icons.Outlined.AccountTree else if (s.storage == "folder") Icons.Outlined.Folder else Icons.Outlined.Cloud)
                    Column(Modifier.weight(1f)) {
                        Text(s.location ?: "", color = colors.label, fontSize = 15.sp, fontFamily = FontFamily.Monospace, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        Text(
                            syncStatusText(s.pending, s.syncedAt, e, syncing), fontSize = 13.sp,
                            color = if (e != null) colors.red else if (s.pending) colors.orange else colors.secondary,
                        )
                    }
                }
                // The way to put right what stopped the last sync, or syncing now.
                val m = e?.message ?: ""
                when {
                    e?.kind == "driveSignIn" && e.provider != null ->
                        DriveSignInAgainRows(DriveSignInNeed(e.provider, e.account), last = true) { scope.launch { AppModel.driveSignedIn() } }
                    e?.kind == "hostKeyChanged" -> ButtonCell(t("Replace server key"), last = true) { replaceKey = true }
                    e?.kind == "git" && ("token rejected" in m || "github sign-in needed" in m || "github app has no access" in m) ->
                        ButtonCell(t("Sign in to the repository again"), last = true) { newLogin = true }
                    else -> ButtonCell(t("Sync now"), busy = syncing, last = true) { scope.launch { syncNow() } }
                }
            }
            section(key = "device") {
                ButtonCell(t("Add another device"), icon = Icons.Outlined.Devices) { addingDevice = true }
                ButtonCell(t("Recovery key"), icon = Icons.Outlined.Key, last = true) { showingKey = true }
            }
            section(key = "off") {
                ButtonCell(t("Turn off on this device"), icon = Icons.Outlined.Cancel, destructive = true, last = true) { confirmOff = true }
            }
        } else {
            section(key = "setup") {
                ButtonCell(t("Connect to your vault"), icon = Icons.Outlined.Sync) { setup = SyncJoinMode.Connect }
                ButtonCell(t("Turn on sync"), icon = Icons.Outlined.AddCircleOutline, last = true) { setup = SyncJoinMode.Create }
            }
        }
        errorItem(error)
    }

    setup?.let { m -> SyncJoinSheet(m) { setup = null } }
    if (addingDevice) AddDeviceSheet { addingDevice = false }
    if (showingKey) {
        Sheet({ showingKey = false }) {
            SheetBar(t("Recovery key"), right = { BarButton(t("Done"), bold = true) { showingKey = false } })
            LazyColumn(Modifier.fillMaxSize()) { recoveryKeySections() }
        }
    }
    if (newLogin) RepoLoginSheet(AppModel.sync?.location ?: "") { newLogin = false }
    if (confirmOff) {
        Confirm(t("Turn off sync on this device?"), null, t("Turn off"), { confirmOff = false }) { turnOff() }
    }
    if (replaceKey) {
        Confirm(t("Replace the server key?"), null, t("Replace key"), { replaceKey = false }) { replaceServerKey() }
    }
}

/** iOS's confirmation dialog: the question, a destructive button, Cancel. */
@Composable
fun Confirm(title: String, message: String?, action: String, dismiss: () -> Unit, confirm: () -> Unit) {
    AlertDialog(
        onDismissRequest = dismiss,
        title = { Text(title, fontSize = 17.sp, fontWeight = FontWeight.SemiBold) },
        text = message?.let { { Text(it, color = colors.secondary) } },
        confirmButton = { TextButton({ dismiss(); confirm() }) { Text(action, color = colors.red) } },
        dismissButton = { TextButton(dismiss) { Text(t("Cancel")) } },
        containerColor = colors.cell,
        titleContentColor = colors.label,
    )
}

/** How a sync stands, in words: its error, under way, waiting to go, or when it last went. */
fun syncStatusText(pending: Boolean, syncedAt: Double?, error: ErrorDetail?, syncing: Boolean): String = when {
    error != null -> syncErrorText(error)
    syncing -> t("Syncing…")
    pending -> t("Not sent yet")
    syncedAt == null -> t("Not synced yet")
    else -> t("Synced %@", ago(syncedAt))
}

/** A sync error in words. */
fun syncErrorText(e: ErrorDetail): String = when (e.kind) {
    "driveSignIn" -> {
        val name = e.provider?.title ?: ""
        e.account?.let { t("Sign in to %@ as %@ again", name, it) } ?: t("Sign in to %@ again", name)
    }
    "driveNoAccess" -> t("The %@ account has no access to the folder", e.provider?.title ?: "")
    "hostKeyChanged" -> t("The server's key has changed")
    "unreachable", "io" -> t("No connection")
    else -> e.message ?: e.kind
}

/** A one-time code (and its QR) another device of this person's joins sync with. */
@Composable
fun AddDeviceSheet(close: () -> Unit) {
    var code by remember { mutableStateOf<String?>(null) }
    var expiresAt by remember { mutableStateOf(0.0) }
    var error by remember { mutableStateOf<String?>(null) }
    var copies by remember { mutableStateOf(0) }
    val context = LocalContext.current

    LaunchedEffect(Unit) {
        try {
            val p = Core.call("sync_add_device").obj()
            expiresAt = p.double("expiresAt") ?: 0.0
            code = p.str("code")
        } catch (e: Exception) {
            error = coreErrorText(e)
        }
    }

    Sheet(close) {
        SheetBar(t("New device"), right = { BarButton(t("Done"), bold = true, onClick = close) })
        Column(
            Modifier.fillMaxSize().padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterVertically),
        ) {
            val c = code
            val e = error
            when {
                c != null -> {
                    QrImage(joinLink(c, AppModel.sync))
                    SelectionContainer {
                        Text(c, color = colors.label, fontSize = 20.sp, fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold)
                    }
                    Text(t("Valid until %@", DateFormat.getTimeFormat(context).format(Date(expiresAt.toLong()))), color = colors.secondary, fontSize = 13.sp)
                    Row(
                        Modifier.clip(RoundedCornerShape(8.dp)).background(colors.accent.copy(alpha = 0.15f))
                            .clickable { Clipboard.copy(c); copies++ }.padding(horizontal = 16.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        CopyIcon(copies > 0)
                        Spacer(Modifier.width(8.dp))
                        Text(if (copies > 0) t("Copied") else t("Copy"), color = colors.accent, fontSize = 17.sp)
                    }
                }
                e != null -> Text(e, color = colors.red, textAlign = TextAlign.Center)
                else -> CircularProgressIndicator(color = colors.accent)
            }
            Spacer(Modifier.height(60.dp))
        }
    }
}

/** What the other device scans: the code, and where the vault is as it reaches it. */
private fun joinLink(code: String, sync: SyncStatus?): String {
    val b = Uri.Builder().scheme("detour").authority("join").appendQueryParameter("code", code)
    val drive = AppModel.syncDrive
    val repo = sync?.location
    if (drive != null) b.appendQueryParameter("drive", Json.stringify(drive.json))
    else if (repo != null && (repo.startsWith("https://") || repo.startsWith("http://"))) b.appendQueryParameter("repo", repo)
    return b.build().toString()
}

/** A QR code drawn on the phone, black on white. */
@Composable
fun QrImage(text: String) {
    val matrix = remember(text) {
        runCatching {
            QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 0, 0, mapOf(EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.L, EncodeHintType.MARGIN to 0))
        }.getOrNull()
    } ?: return
    Box(Modifier.widthIn(max = 260.dp).fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Color.White).padding(12.dp)) {
        Canvas(Modifier.fillMaxWidth().aspectRatio(1f)) {
            val n = matrix.width
            val cell = size.width / n
            for (y in 0 until n) for (x in 0 until n) {
                if (matrix[x, y]) drawRect(Color.Black, Offset(x * cell, y * cell), Size(cell + 0.5f, cell + 0.5f))
            }
        }
    }
}

/** A new way into the sync repository: GitHub again, or a new token. */
@Composable
fun RepoLoginSheet(url: String, close: () -> Unit) {
    val model = remember { StorageModel() }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val ready = if (model.login == StorageModel.Login.Github) model.github != null else model.token.isNotEmpty()

    fun save() {
        busy = true
        scope.launch {
            try {
                if (model.login == StorageModel.Login.Github) Core.call("sync_use_github")
                else Core.call("sync_set_token", mapOf("token" to model.token))
                AppModel.reload()
                close()
            } catch (e: Exception) {
                error = coreErrorText(e)
            }
            busy = false
        }
    }

    Sheet(close) {
        SheetBar(
            t("Repository login"),
            left = { BarButton(t("Cancel"), onClick = close) },
            right = { if (busy) CircularProgressIndicator(Modifier.size(22.dp), color = colors.accent) else BarButton(t("Save"), bold = true, enabled = ready) { save() } },
        )
        LazyColumn(Modifier.fillMaxSize()) {
            section(key = "url") {
                Cell(last = true) { Text(url, color = colors.label, fontSize = 15.sp, fontFamily = FontFamily.Monospace) }
            }
            section(key = "login") { RepoLoginRows(model) }
            errorItem(error)
        }
    }
}
