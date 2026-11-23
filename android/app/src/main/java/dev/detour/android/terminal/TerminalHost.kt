package dev.detour.android.terminal

import android.content.Context
import android.graphics.Typeface
import android.util.TypedValue
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.inputmethod.InputMethodManager
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView
import com.termux.view.TerminalView
import dev.detour.android.ui.colors
import com.termux.view.TerminalViewClient
import com.termux.terminal.TerminalSession as Emulated

/** The terminal view for a session: draws its emulator, takes the keyboard and touches. */
@Composable
fun TerminalHost(session: TerminalSession, modifier: Modifier = Modifier) {
    val theme = TerminalSettings.theme(colors.dark)
    val size = TerminalSettings.fontSize
    session.theme = theme
    AndroidView(
        factory = { context ->
            TerminalView(context, null).apply {
                setTerminalViewClient(Client(this, session))
                setTextSize(TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, size, context.resources.displayMetrics).toInt())
                tag = size
                isFocusable = true
                isFocusableInTouchMode = true
                // No grey veil over the terminal when focus came from a hardware key.
                defaultFocusHighlightEnabled = false
                keepScreenOn = true
                attachSession(session.emulated)
                session.view = this
                post { requestFocus(); if (!session.keysKeyboard) showKeyboard(this) }
            }
        },
        update = { view ->
            if (view.tag != size) {
                view.tag = size
                view.setTextSize(TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, size, view.resources.displayMetrics).toInt())
            }
            if (view.currentSession !== session.emulated) {
                view.attachSession(session.emulated)
                session.view = view
            }
        },
        modifier = modifier,
    )
    DisposableEffect(session) {
        onDispose { session.view?.let { if (it.currentSession === session.emulated) session.view = null } }
    }
}

fun showKeyboard(view: TerminalView) {
    view.requestFocus()
    (view.context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager).showSoftInput(view, 0)
}

fun hideKeyboard(view: TerminalView) {
    (view.context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager).hideSoftInputFromWindow(view.windowToken, 0)
}

private class Client(private val view: TerminalView, private val session: TerminalSession) : TerminalViewClient {
    private var scale = 1f

    override fun onScale(scale: Float): Float {
        // Pinch changes the text size.
        if (scale < 0.9f || scale > 1.1f) {
            val now = (view.resources.displayMetrics.scaledDensity * 13 * (this.scale * scale)).coerceIn(16f, 64f)
            this.scale = now / (view.resources.displayMetrics.scaledDensity * 13)
            view.setTextSize(now.toInt())
            return 1f
        }
        return scale
    }

    // A tap brings back the keyboard chosen: the keys one, or the system one.
    override fun onSingleTapUp(e: MotionEvent?) {
        if (session.keysKeyboard) session.keysShown = true else showKeyboard(view)
    }
    override fun shouldBackButtonBeMappedToEscape() = false
    override fun shouldEnforceCharBasedInput() = true
    override fun shouldUseCtrlSpaceWorkaround() = false
    override fun isTerminalViewSelected() = true
    override fun copyModeChanged(copyMode: Boolean) {}
    override fun onKeyDown(keyCode: Int, e: KeyEvent?, session: Emulated?) = false
    override fun onKeyUp(keyCode: Int, e: KeyEvent?) = false
    override fun onLongPress(event: MotionEvent?) = false
    // Ctrl and Alt of the key bar apply in the session, to what is typed next.
    override fun readControlKey() = false
    override fun readAltKey() = false
    override fun readShiftKey() = false
    override fun readFnKey() = false
    override fun onCodePoint(codePoint: Int, ctrlDown: Boolean, session: Emulated?) = false
    override fun onEmulatorSet() {}
    override fun logError(tag: String?, message: String?) {}
    override fun logWarn(tag: String?, message: String?) {}
    override fun logInfo(tag: String?, message: String?) {}
    override fun logDebug(tag: String?, message: String?) {}
    override fun logVerbose(tag: String?, message: String?) {}
    override fun logStackTraceWithMessage(tag: String?, message: String?, e: Exception?) {}
    override fun logStackTrace(tag: String?, e: Exception?) {}
}
