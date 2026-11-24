package dev.detour.android.core

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** A cloud drive a vault or a team can be kept in. */
enum class DriveProvider(val id: String, val title: String) {
    OneDrive("onedrive", "OneDrive"),
    GoogleDrive("gdrive", "Google Drive"),
    Dropbox("dropbox", "Dropbox");

    companion object {
        fun of(id: String?): DriveProvider? = entries.firstOrNull { it.id == id }
    }
}

/** A folder in a drive, as the core names it (`detour_core::drive::DriveRemote`). */
data class DriveFolderRef(
    val provider: DriveProvider,
    val id: String,
    val drive: String? = null,
    /** Its path or name, to show. */
    val label: String,
    val link: String? = null,
    /** The account it is reached with (any of the drive's when unset). */
    val account: String? = null,
) {
    /** As the core takes it. */
    val json: Map<String, Any?>
        get() = mapOf("provider" to provider.id, "id" to id, "label" to label, "drive" to drive, "link" to link, "account" to account)
            .filterValues { it != null }

    /** What the person sees: "Detour/Team", or the drive's name. */
    val title: String get() = label.ifEmpty { provider.title }

    companion object {
        fun from(m: Map<String, Any?>): DriveFolderRef? {
            val p = DriveProvider.of(m.str("provider")) ?: return null
            return DriveFolderRef(p, m.str("id") ?: return null, m.str("drive"), m.str("label") ?: "", m.str("link"), m.str("account"))
        }
    }
}

/** A folder's subfolders. */
data class DriveListing(val folder: DriveFolderRef?, val folders: List<Entry>) {
    data class Entry(val id: String, val drive: String?, val name: String, val shared: Boolean) {
        val key: String get() = "${drive ?: ""}/$id"
    }

    companion object {
        fun from(v: Any?) = DriveListing(
            DriveFolderRef.from(v.obj()["folder"].obj()),
            v.obj()["folders"].list().map { it.obj() }.map { Entry(it.str("id") ?: "", it.str("drive"), it.str("name") ?: "", it.bool("shared") ?: false) },
        )
    }
}

/** This device's sign-in to a drive. */
data class DriveAccount(val provider: DriveProvider, val account: String, val signedIn: Boolean)

/**
 * A refusal of the core as it reports it in a status (`driveSignIn`, `driveNoAccess`,
 * `hostKeyChanged`...), with the fields the screens act on.
 */
data class ErrorDetail(val kind: String, val provider: DriveProvider?, val account: String?, val message: String?, val actual: String?) {
    companion object {
        fun from(v: Any?): ErrorDetail? = v?.obj()?.takeIf { it.isNotEmpty() }?.let {
            ErrorDetail(it.str("kind") ?: "internal", DriveProvider.of(it.str("provider")), it.str("account"), it.str("message"), it.str("actual"))
        }
    }
}

/** A drive (and its account) something kept there needs signing in to again. */
data class DriveSignInNeed(val provider: DriveProvider, val account: String?) {
    val id: String get() = "${provider.id}/${account ?: ""}"
}

/** A team joined on another device of this person's and not here yet. */
data class PendingTeam(
    val teamId: String,
    val name: String,
    /** Its repository: joined here by itself. */
    val url: String?,
    /** Its drive folder: joined here by itself once signed in to the drive. */
    val drive: DriveFolderRef?,
    val error: ErrorDetail?,
) {
    companion object {
        fun from(m: Map<String, Any?>) = PendingTeam(
            m.str("teamId") ?: "", m.str("name") ?: "", m.str("url"),
            m["drive"]?.obj()?.let { DriveFolderRef.from(it) }, ErrorDetail.from(m["error"]),
        )
    }
}

/** What starting a sign-in gave: a page to open, and whether it ends with a code to paste. */
class DriveStart(val url: String, val paste: Boolean)

object Drives {
    /** This device's sign-ins to a drive's accounts. */
    suspend fun accounts(p: DriveProvider): List<DriveAccount> = all().filter { it.provider == p }

