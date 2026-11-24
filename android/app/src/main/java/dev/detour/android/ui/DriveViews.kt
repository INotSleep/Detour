package dev.detour.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.CreateNewFolder
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Groups
import androidx.compose.material.icons.outlined.People
import androidx.compose.material.icons.outlined.UnfoldMore
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.core.Core
import dev.detour.android.core.DriveFolderRef
import dev.detour.android.core.DriveListing
import dev.detour.android.core.DrivePick
import dev.detour.android.core.DriveProvider
import dev.detour.android.core.DriveSignInFlow
import dev.detour.android.core.DriveSignInNeed
import dev.detour.android.core.Drives
import dev.detour.android.core.PendingTeam
import dev.detour.android.core.obj
import dev.detour.android.core.t
import kotlinx.coroutines.launch

/** The field for a code the drive's page showed. */
@Composable
private fun DriveCodeField(flow: DriveSignInFlow, last: Boolean, finish: () -> Unit) {
    Cell(last = last) {
        Box(Modifier.weight(1f)) {
            Field(flow.code, { flow.code = it }, t("Code from the page"), mono = true, plain = true, onDone = finish)
        }
        Spacer(Modifier.width(10.dp))
        BarButton(t("Done"), enabled = flow.code.isNotBlank(), onClick = finish)
    }
}

/**
 * Which drive, the account there, and the folder: picked from its folders, or opened from a
 * share link. The folder carries the account it is reached with.
 */
fun LazyListScope.driveSections(pick: DrivePick, key: String) {
    section(key = "$key-drive") {
        MenuCell(t("Service"), DriveProvider.entries.map { it to it.title }, pick.provider) { pick.choose(it) }
        DriveAccountRows(pick)
    }
    val account = pick.account
    if (account != null) {
        section(key = "$key-folder") { DriveFolderRows(pick, account) }
    }
}

/**
 * The account of a drive a vault is reached with: one this device is signed in to, another one
 * added, or signed out of. `pick.account` is the chosen one while it is signed in.
 */
@Composable
private fun DriveAccountRows(pick: DrivePick) {
    val flow = pick.flow
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val provider = pick.provider
    fun signIn() = scope.launch { pick.added(flow.signIn(context, provider)) }

    LaunchedEffect(provider) {
        flow.paste = false
        flow.error = null
        pick.load(pick.account)
        pick.loaded = true
    }
    val current = pick.current
    val again = !flow.paste && current != null && !current.signedIn
    val error = flow.error
    val lastRow = !again && error == null
    when {
        flow.paste -> DriveCodeField(flow, lastRow) { scope.launch { pick.added(flow.finishCode(provider)) } }
        current != null -> {
            var menu by remember { mutableStateOf(false) }
            Box {
                Cell(last = lastRow, onClick = { menu = true }) {
                    Text(t("Account"), color = colors.label, fontSize = 17.sp, modifier = Modifier.weight(1f))
                    if (flow.busy) {
                        CircularProgressIndicator(Modifier.size(16.dp), color = colors.secondary, strokeWidth = 2.dp)
                        Spacer(Modifier.width(6.dp))
                    }
                    Text(current.account, color = colors.secondary, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.MiddleEllipsis, modifier = Modifier.weight(1f, fill = false))
                    Icon(Icons.Outlined.UnfoldMore, null, tint = colors.secondary, modifier = Modifier.padding(start = 4.dp).size(16.dp))
                }
                DropdownMenu(menu, { menu = false }, Modifier.align(Alignment.TopEnd)) {
                    pick.accounts.forEach { a ->
                        DropdownMenuItem(
                            { Text(a.account) }, { menu = false; pick.chosen = a.account; pick.publish() },
                            trailingIcon = if (a.account == current.account) ({ Icon(Icons.Outlined.Check, null, tint = colors.accent) }) else null,
                        )
                    }
                    DropdownMenuItem(text = { Text(t("Add account")) }, onClick = { menu = false; signIn() }, leadingIcon = { Icon(Icons.Outlined.Add, null) })
                    HorizontalDivider(color = colors.separator)
                    DropdownMenuItem(
                        { Text(t("Sign out"), color = colors.red) },
                        { menu = false; scope.launch { pick.signOut(current.account) } },
                        leadingIcon = { Icon(Icons.AutoMirrored.Outlined.Logout, null, tint = colors.red) },
                    )
                }
            }
        }
        pick.loaded -> ButtonCell(t("Sign in to %@", provider.title), busy = flow.busy, last = lastRow) { signIn() }
        else -> Cell(last = lastRow) {
            Text(t("Account"), color = colors.label, fontSize = 17.sp, modifier = Modifier.weight(1f))
            CircularProgressIndicator(Modifier.size(16.dp), color = colors.secondary, strokeWidth = 2.dp)
        }
    }
    if (again) ButtonCell(t("Sign in to %@ again", provider.title), enabled = !flow.busy, last = error == null) { signIn() }
    if (error != null) ErrorCell(error)
}

