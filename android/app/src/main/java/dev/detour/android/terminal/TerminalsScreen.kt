package dev.detour.android.terminal

import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBackIos
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Apps
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Keyboard
import androidx.compose.material.icons.outlined.KeyboardHide
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.layout.imeAnimationTarget
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.graphics.luminance
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.OpenSession
import dev.detour.android.core.t
import dev.detour.android.keys.BarKey
import dev.detour.android.keys.KeyBarStore
import dev.detour.android.keys.KeyboardEditor
import dev.detour.android.keys.KeysKeyboardView
import dev.detour.android.ui.Field
import dev.detour.android.ui.HostPicker
import dev.detour.android.ui.NewKind
import dev.detour.android.ui.NewSessionMenu
import dev.detour.android.files.FilesSession
import dev.detour.android.files.FilesView
import dev.detour.android.ui.ProminentButton
import dev.detour.android.ui.colors

/**
 * The open sessions: the current one, and under it, by the keyboard, the sessions' bar (back to
 * the hosts, the sessions' tabs, a new session) and the key bar.
 */
/** Whether the system keyboard was up over the terminal when the app was left. */
object KeyboardMemory {
    var up = false
}

/** Whether the app is dark, for the status bar's icons once the terminals close. */
private var colorsDarkAtDispose = true

/** The system keyboard's height as last seen, in pixels: the keys keyboard takes exactly its place. */
private var lastImeHeight = -1
    get() {
        if (field < 0) field = Clipboard.context.getSharedPreferences("keys", 0).getInt("imeHeight", 0)
        return field
    }
    set(v) {
        if (v == field) return
        field = v
        Clipboard.context.getSharedPreferences("keys", 0).edit().putInt("imeHeight", v).apply()
    }

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TerminalsScreen() {
    val current = AppModel.sessions.firstOrNull { it.id == AppModel.current }
    val terminal = (current as? OpenSession.Terminal)?.session
    var picking by remember { mutableStateOf<NewKind?>(null) }
    val density = LocalDensity.current
    val imeShown = WindowInsets.isImeVisible
    val nav = WindowInsets.navigationBars.getBottom(density)
    // Where the system keyboard is now, and where it will end up once it has moved.
    val imeNow = (WindowInsets.ime.getBottom(density) - nav).coerceAtLeast(0)
    val imeTarget = (WindowInsets.imeAnimationTarget.getBottom(density) - nav).coerceAtLeast(0)
    // Its height once it has settled (a keyboard may say a smaller one first, then add its
    // suggestions' row).
    val settled = imeShown && imeTarget > 0 && imeNow == imeTarget
    if (settled) lastImeHeight = imeTarget
    val panelPx = if (lastImeHeight > 0) lastImeHeight else with(density) { 290.dp.roundToPx() }
    val keys = terminal?.keysKeyboard == true
    val keysUp = keys && terminal?.keysShown == true
    val returning = terminal?.returning == true

    // The system keyboard back: the keys keyboard keeps its place until the other is all the way
    // up, so nothing above moves.
    LaunchedEffect(returning, imeShown, imeNow, imeTarget) {
        if (terminal != null && returning && imeShown && imeTarget > 0 && imeNow >= imeTarget) {
            terminal.keysKeyboard = false
            terminal.keysShown = false
            terminal.returning = false
        }
    }
    // The place kept under the bars for a keyboard: the keys one's, or where the system one ends
    // up. The terminal takes its size from it at once instead of following the keyboard's
    // animation (resizing it on every frame is what makes it jump); the bars ride on the keyboard.
    // The keys keyboard slides up and down as a keyboard does, unless it takes the system one's
    // place (or gives it back): then it is just there.
    val keysTarget = if (keysUp || returning) 1f else 0f
    val keysAnim = remember { androidx.compose.animation.core.Animatable(keysTarget) }
    LaunchedEffect(keysTarget) {
        if (imeShown || imeTarget > 0) keysAnim.snapTo(keysTarget)
        else keysAnim.animateTo(keysTarget, androidx.compose.animation.core.spring(dampingRatio = 1f, stiffness = 600f))
    }
    val keysLift = (panelPx * keysAnim.value).toInt()
    val reserved = when {
        keysUp || returning -> panelPx
        imeShown && !settled && imeTarget > 0 -> maxOf(imeTarget, lastImeHeight)
        else -> imeTarget
    }
    val lift = maxOf(imeNow, keysLift)
    var barsHeight by remember { mutableStateOf(0) }

    // The screen around the terminal in its colour, the status bar's icons readable on it.
    val background = TerminalSettings.theme(colors.dark).background
    val window = (LocalView.current.context as? android.app.Activity)?.window
    val view = LocalView.current
    DisposableEffect(background, colors.dark) {
        val controller = window?.let { androidx.core.view.WindowCompat.getInsetsController(it, view) }
        controller?.isAppearanceLightStatusBars = background.luminance() > 0.5f
        onDispose { controller?.isAppearanceLightStatusBars = !colorsDarkAtDispose }
    }
    colorsDarkAtDispose = colors.dark
    Box(
        Modifier.fillMaxSize().background(background)
            .windowInsetsPadding(WindowInsets.statusBars)
            .windowInsetsPadding(WindowInsets.navigationBars)
    ) {
        Box(Modifier.fillMaxSize().padding(bottom = with(density) { (barsHeight + reserved).toDp() })) {
            when (current) {
                is OpenSession.Terminal -> key(current.id) {
                    TerminalHost(current.session, Modifier.fillMaxSize())
                    Overlay(current.session)
                }
                is OpenSession.Files -> key(current.id) { FilesView(current.files, Modifier.fillMaxSize()) }
                null -> {}
            }
        }
        if (terminal != null && keysLift > 0) {
            KeysKeyboardView(
                terminal,
                Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(with(density) { panelPx.toDp() })
                    .graphicsLayer { translationY = (panelPx - keysLift).toFloat() }
                    .background(colors.bar),
            )
        }
        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth()
                .padding(bottom = with(density) { lift.toDp() })
                .onSizeChanged { barsHeight = it.height }
                .background(colors.bar)
        ) {
            HorizontalDivider(thickness = 0.5.dp, color = colors.separator)
            SessionsBar(terminal) { picking = it }
            if (terminal != null && (!KeyBarStore.hidden || keys)) TerminalBar(terminal, if (keys) keysUp else imeShown)
        }
    }
    picking?.let { kind -> HostPicker(t("New session"), { picking = null }) { if (kind == NewKind.Files) AppModel.openFiles(it) else AppModel.open(it) } }
    // Customize slides in from the right, as a page pushed on iOS.
    androidx.compose.animation.AnimatedVisibility(
        terminal?.customizing == true,
        enter = androidx.compose.animation.slideInHorizontally { it },
        exit = androidx.compose.animation.slideOutHorizontally { it },
    ) { KeyboardEditor(back = terminal?.title ?: t("Back")) { terminal?.customizing = false } }
}

