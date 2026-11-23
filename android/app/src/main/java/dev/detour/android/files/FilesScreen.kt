package dev.detour.android.files

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.text.format.Formatter
import android.webkit.MimeTypeMap
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBackIos
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.ArrowCircleDown
import androidx.compose.material.icons.outlined.ArrowCircleUp
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.SubdirectoryArrowRight
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import dev.detour.android.ui.DropdownMenu
import dev.detour.android.ui.DropdownMenuItem
import dev.detour.android.ui.menuIcon
import dev.detour.android.ui.SwipeActions
import dev.detour.android.ui.SwipeAction
import dev.detour.android.ui.deleteAction
import androidx.compose.material.icons.outlined.DriveFileRenameOutline
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.SnapshotStateMap
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import dev.detour.android.ui.BarButton
import dev.detour.android.ui.BarIcon
import dev.detour.android.ui.Cell
import dev.detour.android.ui.EmptyState
import dev.detour.android.ui.Field
import dev.detour.android.ui.ProminentButton
import dev.detour.android.ui.colors
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.text.DateFormat
import java.util.Date

/**
 * A host's files over SFTP, a session like a terminal: folders as a stack with Back, a file saved
 * to the phone, uploads from the phone, new folders, rename and delete. The session opens and
 * closes with its tab; this is only what it shows.
 */
@Composable
fun FilesView(files: FilesSession, modifier: Modifier = Modifier) {
    // What each folder of the stack listed and where it was scrolled, for going back to it.
    val listed = remember(files) { mutableStateMapOf<String, List<RemoteFile>>() }
    val scrolls = remember(files) { HashMap<String, LazyListState>() }

    Column(modifier.fillMaxSize().background(colors.background)) {
        if (files.state == FilesSession.State.Ready && files.folders.isNotEmpty()) {
            val path = files.folders.last()
            fun back() {
                if (files.folders.size < 2) return
                val gone = files.folders.removeAt(files.folders.lastIndex)
                listed.remove(gone)
                scrolls.remove(gone)
            }
            BackHandler(enabled = files.folders.size > 1) { back() }
            key(path) {
                FolderView(
                    files, path, listed,
                    scrolls.getOrPut(path) { LazyListState() },
                    back = files.folders.getOrNull(files.folders.size - 2)?.let { folderTitle(it) },
                    onBack = { back() },
                    push = { files.folders.add(it) },
                )
            }
        } else {
            TitleBar(files.host.label)
            Gate(files, Modifier.weight(1f))
        }
    }
}