/** The folder: chosen in the drive, or opened from a link to a shared folder. */
@Composable
private fun DriveFolderRows(pick: DrivePick, account: String) {
    var browsing by remember { mutableStateOf(false) }
    var link by remember { mutableStateOf("") }
    var opening by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val provider = pick.provider

    fun openLink() {
        val url = link.trim()
        if (url.isEmpty() || opening) return
        opening = true
        error = null
        scope.launch {
            try {
                pick.folder = DriveFolderRef.from(Core.call("drive_link", mapOf("url" to url, "provider" to provider.id, "account" to account)).obj())
                link = ""
            } catch (e: Exception) {
                error = Drives.errorText(e)
            }
            opening = false
        }
    }

    Cell(onClick = { browsing = true }) {
        Text(t("Folder"), color = colors.label, fontSize = 17.sp)
        Spacer(Modifier.width(12.dp))
        val f = pick.folder
        Text(
            f?.title ?: t("Choose"), color = if (f == null) colors.accent else colors.secondary, fontSize = 17.sp,
            maxLines = 1, overflow = TextOverflow.StartEllipsis, modifier = Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.End,
        )
    }
    Cell(last = error == null) {
        Box(Modifier.weight(1f)) {
            androidx.compose.foundation.text.BasicTextField(
                link, { link = it }, singleLine = true,
                textStyle = androidx.compose.ui.text.TextStyle(color = colors.label, fontSize = 17.sp),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(colors.accent),
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                    keyboardType = androidx.compose.ui.text.input.KeyboardType.Uri, autoCorrectEnabled = false, imeAction = ImeAction.Go,
                ),
                keyboardActions = androidx.compose.foundation.text.KeyboardActions(onAny = { openLink() }),
                modifier = Modifier.fillMaxWidth(),
                decorationBox = { inner ->
                    if (link.isEmpty()) Text(t("Or a link to a shared folder"), color = colors.tertiary, fontSize = 17.sp, maxLines = 1)
                    inner()
                },
            )
        }
        if (opening) CircularProgressIndicator(Modifier.size(18.dp), color = colors.secondary, strokeWidth = 2.dp)
        else if (link.isNotBlank()) {
            Spacer(Modifier.width(10.dp))
            BarButton(t("Open")) { openLink() }
        }
    }
    error?.let { ErrorCell(it) }
    if (browsing) DriveBrowser(provider, account, { browsing = false }) { f -> pick.folder = f; browsing = false }
}

