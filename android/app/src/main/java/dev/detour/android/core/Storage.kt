package dev.detour.android.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.delay

/**
 * A drive's folder being chosen: which drive, the account there (one this device is signed in
 * to, another one added, or signed out of), and the folder, which carries the account.
 */
class DrivePick {
    var provider by mutableStateOf(DriveProvider.OneDrive)
    var folder by mutableStateOf<DriveFolderRef?>(null)
    /** The chosen account while it is signed in. */
    var account by mutableStateOf<String?>(null)
    var accounts by mutableStateOf<List<DriveAccount>>(emptyList())
    var chosen by mutableStateOf<String?>(null)
    var loaded by mutableStateOf(false)
    val flow = DriveSignInFlow()

    val current: DriveAccount? get() = accounts.firstOrNull { it.account == chosen }

    fun choose(p: DriveProvider) {
        if (p == provider) return
        provider = p
        if (folder?.provider != p) folder = null
    }

    fun publish() {
        val a = current?.takeIf { it.signedIn }?.account
        account = a
        if (a != null && folder != null && folder?.account != a) folder = folder?.copy(account = a)
    }

    suspend fun load(pick: String?) {
        accounts = Drives.accounts(provider)
        val want = pick ?: chosen
        chosen = accounts.firstOrNull { it.account == want }?.account ?: accounts.firstOrNull { it.signedIn }?.account ?: accounts.firstOrNull()?.account
        publish()
    }

    suspend fun added(who: String?) {
        if (who != null) load(who)
    }

    suspend fun signOut(who: String) {
        runCatching { Core.call("drive_sign_out", mapOf("provider" to provider.id, "account" to who)) }
        chosen = null
        load(null)
    }
}

/**
 * Signing in to a drive: the drive's page in a Custom Tab, back in the app once the drive has
 * answered (or a code from the page to paste, when the drive's port was busy).
 */
class DriveSignInFlow {
    var busy by mutableStateOf(false)
    var paste by mutableStateOf(false)
    var code by mutableStateOf("")
    var error by mutableStateOf<String?>(null)

    /** The account signed in to, or null (cancelled, refused, or a code to paste first). */
    suspend fun signIn(context: android.content.Context, p: DriveProvider): String? =
        dev.detour.android.KeepAliveService.whileSigningIn { signInNow(context, p) }

    private suspend fun signInNow(context: android.content.Context, p: DriveProvider): String? {
        busy = true
        error = null
        try {
            val start = Drives.start(p)
            when (Drives.browse(context, start.url)) {
                "0" -> {
                    error = t("The sign-in did not go through")
                    return null
                }
                null -> {
                    // Closed without the way back: the sign-in may have gone through all the same.
                    val got = runCatching { Drives.poll(p) }.getOrNull()
                    if (!got.isNullOrEmpty()) return done(got)
                    if (start.paste) paste = true
                    return null
                }
            }
            return done(Drives.finish(p))
        } catch (e: Exception) {
            error = Drives.errorText(e)
            return null
        } finally {
            busy = false
        }
    }

    suspend fun finishCode(p: DriveProvider): String? {
        val c = code.trim()
        if (c.isEmpty()) return null
        return try {
            done(Drives.finish(p, c))
        } catch (e: Exception) {
            error = Drives.errorText(e)
            null
        }
    }

    private fun done(who: String?): String? {
        if (who == null) error = t("The sign-in did not go through")
        paste = false
        code = ""
        return who
    }
}

/**
 * Where a vault or a team is kept, as the phone reaches it: a repository over HTTPS (signed in
 * with GitHub, or a token) or over SSH (with a key of the personal vault), or a cloud drive's
 * folder.
 */
class StorageModel {
    enum class Kind { Git, Drive }
    enum class Login { Github, Token }

    data class GithubCode(val code: String, val uri: String)

