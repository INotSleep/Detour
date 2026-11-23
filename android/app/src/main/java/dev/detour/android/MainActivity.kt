package dev.detour.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.SystemBarStyle
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.fragment.app.FragmentActivity
import dev.detour.android.core.Biometry
import dev.detour.android.core.Drives
import dev.detour.android.terminal.AppAppearance
import dev.detour.android.terminal.TerminalsScreen
import dev.detour.android.ui.BiometryOffer
import dev.detour.android.ui.DetourTheme
import dev.detour.android.ui.LockScreen
import dev.detour.android.ui.MainScreen
import dev.detour.android.ui.colors

// A FragmentActivity: the fingerprint's prompt (BiometricPrompt) needs one.
class MainActivity : FragmentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val notifications = registerForActivityResult(androidx.activity.result.contract.ActivityResultContracts.RequestPermission()) {}
        KeepAliveService.askNotifications = {
            if (android.os.Build.VERSION.SDK_INT >= 33 &&
                checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED
            ) notifications.launch(android.Manifest.permission.POST_NOTIFICATIONS)
        }
        enableEdgeToEdge(SystemBarStyle.auto(0, 0), SystemBarStyle.auto(0, 0))
        Biometry.init(applicationContext)
        AppAppearance.load(applicationContext)
        AppModel.start()
        if (savedInstanceState == null) opened(intent)
        setContent {
            // The app's theme sets dark or light and the accent; without one, the phone's.
            val dark = AppAppearance.dark ?: isSystemInDarkTheme()
            LaunchedEffect(dark) {
                enableEdgeToEdge(
                    if (dark) SystemBarStyle.dark(0) else SystemBarStyle.light(0, 0),
                    if (dark) SystemBarStyle.dark(0) else SystemBarStyle.light(0, 0),
                )
            }
            DetourTheme(dark = dark, accent = AppAppearance.accent) {
                // Every haptic of the app goes through the setting.
                androidx.compose.runtime.CompositionLocalProvider(
                    androidx.compose.ui.platform.LocalHapticFeedback provides dev.detour.android.core.Haptics.Gated(androidx.compose.ui.platform.LocalHapticFeedback.current),
                ) {
                    Root()
                    dev.detour.android.ui.HudView()
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        opened(intent)
    }

    // Leaving the app: whether the keyboard is up now, before Android takes it down.
    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        dev.detour.android.terminal.KeyboardMemory.up = androidx.core.view.ViewCompat.getRootWindowInsets(window.decorView)
            ?.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime()) == true
    }

    private var restoreKeyboard = false

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || !restoreKeyboard) return
        restoreKeyboard = false
        val terminal = (AppModel.sessions.firstOrNull { it.id == AppModel.current } as? OpenSession.Terminal)?.session
        if (AppModel.showTerminals && terminal != null && !terminal.keysKeyboard) {
            // After the window applies its own keyboard state for the focus it just got.
            terminal.view?.let { v -> v.postDelayed({ dev.detour.android.terminal.showKeyboard(v) }, 120) }
        }
    }

    override fun onResume() {
        super.onResume()
        AppModel.resumed()
        Drives.resumed()
        // Android brings the keyboard back for the terminal on its own whatever it was, over the
        // screen: it is hidden on return (the manifest), and shown again (once the window has the
        // focus a keyboard needs) only if it was up.
        restoreKeyboard = dev.detour.android.terminal.KeyboardMemory.up
    }

    /** A `detour://` link: a drive's sign-in coming back, or a join link (another device's QR). */
    private fun opened(intent: Intent?) {
        val uri = intent?.data ?: return
        if (uri.scheme != "detour") return
        when (uri.host) {
            "drive-signed-in" -> Drives.returned(uri.getQueryParameter("ok"))
            "join" -> AppModel.openLink(uri.toString())
        }
    }
}

@Composable
private fun Root() {
    Box(Modifier.fillMaxSize().background(colors.background)) {
        // Locked and unlocked fade into each other.
        androidx.compose.animation.Crossfade(AppModel.phase, label = "phase") { phase ->
            Box(Modifier.fillMaxSize()) {
                when (phase) {
                    AppModel.Phase.Loading -> CircularProgressIndicator(Modifier.align(Alignment.Center), color = colors.accent)
                    AppModel.Phase.Setup -> LockScreen(setup = true)
                    AppModel.Phase.Locked -> LockScreen(setup = false)
                    AppModel.Phase.Ready -> MainScreen()
                }
            }
        }
        if (AppModel.phase == AppModel.Phase.Ready) {
            // The sessions come up from the bottom over the tabs, as iOS shows a full-screen cover.
            androidx.compose.animation.AnimatedVisibility(
                AppModel.showTerminals,
                enter = androidx.compose.animation.slideInVertically(androidx.compose.animation.core.spring(dampingRatio = 1f, stiffness = 500f)) { it },
                exit = androidx.compose.animation.slideOutVertically(androidx.compose.animation.core.spring(dampingRatio = 1f, stiffness = 500f)) { it },
            ) {
                BackHandler { dev.detour.android.terminal.leaveTerminals((AppModel.sessions.firstOrNull { it.id == AppModel.current } as? OpenSession.Terminal)?.session) }
                TerminalsScreen()
            }
            BiometryOffer()
        }
    }
}