/** A drive's folders, one level at a time: walk into one, make one, or use this one. */
@Composable
fun DriveBrowser(provider: DriveProvider, account: String, close: () -> Unit, pick: (DriveFolderRef) -> Unit) {
    /** The folders walked into, the last one shown. */
    val path = remember { mutableStateListOf<DriveFolderRef>() }
    var listing by remember { mutableStateOf<DriveListing?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var naming by remember { mutableStateOf(false) }
    var newName by remember { mutableStateOf("") }
    var attempt by remember { mutableStateOf(0) }
    val scope = rememberCoroutineScope()
    val parent = path.lastOrNull()

    LaunchedEffect(parent, attempt) {
        error = null
        listing = null
        try {
            listing = DriveListing.from(Core.call("drive_browse", mapOf("provider" to provider.id, "folder" to parent?.json, "account" to account)))
        } catch (e: Exception) {
            error = Drives.errorText(e)
        }
    }

    fun child(f: DriveListing.Entry): DriveFolderRef {
        val label = if (f.shared || parent == null) f.name else "${parent.label}/${f.name}"
        return DriveFolderRef(provider, f.id, f.drive ?: listing?.folder?.drive, label, null, account)
    }

    fun create() {
        val name = newName.trim()
        newName = ""
        naming = false
        if (name.isEmpty()) return
        scope.launch {
            try {
                DriveFolderRef.from(Core.call("drive_folder_create", mapOf("provider" to provider.id, "parent" to parent?.json, "name" to name, "account" to account)).obj())
                    ?.let { path.add(it) }
            } catch (e: Exception) {
                error = Drives.errorText(e)
            }
        }
    }

    Sheet(close) {
        BackHandler(path.isNotEmpty()) { path.removeAt(path.lastIndex) }
        SheetBar(
            parent?.let { p -> p.label.split("/").lastOrNull()?.ifEmpty { null } ?: p.label } ?: provider.title,
            left = {
                if (path.isEmpty()) BarButton(t("Cancel"), onClick = close)
                else BackButton(if (path.size == 1) provider.title else path[path.size - 2].label.split("/").last()) { path.removeAt(path.lastIndex) }
            },
            right = { BarIcon(Icons.Outlined.CreateNewFolder, t("New folder")) { naming = true } },
        )
        Box(Modifier.weight(1f).fillMaxWidth()) {
            val folders = listing?.folders ?: emptyList()
            LazyColumn(Modifier.fillMaxSize()) {
                if (folders.isNotEmpty()) {
                    section(key = parent?.id ?: "root") {
                        folders.forEachIndexed { i, f ->
                            Cell(last = i == folders.lastIndex, onClick = { path.add(child(f)) }, inset = 52) {
                                CellIcon(if (f.shared) Icons.Outlined.People else Icons.Outlined.Folder)
                                Text(f.name, color = colors.label, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                                if (f.shared) Text(t("Shared"), color = colors.secondary, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 6.dp))
                                Icon(Icons.Outlined.ChevronRight, null, tint = colors.tertiary, modifier = Modifier.size(20.dp))
                            }
                        }
                    }
                }
                item { Spacer(Modifier.height(24.dp)) }
            }
            val e = error
            when {
                e != null -> Column(Modifier.align(Alignment.Center).padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(Icons.Outlined.Warning, null, tint = colors.secondary, modifier = Modifier.size(40.dp))
                    Spacer(Modifier.height(10.dp))
                    Text(e, color = colors.label, fontSize = 17.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                    Spacer(Modifier.height(10.dp))
                    BarButton(t("Try again")) { attempt++ }
                }
                listing == null -> CircularProgressIndicator(Modifier.align(Alignment.Center), color = colors.accent)
                folders.isEmpty() -> Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
                    Icon(Icons.Outlined.Folder, null, tint = colors.secondary, modifier = Modifier.size(40.dp))
                    Spacer(Modifier.height(10.dp))
                    Text(t("No folders here"), color = colors.label, fontSize = 20.sp)
                }
            }
        }
        if (parent != null) {
            ProminentButton(t("Use this folder"), Modifier.fillMaxWidth().padding(16.dp).navigationBarsPadding()) { pick(parent) }
        }
    }
    if (naming) {
        AlertDialog(
            onDismissRequest = { naming = false; newName = "" },
            title = { Text(t("New folder")) },
            text = { Field(newName, { newName = it }, t("Name"), onDone = { create() }) },
            confirmButton = { TextButton({ create() }) { Text(t("Create")) } },
            dismissButton = { TextButton({ naming = false; newName = "" }) { Text(t("Cancel")) } },
            // The field is drawn on the cell colour.
            containerColor = colors.background,
            titleContentColor = colors.label,
        )
    }
}

/** Signing in to a drive again, where something kept there asks for it. */
@Composable
fun DriveSignInAgainRows(need: DriveSignInNeed, last: Boolean, done: () -> Unit) {
    val flow = remember(need.id) { DriveSignInFlow() }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val error = flow.error
    if (flow.paste) {
        DriveCodeField(flow, last && error == null) { scope.launch { if (flow.finishCode(need.provider) != null) done() } }
    } else {
        ButtonCell(t("Sign in to %@ again", need.provider.title), detail = need.account, busy = flow.busy, last = last && error == null) {
            scope.launch { if (flow.signIn(context, need.provider) != null) done() }
        }
    }
    if (error != null) ErrorCell(error, last = last)
}

/**
 * A team on another device kept in a folder its drive's own app syncs: a row that opens it here.
 * While it is being opened, it shows how far.
 */
@Composable
fun FolderTeamRow(team: PendingTeam, last: Boolean, onClick: () -> Unit) {
    val opening = AppModel.openingTeams[team.teamId]
    val going = opening != null && opening.error == null
    Cell(last = last, onClick = if (going) null else onClick, inset = 52) {
        CellIcon(Icons.Outlined.Groups)
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(team.name, color = colors.label, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                if (opening != null && going) {
                    if (opening.total > 0) Text("${opening.done} / ${opening.total}", color = colors.secondary, fontSize = 15.sp, maxLines = 1, modifier = Modifier.padding(start = 8.dp))
                } else {
                    Text(t("Open from a cloud drive"), color = colors.secondary, fontSize = 15.sp, maxLines = 1, modifier = Modifier.padding(start = 8.dp))
                }
            }
            if (opening != null && going) {
                val progress by animateFloatAsState(if (opening.total > 0) opening.done.toFloat() / opening.total else 0f, label = "opening")
                Spacer(Modifier.height(8.dp))
                if (opening.total > 0) {
                    LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth().height(4.dp), color = colors.accent, trackColor = colors.separator, gapSize = 0.dp, drawStopIndicator = {})
                } else {
                    LinearProgressIndicator(Modifier.fillMaxWidth().height(4.dp), color = colors.accent, trackColor = colors.separator, gapSize = 0.dp)
                }
            }
            opening?.error?.let { Text(it, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(top = 2.dp)) }
        }
    }
}

