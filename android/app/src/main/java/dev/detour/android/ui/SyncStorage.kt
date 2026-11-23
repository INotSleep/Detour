package dev.detour.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.IosShare
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.core.Core
import dev.detour.android.core.Drives
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.SshKey
import dev.detour.android.core.StorageModel
import dev.detour.android.core.coreErrorText
import dev.detour.android.core.list
import dev.detour.android.core.obj
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.launch

/** Choosing the storage: a repository or a cloud drive, and how to get in. */
fun LazyListScope.storageSections(model: StorageModel, key: String) {
    section(key = "$key-kind") {
        Box(Modifier.padding(12.dp)) {
            Segmented(listOf(StorageModel.Kind.Git to t("Repository"), StorageModel.Kind.Drive to t("Cloud drive")), model.kind) { model.kind = it }
        }
    }
    if (model.kind == StorageModel.Kind.Drive) {
        driveSections(model.drive, key)
    } else {
        section(t("Repository"), key = "$key-url") {
            FieldCell(model.url, { model.url = it }, "https://github.com/you/detour-vault", plain = true, last = true, ime = androidx.compose.ui.text.input.ImeAction.Done)
        }
        if (model.overSSH) repoKeySection(model, key) else repoLoginSection(model, key)
    }
}

private suspend fun personalKeys(): List<SshKey> =
    runCatching { PersonalVault.get()["keys"].list().map { SshKey.from(it.obj()) } }.getOrDefault(emptyList())

/**
 * Getting into a repository over SSH: a key of the personal vault (or a new one), whose public
 * half the repository has to know.
 */
fun LazyListScope.repoKeySection(model: StorageModel, key: String) {
    val chosen = model.keys.firstOrNull { it.id == model.keyId }
    section(key = "$key-key", footer = if (chosen != null) t("Add it to the repository as a deploy key with write access") else null) {
        RepoKeyRows(model)
    }
}

@Composable
private fun RepoKeyRows(model: StorageModel) {
    var making by remember { mutableStateOf(false) }
    var copies by remember { mutableStateOf(0) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val chosen = model.keys.firstOrNull { it.id == model.keyId }

    suspend fun load(pick: String? = null) {
        model.keys = personalKeys()
        val want = pick ?: model.keyId
        model.keyId = model.keys.firstOrNull { it.id == want }?.id ?: model.keys.firstOrNull()?.id
    }

    // A key for this repository, named after it.
    fun make() {
        making = true
        error = null
        scope.launch {
            val repo = model.trimmedURL.split('/', ':').lastOrNull()?.replace(".git", "")
            val name = (if (repo.isNullOrEmpty()) "git" else repo) + "-android"
            try {
                Core.call("key_generate", mapOf("vaultId" to "personal", "name" to name, "algorithm" to "ed25519", "remember" to true))
                load(personalKeys().lastOrNull { it.name == name }?.id)
            } catch (e: Exception) {
                error = coreErrorText(e)
            }
            making = false
        }
    }

    LaunchedEffect(Unit) { load() }

    if (model.keys.isNotEmpty()) {
        MenuCell(t("Key"), model.keys.map { it.id to it.name }, model.keyId) { model.keyId = it }
    }
    // The button is always there: the keys load with it.
    ButtonCell(t("New key"), icon = Icons.Outlined.Add, busy = making, last = chosen == null && error == null) { make() }
    if (chosen != null) {
        Cell(onClick = { Clipboard.copy(chosen.publicKey); copies++ }) {
            Text(t("Public key"), color = colors.label, fontSize = 17.sp, modifier = Modifier.weight(1f))
            CopyIcon(copies > 0)
        }
        ButtonCell(t("Share"), icon = Icons.Outlined.IosShare, last = error == null) { share(context, chosen.publicKey + "\n") }
    }
    error?.let { ErrorCell(it) }
}

/** Getting into a repository: signed in with GitHub, or an access token. */
fun LazyListScope.repoLoginSection(model: StorageModel, key: String) {
    section(key = "$key-login") { RepoLoginRows(model) }
}

@Composable
fun RepoLoginRows(model: StorageModel) {
    var copies by remember { mutableStateOf(0) }
    val context = LocalContext.current

    LaunchedEffect(Unit) { model.loadGithub() }

    Box(Modifier.padding(12.dp)) {
        Segmented(listOf(StorageModel.Login.Github to "GitHub", StorageModel.Login.Token to t("Token")), model.login) { model.login = it }
    }
    if (model.login == StorageModel.Login.Github) {
        val github = model.github
        val signIn = model.signIn
        val error = model.signInError
        when {
            github != null -> Cell(last = error == null) {
                Icon(Icons.Filled.CheckCircle, null, tint = colors.green, modifier = Modifier.size(22.dp))
                Spacer(Modifier.width(12.dp))
                Text(github, color = colors.green, fontSize = 17.sp)
            }
            signIn != null -> {
                Cell(onClick = { Clipboard.copy(signIn.code); copies++ }) {
                    Text(signIn.code, color = colors.label, fontSize = 22.sp, fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold, modifier = Modifier.weight(1f))
                    CopyIcon(copies > 0)
                }
                ButtonCell(t("Open GitHub"), last = error == null) { Drives.open(context, signIn.uri) }
            }
            // Not this row's scope: the list drops the row when it scrolls away, and the wait with it.
            else -> ButtonCell(t("Sign in with GitHub"), last = error == null) {
                dev.detour.android.AppModel.scope.launch { model.startGithub { Drives.open(context, it) } }
            }
        }
        error?.let { ErrorCell(it) }
    } else {
        FieldCell(model.token, { model.token = it }, t("Access token"), secure = true, last = true, ime = androidx.compose.ui.text.input.ImeAction.Done)
    }
}