/** Connecting, or what the connection waits for. */
@Composable
private fun Gate(files: FilesSession, modifier: Modifier) {
    var password by remember { mutableStateOf("") }
    fun open(secret: String? = null, trust: String? = null) {
        AppModel.scope.launch { files.open(secret, trust) }
    }
    fun submit() {
        if (password.isEmpty()) return
        val pw = password
        password = ""
        open(secret = pw)
    }
    Box(modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Column(
            Modifier.widthIn(max = 360.dp).padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            when (val s = files.state) {
                FilesSession.State.Connecting, FilesSession.State.Ready -> {
                    CircularProgressIndicator(color = colors.accent, modifier = Modifier.size(28.dp))
                    Text("${files.host.login}@${files.host.address}", color = colors.secondary, fontSize = 15.sp, fontFamily = FontFamily.Monospace, textAlign = TextAlign.Center)
                }
                is FilesSession.State.NeedPassword -> {
                    val label = if (s.passphrase) t("Passphrase") else t("Password")
                    Text(s.prompt.ifEmpty { label }, color = colors.label, fontSize = 15.sp, textAlign = TextAlign.Center)
                    Field(password, { password = it }, label, secure = true, onDone = { submit() })
                    ProminentButton(t("Connect"), Modifier.fillMaxWidth(), enabled = password.isNotEmpty()) { submit() }
                }
                is FilesSession.State.UnknownKey -> {
                    Text(t("New server key"), color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
                    Text(s.fingerprint, color = colors.label, fontSize = 12.sp, fontFamily = FontFamily.Monospace, textAlign = TextAlign.Center)
                    ProminentButton(t("Trust and connect")) { open(trust = s.fingerprint) }
                }
                is FilesSession.State.Failed -> {
                    Text(s.message, color = colors.label, fontSize = 15.sp, textAlign = TextAlign.Center)
                    ProminentButton(t("Retry")) { open() }
                }
            }
        }
    }
}

/** One folder of the server. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun FolderView(
    files: FilesSession,
    path: String,
    listed: SnapshotStateMap<String, List<RemoteFile>>,
    scroll: LazyListState,
    back: String?,
    onBack: () -> Unit,
    push: (String) -> Unit,
) {
    val entries = listed[path]
    var error by remember { mutableStateOf<String?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    var opening by remember { mutableStateOf<String?>(null) }
    var menu by remember { mutableStateOf(false) }
    var newFolder by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf<RemoteFile?>(null) }
    var deleting by remember { mutableStateOf<RemoteFile?>(null) }
    var exists by remember { mutableStateOf<List<Uri>>(emptyList()) }
    var flash by remember { mutableStateOf<String?>(null) }
    /** The file being saved while the phone asks where to. */
    var saving by rememberSaveable { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current

    suspend fun load() {
        try {
            listed[path] = files.list(path)
            error = null
        } catch (f: CoreFailure) {
            error = f["message"] ?: f.kind
        } catch (e: Exception) {
            error = e.message ?: e.toString()
        }
    }

    fun showError(message: String) { flash = message }

    fun message(e: Exception) = (e as? CoreFailure)?.let { it["message"] ?: it.kind } ?: e.message ?: e.toString()

    /** Runs a change, then shows the folder as it is now. */
    fun act(change: suspend () -> Unit) {
        AppModel.scope.launch {
            try { change() } catch (e: Exception) { showError(message(e)) }
            load()
        }
    }

    fun upload(uris: List<Uri>, replace: Boolean) {
        AppModel.scope.launch {
            val taken = ArrayList<Uri>()
            for (uri in uris) {
                try {
                    files.upload(uri, path, replace)
                } catch (f: CoreFailure) {
                    if ((f["message"] ?: "").endsWith("already exists")) taken.add(uri) else showError(f["message"] ?: f.kind)
                } catch (e: Exception) {
                    showError(message(e))
                }
            }
            load()
            exists = taken
        }
    }

    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (uris.isNotEmpty()) upload(uris, replace = false)
    }
    val saver = rememberLauncherForActivityResult(SaveAs()) { uri ->
        val remote = saving ?: return@rememberLauncherForActivityResult
        saving = null
        if (uri == null) return@rememberLauncherForActivityResult
        AppModel.scope.launch {
            try {
                files.save(remote, uri)
            } catch (e: Exception) {
                // An empty document is left where it was to go: it goes too.
                runCatching { DocumentsContract.deleteDocument(context.contentResolver, uri) }
                showError(message(e))
            }
        }
    }

    /** A link may lead to a folder: it opens like one if it lists. */
    fun open(f: RemoteFile) {
        val full = FilesSession.join(path, f.name)
        if (f.isDir) { push(full); return }
        if (!f.isLink) { saving = full; saver.launch(f.name); return }
        scope.launch {
            opening = f.name
            val folder = runCatching { files.list(full) }.isSuccess
            opening = null
            if (folder) push(full) else { saving = full; saver.launch(f.name) }
        }
    }

    LaunchedEffect(path) { if (listed[path] == null) load() }
    LaunchedEffect(flash) {
        if (flash != null) { delay(4000); flash = null }
    }

    TitleBar(
        if (path == "/") "/" else folderTitle(path),
        back = back,
        onBack = onBack,
    ) {
        Box {
            BarIcon(Icons.Outlined.Add, t("Add")) { menu = true }
            DropdownMenu(menu, { menu = false }) {
                DropdownMenuItem(leadingIcon = menuIcon("Upload"), text = { Text(t("Upload")) }, onClick = { menu = false; picker.launch(arrayOf("*/*")) })
                DropdownMenuItem(leadingIcon = menuIcon("New folder"), text = { Text(t("New folder")) }, onClick = { menu = false; newFolder = true })
                DropdownMenuItem(leadingIcon = menuIcon("Copy path"), text = { Text(t("Copy path")) }, onClick = { menu = false; Clipboard.copy(path) })
            }
        }
    }
    Box(Modifier.fillMaxSize()) {
        PullToRefreshBox(
            refreshing,
            onRefresh = { scope.launch { refreshing = true; load(); refreshing = false } },
            Modifier.fillMaxSize(),
        ) {
            LazyColumn(Modifier.fillMaxSize(), state = scroll) {
                if (entries != null && entries.isNotEmpty()) {
                    item { Spacer(Modifier.height(16.dp)) }
                    itemsIndexed(entries, key = { _, f -> f.name }) { i, f ->
                        FileRow(
                            f, first = i == 0, last = i == entries.lastIndex, busy = opening == f.name,
                            open = { open(f) },
                            rename = { renaming = f },
                            copyPath = { Clipboard.copy(FilesSession.join(path, f.name)) },
                            delete = { deleting = f },
                        )
                    }
                    item { Spacer(Modifier.height(100.dp)) }
                } else if (error != null) {
                    item {
                        Column(Modifier.fillMaxWidth().padding(top = 120.dp, start = 32.dp, end = 32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                            Icon(Icons.Outlined.WarningAmber, null, tint = colors.secondary, modifier = Modifier.size(44.dp))
                            Spacer(Modifier.height(12.dp))
                            Text(error ?: "", color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
                            Spacer(Modifier.height(8.dp))
                            BarButton(t("Retry")) { scope.launch { load() } }
                        }
                    }
                } else if (entries != null) {
                    item { EmptyState(Icons.Outlined.Folder, t("Empty folder")) }
                }
            }
            if (entries == null && error == null) {
                CircularProgressIndicator(color = colors.accent, modifier = Modifier.align(Alignment.Center).size(28.dp))
            }
        }
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            AnimatedVisibility(flash != null, enter = fadeIn(), exit = fadeOut()) {
                Text(
                    flash ?: "", color = colors.red, fontSize = 15.sp, textAlign = TextAlign.Center,
                    modifier = Modifier.padding(horizontal = 16.dp).fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(colors.cell).padding(12.dp),
                )
            }
            TransfersBar(files)
        }
    }

    if (newFolder) {
        NameDialog(t("New folder"), "", t("Create"), { newFolder = false }) { name ->
            act { files.mkdir(FilesSession.join(path, name)) }
        }
    }
    renaming?.let { f ->
        NameDialog(t("Rename"), f.name, t("Rename"), { renaming = null }) { name ->
            if (name != f.name) act { files.rename(FilesSession.join(path, f.name), FilesSession.join(path, name)) }
        }
    }
    deleting?.let { f ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            containerColor = colors.cell,
            title = { Text(t("Delete %@?", f.name), color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold) },
            confirmButton = {
                TextButton({ deleting = null; act { files.remove(FilesSession.join(path, f.name)) } }) { Text(t("Delete"), color = colors.red) }
            },
            dismissButton = { TextButton({ deleting = null }) { Text(t("Cancel"), color = colors.accent) } },
        )
    }
    if (exists.isNotEmpty()) {
        val title = if (exists.size == 1) t("%@ already exists", files.nameOf(exists[0])) else t("Some files already exist")
        AlertDialog(
            onDismissRequest = { exists = emptyList() },
            containerColor = colors.cell,
            title = { Text(title, color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold) },
            confirmButton = {
                TextButton({ val uris = exists; exists = emptyList(); upload(uris, replace = true) }) { Text(t("Replace"), color = colors.accent) }
            },
            dismissButton = { TextButton({ exists = emptyList() }) { Text(t("Cancel"), color = colors.accent) } },
        )
    }
}