    var kind by mutableStateOf(Kind.Git)
    var url by mutableStateOf("")
    var login by mutableStateOf(Login.Github)
    var token by mutableStateOf("")
    /** The GitHub account this device is signed in with. */
    var github by mutableStateOf<String?>(null)
    /** A GitHub sign-in under way: the code to enter and where. */
    var signIn by mutableStateOf<GithubCode?>(null)
    var signInError by mutableStateOf<String?>(null)
    val drive = DrivePick()
    /** The personal key a repository over SSH is reached with. */
    var keyId by mutableStateOf<String?>(null)
    /** The personal vault's keys, to pick one from. */
    var keys by mutableStateOf<List<SshKey>>(emptyList())

    val trimmedURL: String get() = url.trim()

    /** A repository reached over SSH (`git@host:path`, `ssh://...`): anything not HTTPS. */
    val overSSH: Boolean
        get() {
            val u = trimmedURL.lowercase()
            return u.isNotEmpty() && !u.startsWith("https://") && !u.startsWith("http://")
        }

    /** Everything needed is filled in. */
    val ready: Boolean
        get() = when (kind) {
            Kind.Drive -> drive.folder != null && drive.account != null
            Kind.Git -> trimmedURL.isNotEmpty() && (if (overSSH) keyId != null else if (login == Login.Github) github != null else token.isNotEmpty())
        }

    /** The repository as the core takes it (`url`, `auth`, `token`, `github`). */
    val git: Map<String, Any?>
        get() = if (overSSH) mapOf("url" to trimmedURL, "auth" to mapOf("type" to "key", "keyId" to (keyId ?: "")))
        else mapOf("url" to trimmedURL, "auth" to mapOf("type" to "password"), "token" to (if (login == Login.Token) token else null), "github" to (login == Login.Github))

    /** The drive folder as the core takes it, reached with the chosen account. */
    val driveJson: Map<String, Any?>?
        get() = drive.folder?.copy(account = drive.account)?.json

    suspend fun loadGithub() {
        github = runCatching { Core.call("github_user") as? String }.getOrNull()
    }

    /** Signs this device in with GitHub: a code to enter on GitHub's page, then waits. */
    suspend fun startGithub(open: (String) -> Unit) = dev.detour.android.KeepAliveService.whileSigningIn { startGithubNow(open) }

    private suspend fun startGithubNow(open: (String) -> Unit) {
        signInError = null
        try {
            val d = Core.call("github_start").obj()
            val device = d.str("deviceCode") ?: return
            val user = d.str("userCode") ?: return
            val uri = d.str("verificationUri") ?: return
            signIn = GithubCode(user, uri)
            Clipboard.copy(user)
            open(uri)
            var interval = d.double("interval") ?: 5.0
            while (signIn != null) {
                delay((interval * 1000).toLong())
                // A check that couldn't get through is just tried again at the next one.
                val r = try {
                    Core.call("github_poll", mapOf("deviceCode" to device)).obj()
                } catch (f: CoreFailure) {
                    if (f.kind == "unreachable") continue else throw f
                }
                when (r.str("status")) {
                    "done" -> {
                        github = r.str("user")
                        signIn = null
                    }
                    "slowDown" -> interval = r.double("interval") ?: (interval + 5)
                }
            }
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (e: Exception) {
            signIn = null
            signInError = coreErrorText(e)
        }
    }
}

/** What a refusal of the core means, in words. */
fun coreErrorText(e: Throwable, folder: Boolean = false): String {
    val f = e as? CoreFailure ?: return e.message ?: e.toString()
    return when (f.kind) {
        "invalidCode" -> t("Wrong code")
        "codeExpired" -> t("The code has expired: make a new one on the other device")
        "notASyncVault" -> if (folder) t("This folder has no synced vault") else t("This repository has no synced vault")
        "folderNotEmpty" -> if (folder) t("This folder is not empty") else t("This repository is not empty")
        "notATeamVault" -> if (folder) t("This folder has a different team") else t("This repository has no team vault")
        "io", "unreachable" -> t("No connection")
        else -> Drives.errorText(f)
    }
}