/** Back to the app's tabs: the keyboard goes down with the terminal. */
fun leaveTerminals(terminal: TerminalSession?) {
    terminal?.view?.let { hideKeyboard(it) }
    terminal?.keysShown = false
    AppModel.showTerminals = false
}

/** Swaps the system keyboard and the keys keyboard. */
private fun switchKeyboard(session: TerminalSession) {
    val view = session.view ?: return
    if (session.keysKeyboard) {
        session.returning = true
        showKeyboard(view)
    } else {
        session.keysKeyboard = true
        session.keysShown = true
        hideKeyboard(view)
    }
}

/** Back, the sessions' tabs, and a new session. */
@Composable
private fun SessionsBar(terminal: TerminalSession?, new: (NewKind) -> Unit) {
    var menu by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth().height(48.dp).padding(horizontal = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(40.dp).clickable { leaveTerminals(terminal) }, contentAlignment = Alignment.Center) {
            Icon(Icons.AutoMirrored.Outlined.ArrowBackIos, t("Back"), tint = colors.accent, modifier = Modifier.size(20.dp))
        }
        Row(Modifier.weight(1f).horizontalScroll(rememberScrollState()).animateContentSize(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            for (s in AppModel.sessions) Chip(s, s.id == AppModel.current)
        }
        if (terminal != null) {
            // The system keyboard (ABC) or the keys keyboard.
            Box(Modifier.size(40.dp).clickable { switchKeyboard(terminal) }, contentAlignment = Alignment.Center) {
                if (terminal.keysKeyboard && !terminal.returning) Text("ABC", color = colors.accent, fontSize = 13.sp, fontWeight = FontWeight.Bold)
                else Icon(Icons.Outlined.Apps, t("Keys keyboard"), tint = colors.accent, modifier = Modifier.size(24.dp))
            }
        }
        Box(Modifier.size(40.dp).clickable { menu = true }, contentAlignment = Alignment.Center) {
            Icon(Icons.Outlined.Add, t("New session"), tint = colors.accent, modifier = Modifier.size(24.dp))
            NewSessionMenu(menu, { menu = false }, pick = new)
        }
    }
}

