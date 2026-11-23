package dev.detour.android.core

import android.content.Context
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.suspendCancellableCoroutine
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.spec.MGF1ParameterSpec
import java.security.spec.X509EncodedKeySpec
import java.util.Locale
import javax.crypto.Cipher
import javax.crypto.spec.OAEPParameterSpec
import javax.crypto.spec.PSource
import kotlin.coroutines.resume

/**
 * The master password kept on this device behind the fingerprint, only if the person turns it
 * on. It is encrypted to a key of the Android Keystore whose private half opens only after a
 * strong biometric check; a new finger enrolled takes the key away. The public half needs no
 * check, so a password changed elsewhere is saved again without asking.
 */
object Biometry {
    private const val ALIAS = "detour-master"
    private const val PREF = "master"
    private const val TRANSFORM = "RSA/ECB/OAEPWithSHA-256AndMGF1Padding"
    // The Keystore's RSA keys do OAEP with SHA-1 for MGF1 on every Android version.
    private val oaep = OAEPParameterSpec("SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT)

    private lateinit var context: Context
    private val prefs: SharedPreferences get() = context.getSharedPreferences("biometry", Context.MODE_PRIVATE)

    fun init(context: Context) {
        this.context = context.applicationContext
    }

    private fun keyStore(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    val isSaved: Boolean
        get() = prefs.contains(PREF) && runCatching { keyStore().containsAlias(ALIAS) }.getOrDefault(false)

    /** Whether this device can unlock with a strong biometric now. */
    val available: Boolean
        get() = BiometricManager.from(context).canAuthenticate(BIOMETRIC_STRONG) == BiometricManager.BIOMETRIC_SUCCESS

    /** The name the texts use for it ("Unlock with %@"). */
    val name: String
        get() {
            val n = if (context.packageManager.hasSystemFeature(PackageManager.FEATURE_FINGERPRINT)) t("Fingerprint") else t("Biometrics")
            // Inside a sentence, except where nouns keep their capital.
            return if (Locale.getDefault().language == "de") n else n.replaceFirstChar { it.lowercase(Locale.getDefault()) }
        }

    fun save(password: String): Boolean {
        delete()
        return try {
            val gen = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_RSA, "AndroidKeyStore")
            gen.initialize(
                KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setKeySize(2048)
                    .setDigests(KeyProperties.DIGEST_SHA256, KeyProperties.DIGEST_SHA1)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP)
                    .setUserAuthenticationRequired(true)
                    .setInvalidatedByBiometricEnrollment(true)
                    .build()
            )
            val pair = gen.generateKeyPair()
            // The public half outside the Keystore, which would ask for the check otherwise.
            val pub = KeyFactory.getInstance(pair.public.algorithm).generatePublic(X509EncodedKeySpec(pair.public.encoded))
            val cipher = Cipher.getInstance(TRANSFORM).apply { init(Cipher.ENCRYPT_MODE, pub, oaep) }
            val data = cipher.doFinal(password.toByteArray(Charsets.UTF_8))
            prefs.edit().putString(PREF, Base64.encodeToString(data, Base64.NO_WRAP)).apply()
            true
        } catch (_: Exception) {
            delete()
            false
        }
    }

    /** The saved password, after the biometric check; null if cancelled or gone. */
    suspend fun load(activity: FragmentActivity, reason: String): String? {
        val data = prefs.getString(PREF, null)?.let { runCatching { Base64.decode(it, Base64.NO_WRAP) }.getOrNull() } ?: return null
        val cipher = try {
            val key = keyStore().getKey(ALIAS, null) as? PrivateKey ?: return null
            Cipher.getInstance(TRANSFORM).apply { init(Cipher.DECRYPT_MODE, key, oaep) }
        } catch (_: KeyPermanentlyInvalidatedException) {
            delete()
            return null
        } catch (_: Exception) {
            return null
        }
        val opened = suspendCancellableCoroutine<Cipher?> { cont ->
            val prompt = BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    if (cont.isActive) cont.resume(result.cryptoObject?.cipher)
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    if (cont.isActive) cont.resume(null)
                }
            })
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle(reason)
                .setNegativeButtonText(t("Cancel"))
                .setAllowedAuthenticators(BIOMETRIC_STRONG)
                .build()
            prompt.authenticate(info, BiometricPrompt.CryptoObject(cipher))
            cont.invokeOnCancellation { prompt.cancelAuthentication() }
        } ?: return null
        return runCatching { String(opened.doFinal(data), Charsets.UTF_8) }.getOrNull()
    }

    fun delete() {
        prefs.edit().remove(PREF).apply()
        runCatching { keyStore().deleteEntry(ALIAS) }
    }
}