/** A folder's name as its title. */
private fun folderTitle(path: String) = if (path == "/") "/" else path.trimEnd('/').substringAfterLast('/').ifEmpty { "/" }

/** The bar over a folder: Back to the one before it, the title, the folder's actions. */
@Composable
private fun TitleBar(title: String, back: String? = null, onBack: () -> Unit = {}, actions: @Composable () -> Unit = {}) {
    Box(Modifier.fillMaxWidth().height(44.dp).padding(horizontal = 8.dp)) {
        if (back != null) {
            val interaction = remember { MutableInteractionSource() }
            val pressed by interaction.collectIsPressedAsState()
            val tint = colors.accent.copy(alpha = if (pressed) 0.4f else 1f)
            Row(
                Modifier.align(Alignment.CenterStart).clickable(interaction, null, onClick = onBack).padding(vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.AutoMirrored.Outlined.ArrowBackIos, t("Back"), tint = tint, modifier = Modifier.size(20.dp))
                Text(back, color = tint, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 96.dp))
            }
        }
        Text(
            title, color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.MiddleEllipsis,
            textAlign = TextAlign.Center, modifier = Modifier.align(Alignment.Center).widthIn(max = 180.dp),
        )
        Box(Modifier.align(Alignment.CenterEnd)) { actions() }
    }
}

