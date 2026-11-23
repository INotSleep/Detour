package dev.detour.android.ui

import android.content.Context
import android.content.pm.PackageManager
import com.google.android.gms.common.moduleinstall.ModuleInstall
import com.google.android.gms.common.moduleinstall.ModuleInstallRequest
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import dev.detour.android.core.DriveFolderRef
import dev.detour.android.core.Json
import dev.detour.android.core.obj
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume

/**
 * A code for joining sync, as another device shows it: `detour://join?code=…&repo=…` or
 * `…&drive=…` (its QR), or the code alone.
 */
data class JoinLink(val code: String, val repo: String?, val drive: DriveFolderRef?) {
    companion object {
        private val bare = Regex("^[A-Za-z0-9]{4}(-[A-Za-z0-9]{4}){3,}$")

        fun parse(text: String): JoinLink? {
            val t = text.trim()
            if (t.startsWith("detour://join", ignoreCase = true)) {
                val rest = t.substring("detour://join".length)
                if (rest.isNotEmpty() && rest[0] != '?' && rest[0] != '/') return null
                val query = query(rest.substringAfter('?', ""))
                val code = query["code"]?.takeIf { it.isNotEmpty() } ?: return null
                val drive = query["drive"]?.let { runCatching { DriveFolderRef.from(Json.parse(it).obj()) }.getOrNull() }
                return JoinLink(code, query["repo"]?.ifEmpty { null }, drive)
            }
            return if (bare.matches(t)) JoinLink(t, null, null) else null
        }

        // Values as iOS writes them: percent-encoded, a "+" left as it is.
        private fun query(q: String): Map<String, String> = q.split('&').mapNotNull { kv ->
            val i = kv.indexOf('=')
            if (i <= 0) null else decode(kv.substring(0, i)) to decode(kv.substring(i + 1))
        }.toMap()

        private fun decode(s: String): String {
            val out = java.io.ByteArrayOutputStream()
            var i = 0
            while (i < s.length) {
                val c = s[i]
                if (c == '%' && i + 2 < s.length) {
                    val b = s.substring(i + 1, i + 3).toIntOrNull(16)
                    if (b != null) {
                        out.write(b)
                        i += 3
                        continue
                    }
                }
                out.write(c.toString().toByteArray(Charsets.UTF_8))
                i++
            }
            return out.toString("UTF-8")
        }
    }
}

/** Google's code scanner: it has the camera itself, so the app asks for no permission. */
object JoinScanner {
    fun supported(context: Context): Boolean = context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)

    sealed interface Result {
        data class Found(val text: String) : Result
        data object Cancelled : Result
        data class Failed(val message: String) : Result
    }

    /** Scans one QR (the scanner is fetched by Play services the first time). */
    suspend fun scan(context: Context): Result {
        val options = GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build()
        val scanner = GmsBarcodeScanning.getClient(context, options)
        val installed = suspendCancellableCoroutine<String?> { cont ->
            ModuleInstall.getClient(context).installModules(ModuleInstallRequest.newBuilder().addApi(scanner).build())
                .addOnSuccessListener { if (cont.isActive) cont.resume(null) }
                .addOnFailureListener { if (cont.isActive) cont.resume(it.localizedMessage ?: it.toString()) }
        }
        if (installed != null) return Result.Failed(installed)
        return suspendCancellableCoroutine { cont ->
            scanner.startScan()
                .addOnSuccessListener { if (cont.isActive) cont.resume(it.rawValue?.let { v -> Result.Found(v) } ?: Result.Cancelled) }
                .addOnCanceledListener { if (cont.isActive) cont.resume(Result.Cancelled) }
                .addOnFailureListener { if (cont.isActive) cont.resume(Result.Failed(it.localizedMessage ?: it.toString())) }
        }
    }
}