/** A session's tab. The shell may set a long title: it is cut short. */
@Composable
private fun Chip(s: OpenSession, selected: Boolean) {
    val (title, dot) = when (s) {
        is OpenSession.Terminal -> s.session.title to when (s.session.state) {
            TerminalSession.State.Connected -> colors.green
            TerminalSession.State.Connecting -> Color(0xFFFFD60A)
            else -> colors.red
        }
        is OpenSession.Files -> s.host.label to when (s.files.state) {
            FilesSession.State.Ready -> colors.green
            FilesSession.State.Connecting -> Color(0xFFFFD60A)
            else -> colors.red
        }
    }
    Row(
        Modifier.clip(CircleShape).background(if (selected) colors.accent.copy(alpha = 0.2f) else colors.cell)
            .clickable { AppModel.show(s) }.padding(start = 10.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(7.dp).clip(CircleShape).background(dot))
        Spacer(Modifier.width(6.dp))
        Text(title, color = colors.label, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.MiddleEllipsis, modifier = Modifier.widthIn(max = 130.dp))
        Box(Modifier.size(22.dp).clickable { AppModel.close(s) }, contentAlignment = Alignment.Center) {
            Icon(Icons.Outlined.Close, t("Close"), tint = colors.secondary, modifier = Modifier.size(14.dp))
        }
    }
}

/**
 * Over the keyboard: the key bar and the button that shows or hides the keyboard; over the keys
 * keyboard (which has those keys and more), Customize and the same button.
 */
@Composable
private fun TerminalBar(session: TerminalSession, shown: Boolean) {
    Row(Modifier.fillMaxWidth().height(46.dp), verticalAlignment = Alignment.CenterVertically) {
        if (session.keysKeyboard) {
            Row(
                Modifier.weight(1f).padding(horizontal = 6.dp).height(34.dp).clip(RoundedCornerShape(8.dp)).background(colors.fill)
                    .clickable { session.customizing = true },
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Outlined.Tune, null, tint = colors.accent, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text(t("Customize"), color = colors.accent, fontSize = 15.sp, fontWeight = FontWeight.Medium)
            }
        } else {
            Row(
                Modifier.weight(1f).horizontalScroll(rememberScrollState()).padding(horizontal = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                for (k in KeyBarStore.keys) KeyButton(k, session)
            }
        }
        VerticalDivider(Modifier.height(26.dp), color = colors.separator)
        Box(
            Modifier.width(44.dp).fillMaxHeight().clickable {
                if (session.keysKeyboard) session.keysShown = !session.keysShown
                else session.view?.let { if (shown) hideKeyboard(it) else showKeyboard(it) }
            },
            contentAlignment = Alignment.Center,
        ) {
            Icon(if (shown) Icons.Outlined.KeyboardHide else Icons.Outlined.Keyboard, t(if (shown) "Hide keyboard" else "Show keyboard"), tint = colors.label, modifier = Modifier.size(22.dp))
        }
    }
}

@Composable
private fun KeyButton(key: BarKey, session: TerminalSession) {
    val haptic = LocalHapticFeedback.current
    val on = (key.kind == BarKey.Kind.Ctrl && session.ctrl) || (key.kind == BarKey.Kind.Alt && session.alt)
    KeyCap(key, on, Modifier.width(40.dp).height(34.dp)) {
        haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
        when (key.kind) {
            BarKey.Kind.Ctrl -> session.ctrl = !session.ctrl
            BarKey.Kind.Alt -> session.alt = !session.alt
            else -> key.bytes(session.appCursor)?.let { session.send(it) }
        }
    }
}

/** How a key looks, the same in the bar, the keys keyboard and their editor. */
@Composable
fun KeyCap(key: BarKey?, on: Boolean = false, modifier: Modifier = Modifier, onClick: (() -> Unit)? = null) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    Box(
        modifier.clip(RoundedCornerShape(7.dp))
            .background(
                when {
                    key == null -> colors.secondary.copy(alpha = 0.12f)
                    on -> colors.accent
                    pressed -> colors.fill.copy(alpha = 0.6f)
                    else -> colors.fill
                }
            )
            .then(if (onClick != null) Modifier.clickable(interaction, null, onClick = onClick) else Modifier)
            .padding(horizontal = 3.dp),
        contentAlignment = Alignment.Center,
    ) {
        if (key != null) {
            // Every key the same size: a long name gets smaller rather than cut, as on iOS.
            androidx.compose.foundation.text.BasicText(
                key.title,
                style = androidx.compose.ui.text.TextStyle(
                    color = if (on) Color.White else colors.label,
                    fontWeight = FontWeight.Medium,
                    fontFamily = if (key.kind == BarKey.Kind.Snippet) null else FontFamily.Monospace,
                    textAlign = TextAlign.Center,
                ),
                maxLines = if (key.kind == BarKey.Kind.Snippet) 2 else 1,
                autoSize = androidx.compose.foundation.text.TextAutoSize.StepBased(minFontSize = 7.sp, maxFontSize = 15.sp, stepSize = 0.5.sp),
            )
        }
    }
}