/** A file or folder as a row of the folder's card, with its menu on a long press. */
@Composable
private fun FileRow(
    file: RemoteFile,
    first: Boolean,
    last: Boolean,
    busy: Boolean,
    open: () -> Unit,
    rename: () -> Unit,
    copyPath: () -> Unit,
    delete: () -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    val context = LocalContext.current
    val shape = RoundedCornerShape(
        topStart = if (first) 10.dp else 0.dp, topEnd = if (first) 10.dp else 0.dp,
        bottomStart = if (last) 10.dp else 0.dp, bottomEnd = if (last) 10.dp else 0.dp,
    )
    Box(Modifier.padding(horizontal = 16.dp).clip(shape).background(colors.cell)) {
        SwipeActions(listOf(deleteAction(delete), SwipeAction(dev.detour.android.core.t("Rename"), androidx.compose.material.icons.Icons.Outlined.DriveFileRenameOutline, colors.accent, rename))) {
            Cell(last = last, onClick = open, onLongClick = { menu = true }, inset = 52) {
                Icon(
                    when {
                        file.isDir -> Icons.Filled.Folder
                        file.isLink -> Icons.Outlined.SubdirectoryArrowRight
                        else -> Icons.Outlined.Description
                    },
                    null,
                    tint = if (file.isDir) colors.accent else colors.secondary,
                    modifier = Modifier.size(24.dp),
                )
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(file.name, color = colors.label, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
                    val detail = detail(file, context)
                    if (detail.isNotEmpty()) Text(detail, color = colors.secondary, fontSize = 12.sp, maxLines = 1)
                }
                if (busy) CircularProgressIndicator(color = colors.secondary, strokeWidth = 2.dp, modifier = Modifier.size(18.dp))
            }
        }
        DropdownMenu(menu, { menu = false }) {
            if (!file.isDir) DropdownMenuItem(leadingIcon = menuIcon("Save"), text = { Text(t("Save")) }, onClick = { menu = false; open() })
            DropdownMenuItem(leadingIcon = menuIcon("Rename"), text = { Text(t("Rename")) }, onClick = { menu = false; rename() })
            DropdownMenuItem(leadingIcon = menuIcon("Copy path"), text = { Text(t("Copy path")) }, onClick = { menu = false; copyPath() })
            DropdownMenuItem(leadingIcon = menuIcon("Delete"), text = { Text(t("Delete"), color = colors.red) }, onClick = { menu = false; delete() })
        }
    }
}

private fun detail(file: RemoteFile, context: Context): String {
    val parts = ArrayList<String>()
    if (!file.isDir) parts.add(Formatter.formatFileSize(context, file.size.toLong()))
    file.modified?.let { parts.add(DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(it.toLong()))) }
    return parts.joinToString(" · ")
}

/** A name to type: a new folder's, or a new one for a file. Enter does it. */
@Composable
private fun NameDialog(title: String, initial: String, action: String, onDismiss: () -> Unit, onDone: (String) -> Unit) {
    var name by remember { mutableStateOf(initial) }
    val focus = remember { FocusRequester() }
    fun done() {
        val n = name.trim()
        if (n.isEmpty()) return
        onDismiss()
        onDone(n)
    }
    LaunchedEffect(Unit) { focus.requestFocus() }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = colors.cell,
        title = { Text(title, color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold) },
        text = {
            Box(Modifier.clip(RoundedCornerShape(10.dp)).background(colors.background)) {
                Field(name, { name = it }, t("Name"), Modifier.focusRequester(focus), plain = true, onDone = { done() })
            }
        },
        confirmButton = {
            TextButton({ done() }, enabled = name.isNotBlank()) {
                Text(action, color = if (name.isNotBlank()) colors.accent else colors.tertiary, fontWeight = FontWeight.SemiBold)
            }
        },
        dismissButton = { TextButton(onDismiss) { Text(t("Cancel"), color = colors.accent) } },
    )
}

/** What is being downloaded, uploaded or deleted, with Cancel. */
@Composable
private fun TransfersBar(files: FilesSession) {
    val tr = files.transfers.firstOrNull() ?: return
    Row(
        Modifier.padding(horizontal = 16.dp).padding(bottom = 4.dp).fillMaxWidth()
            .clip(RoundedCornerShape(14.dp)).background(colors.cell).padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            when (tr.kind) {
                FilesSession.Transfer.Kind.Down -> Icons.Outlined.ArrowCircleDown
                FilesSession.Transfer.Kind.Up -> Icons.Outlined.ArrowCircleUp
                FilesSession.Transfer.Kind.Delete -> Icons.Outlined.Delete
            },
            null, tint = colors.accent, modifier = Modifier.size(24.dp),
        )
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(
                if (files.transfers.size > 1) "${tr.name} +${files.transfers.size - 1}" else tr.name,
                color = colors.label, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.MiddleEllipsis,
            )
            val f = tr.fraction
            LinearProgressIndicator(
                progress = { f?.toFloat() ?: 0f },
                modifier = Modifier.fillMaxWidth().alpha(if (f == null) 0.4f else 1f),
                color = colors.accent, trackColor = colors.fill, strokeCap = StrokeCap.Round, gapSize = 0.dp, drawStopIndicator = {},
            )
        }
        Spacer(Modifier.width(12.dp))
        BarButton(t("Cancel")) { files.cancel(tr) }
    }
}

/** The system's "save as", with the file's own type so the right apps and folders are offered. */
private class SaveAs : ActivityResultContracts.CreateDocument("application/octet-stream") {
    override fun createIntent(context: Context, input: String): Intent {
        val ext = input.substringAfterLast('.', "").lowercase()
        val type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "application/octet-stream"
        return super.createIntent(context, input).setType(type)
    }
}
