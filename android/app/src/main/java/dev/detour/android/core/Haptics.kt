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

    /** The system's haptics, silent when turned off: given to the whole app at its root. */
    class Gated(private val base: HapticFeedback) : HapticFeedback {
        override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {
            if (enabled) base.performHapticFeedback(hapticFeedbackType)
        }
    }
}