/** What the session waits for, over the terminal. */
@Composable
private fun Overlay(session: TerminalSession) {
    // The card comes in softly; it goes at once with the connection made.
    androidx.compose.animation.AnimatedVisibility(
        session.state != TerminalSession.State.Connected,
        enter = androidx.compose.animation.fadeIn() + androidx.compose.animation.scaleIn(initialScale = 0.94f),
        exit = androidx.compose.animation.ExitTransition.None,
    ) { OverlayCard(session) }
}

@Composable
private fun OverlayCard(session: TerminalSession) {
    val state = session.state
    if (state == TerminalSession.State.Connected) return
    var password by remember { mutableStateOf("") }
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            Modifier.padding(16.dp).widthIn(max = 340.dp).clip(RoundedCornerShape(16.dp)).background(colors.cell).padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            when (state) {
                TerminalSession.State.Connecting -> {
                    CircularProgressIndicator(color = colors.accent, modifier = Modifier.size(28.dp))
                    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(t("Connecting"), color = colors.label, fontSize = 15.sp)
                        // One line, however long: the middle gives way.
                        Text(
                            "${session.host.login}@${session.host.address}", color = colors.secondary, fontSize = 13.sp,
                            fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace, maxLines = 1,
                            overflow = androidx.compose.ui.text.style.TextOverflow.MiddleEllipsis,
                        )
                    }
                }
                is TerminalSession.State.Closed -> {
                    Text(if (state.dropped) t("Connection lost") else t("Session ended"), color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
                    ProminentButton(t("Reconnect")) { session.reconnect() }
                }
                is TerminalSession.State.NeedPassword -> {
                    fun submit() {
                        if (password.isEmpty()) return
                        session.connect(secret = password)
                        password = ""
                    }
                    Text(state.prompt.ifEmpty { t("Password") }, color = colors.label, fontSize = 15.sp)
                    Field(password, { password = it }, t("Password"), secure = true, fill = colors.background, onDone = { submit() })
                    ProminentButton(t("Connect"), Modifier.fillMaxWidth(), enabled = password.isNotEmpty()) { submit() }
                }
                is TerminalSession.State.UnknownKey -> {
                    Text(t("New server key"), color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
                    Text(state.fingerprint, color = colors.secondary, fontSize = 12.sp, fontFamily = FontFamily.Monospace, textAlign = TextAlign.Center)
                    ProminentButton(t("Trust and connect")) { session.connect(trust = state.fingerprint) }
                }
                is TerminalSession.State.Failed -> {
                    Text(state.message, color = colors.label, fontSize = 15.sp, textAlign = TextAlign.Center)
                    ProminentButton(t("Retry")) { session.connect() }
                }
                TerminalSession.State.Connected -> {}
            }
        }
    }
}
