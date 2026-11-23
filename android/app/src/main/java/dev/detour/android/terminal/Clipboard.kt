package dev.detour.android.terminal

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context

/** The system clipboard, for the terminal's copy and paste. */
object Clipboard {
    lateinit var context: Context

    private val manager get() = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager

    /** Copies, and says so (and taps lightly): a copy is otherwise invisible. */
    fun copy(text: String) {
        manager.setPrimaryClip(ClipData.newPlainText("Detour", text))
        dev.detour.android.ui.Hud.show(dev.detour.android.core.t("Copied"))
    }

    fun paste(): String? = manager.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()
}
