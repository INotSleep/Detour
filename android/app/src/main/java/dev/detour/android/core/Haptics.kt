package dev.detour.android.core

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType

/** Whether the app answers taps with a light vibration (Settings). On unless turned off. */
object Haptics {
    private lateinit var context: Context
    var enabled by mutableStateOf(true)
        private set

    fun load(context: Context) {
        this.context = context
        enabled = context.getSharedPreferences("app", Context.MODE_PRIVATE).getBoolean("haptics", true)
    }

    fun set(on: Boolean) {
        enabled = on
        context.getSharedPreferences("app", Context.MODE_PRIVATE).edit().putBoolean("haptics", on).apply()
    }

    /**
     * The app's haptics: the system's click effects, as the keyboard's keys use (the light "tick"
     * ones Compose picks for selections hardly show on many phones), as touch feedback so the
     * phone's own touch vibration setting applies; silent when turned off here.
     */
    class Gated(private val base: HapticFeedback) : HapticFeedback {
        private val vibrator: android.os.Vibrator? by lazy {
            if (android.os.Build.VERSION.SDK_INT >= 31) context.getSystemService(android.os.VibratorManager::class.java)?.defaultVibrator
            else @Suppress("DEPRECATION") context.getSystemService(android.os.Vibrator::class.java)
        }

        override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {
            if (!enabled) return
            val v = vibrator
            if (v == null || !v.hasVibrator()) {
                base.performHapticFeedback(hapticFeedbackType)
                return
            }
            val strong = hapticFeedbackType == HapticFeedbackType.LongPress || hapticFeedbackType == HapticFeedbackType.Confirm ||
                hapticFeedbackType == HapticFeedbackType.Reject || hapticFeedbackType == HapticFeedbackType.ToggleOn
            val effect = android.os.VibrationEffect.createPredefined(
                if (strong) android.os.VibrationEffect.EFFECT_HEAVY_CLICK else android.os.VibrationEffect.EFFECT_CLICK,
            )
            if (android.os.Build.VERSION.SDK_INT >= 33) {
                v.vibrate(effect, android.os.VibrationAttributes.createForUsage(android.os.VibrationAttributes.USAGE_TOUCH))
            } else {
                @Suppress("DEPRECATION")
                v.vibrate(effect, android.media.AudioAttributes.Builder().setUsage(android.media.AudioAttributes.USAGE_ASSISTANCE_SONIFICATION).build())
            }
        }
    }
}