/**
 * A team joined on another device that keeps it in a folder its drive's own app syncs: opened
 * here through the drive.
 */
@Composable
fun TeamFromDriveSheet(team: PendingTeam, close: () -> Unit) {
    val pick = remember { DrivePick() }
    val ready = pick.folder != null && pick.account != null

    // Opening goes on by itself: the sheet goes, and the teams show how far it is.
    fun open() {
        val folder = pick.folder?.copy(account = pick.account) ?: return
        AppModel.openTeamFromDrive(team, folder)
        close()
    }

    Sheet(close) {
        SheetBar(
            team.name,
            left = { BarButton(t("Cancel"), onClick = close) },
            right = { BarButton(t("Open"), bold = true, enabled = ready) { open() } },
        )
        LazyColumn(Modifier.fillMaxSize()) {
            driveSections(pick, "team")
        }
    }
}

/** The drive sign-ins needed again and the teams to open from a drive, at the top of a list. */
fun LazyListScope.driveNeedsSection(openTeam: (PendingTeam) -> Unit) {
    val needs = AppModel.driveSignIns
    val teams = AppModel.folderTeams
    if (needs.isEmpty() && teams.isEmpty()) return
    section(key = "drive-needs") {
        needs.forEachIndexed { i, n ->
            DriveSignInAgainRows(n, last = i == needs.lastIndex && teams.isEmpty()) { AppModel.scope.launch { AppModel.driveSignedIn() } }
        }
        teams.forEachIndexed { i, team -> FolderTeamRow(team, i == teams.lastIndex) { openTeam(team) } }
    }
}