    /** The accounts this device is signed in to, of all drives. */
    suspend fun signedIn(): List<DriveAccount> = all().filter { it.signedIn }

    private suspend fun all(): List<DriveAccount> = runCatching { Core.call("drive_accounts").list() }.getOrDefault(emptyList()).mapNotNull {
        val m = it.obj()
        val p = DriveProvider.of(m.str("provider")) ?: return@mapNotNull null
        DriveAccount(p, m.str("account") ?: "", m.bool("signedIn") ?: false)
    }

    /** Starts signing in: the page sends the browser back to the app once the drive has answered. */
    suspend fun start(p: DriveProvider): DriveStart {
        val r = Core.call("drive_sign_in", mapOf("provider" to p.id, "back" to "detour://drive-signed-in")).obj()
        val url = r.str("url") ?: throw CoreFailure("internal", emptyMap())
        return DriveStart(url, r.bool("paste") ?: false)
    }

    /** Has the core finished the sign-in? The account once it has, "" while it hasn't, null if it ran out. */
    suspend fun poll(p: DriveProvider): String? {
        val r = Core.call("drive_sign_in_poll", mapOf("provider" to p.id)).obj()
        return when (r.str("status")) {
            "done" -> r.str("account") ?: ""
            "expired", "denied" -> null
            else -> ""
        }
    }

    /** Waits for the core to finish the sign-in it got back from the drive; the account. */
    suspend fun finish(p: DriveProvider): String? {
        repeat(120) {
            val r = poll(p) ?: return null
            if (r.isNotEmpty()) return r
            delay(500)
        }
        return null
    }

    /** Finishes a sign-in with the code the drive's page showed. */
    suspend fun finish(p: DriveProvider, code: String): String? =
        Core.call("drive_sign_in_code", mapOf("provider" to p.id, "code" to code)) as? String

    // The browser's way back: the drive's page ends on `detour://drive-signed-in?ok=1`, which
    // brings the app forward; coming back any other way (the tab closed) is a cancel.

    private val main = CoroutineScope(Dispatchers.Main.immediate)
    private var back: CompletableDeferred<String?>? = null

    /** The app was opened with `detour://drive-signed-in`. */
    fun returned(ok: String?) {
        back?.complete(ok ?: "1")
    }

    /** The app is in front again: without the link above, the page was closed. */
    fun resumed() {
        // A sign-in waiting for the browser listens again (its socket may have gone meanwhile).
        main.launch { runCatching { Core.call("drive_sign_in_resume") } }
        val b = back ?: return
        // The link, when there is one, comes just before this.
        main.launch { delay(600); b.complete(null) }
    }

    /**
     * Opens the sign-in page in a Custom Tab (the system browser without one); `"1"` or `"0"`
     * as the drive answered, or null when the person came back without an answer.
     */
    suspend fun browse(context: Context, url: String): String? {
        val d = CompletableDeferred<String?>()
        back = d
        try {
            open(context, url)
            return d.await()
        } finally {
            if (back === d) back = null
        }
    }

    /** Opens a web page: in a Custom Tab, or the browser. */
    fun open(context: Context, url: String) {
        val uri = Uri.parse(url)
        try {
            CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(context, uri)
        } catch (_: ActivityNotFoundException) {
            context.startActivity(Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }

    /** What went wrong, in words. */
    fun errorText(e: Throwable): String {
        val f = e as? CoreFailure ?: return e.message ?: e.toString()
        val name = DriveProvider.of(f["provider"])?.title ?: ""
        return when (f.kind) {
            "driveSignIn" -> f["account"]?.let { t("Sign in to %@ as %@ again", name, it) } ?: t("Sign in to %@ again", name)
            "driveNoAccess" -> t("The %@ account has no access to the folder", name)
            "folderNotEmpty" -> t("This folder is not empty")
            "notATeamVault" -> t("This folder has a different team")
            "notASyncVault" -> t("This folder has no synced vault")
            "unreachable" -> t("No connection")
            else -> f["message"] ?: f.kind
        }
    }
}
