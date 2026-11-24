package dev.detour.android.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentWidth
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.text.PlatformTextStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Code
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Groups
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Shield
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.zIndex
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.animation.core.Animatable
import dev.detour.android.AppModel
import dev.detour.android.MainTab
import dev.detour.android.core.t
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch

val MainTab.title: String
    get() = when (this) {
        MainTab.Hosts -> t("Hosts")
        MainTab.Sessions -> t("Sessions")
        // "Keys" is the keyboard's keys elsewhere: SSH keys have a word of their own.
        MainTab.Keys -> t("tab.keys").let { if (it == "tab.keys") "Keys" else it }
        MainTab.Snippets -> t("Snippets")
        MainTab.Proxies -> t("Proxies")
        MainTab.Teams -> t("Teams")
        MainTab.Settings -> t("Settings")
    }

val MainTab.icon: ImageVector
    get() = when (this) {
        MainTab.Hosts -> Icons.Outlined.Dns
        MainTab.Sessions -> Icons.Outlined.Terminal
        MainTab.Keys -> Icons.Outlined.Key
        MainTab.Snippets -> Icons.Outlined.Code
        MainTab.Proxies -> Icons.Outlined.Shield
        MainTab.Teams -> Icons.Outlined.Groups
        MainTab.Settings -> Icons.Outlined.Settings
    }

/**
 * The app's screens with the tab bar of all seven at the bottom. A tab is made when first opened
 * and keeps its state after; going to another tab slides its screen in from that tab's side.
 */
@Composable
fun MainScreen() {
    // The tab chosen is made at once (the app can come back on any tab, its state kept).
    val opened = remember { mutableStateMapOf(MainTab.Hosts to true, AppModel.tab to true) }
    var leaving by remember { mutableStateOf<MainTab?>(null) }
    var shown by remember { mutableStateOf(AppModel.tab) }
    val slide = remember { Animatable(0f) }
    var dir by remember { mutableStateOf(1f) }

    LaunchedEffect(AppModel.tab) {
        val new = AppModel.tab
        if (new == shown) return@LaunchedEffect
        opened[new] = true
        dir = if (new.ordinal > shown.ordinal) 1f else -1f
        leaving = shown
        shown = new
        slide.snapTo(1f)
        slide.animateTo(0f, spring(dampingRatio = 0.9f, stiffness = 380f))
        leaving = null
    }

    Column(Modifier.fillMaxSize()) {
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth().clipToBounds()) {
            val width = constraints.maxWidth.toFloat()
            for (tab in MainTab.entries) {
                if (opened[tab] != true) continue
                val visible = tab == shown || tab == leaving
                Box(
                    // The screen shown on top: the others (kept for their state) get no touches.
                    Modifier.fillMaxSize().zIndex(if (tab == shown) 2f else if (tab == leaving) 1f else 0f).graphicsLayer {
                        alpha = if (visible) 1f else 0f
                        translationX = when (tab) {
                            shown -> dir * width * slide.value
                            leaving -> -dir * width * (1f - slide.value)
                            else -> 0f
                        }
                    }
                ) {
                    if (visible || opened[tab] == true) Screen(tab, active = tab == shown)
                }
            }
        }
        TabBar(AppModel.tab, AppModel.sessions.size) { AppModel.tab = it }
    }
}

@Composable
private fun Screen(tab: MainTab, active: Boolean) {
    // The screens behind keep their state but take no touches.
    Box(Modifier.fillMaxSize().then(if (active) Modifier else Modifier.blockTouches())) {
      // Each tab its own pinned bar's state (its large title fills it in).
      androidx.compose.runtime.CompositionLocalProvider(LocalTitleBar provides remember { TitleBarState() }) {
        when (tab) {
            MainTab.Hosts -> HostsScreen()
            MainTab.Sessions -> SessionsScreen()
            MainTab.Keys -> KeysScreen()
            MainTab.Snippets -> SnippetsScreen()
            MainTab.Proxies -> ProxiesScreen()
            MainTab.Teams -> TeamsScreen()
            MainTab.Settings -> SettingsScreen()
        }
      }
    }
}

/**
 * Seven tabs in one row: the chosen one an icon and its name in a pill, the others icons only.
 * The pill slides to the tab tapped while that tab widens for its name and the last one narrows.
 */
@Composable
fun TabBar(selection: MainTab, sessions: Int, select: (MainTab) -> Unit) {
    val haptic = LocalHapticFeedback.current
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    val labelStyle = TextStyle(fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
    // How open each tab is: 1 for the chosen one, 0 for the rest, springing between.
    val open = MainTab.entries.map { tab ->
        animateFloatAsState(if (tab == selection) 1f else 0f, spring(dampingRatio = 1f, stiffness = 420f), label = "tab").value
    }
    Column(Modifier.fillMaxWidth().background(colors.bar).windowInsetsPadding(WindowInsets.navigationBars)) {
        HorizontalDivider(thickness = 0.5.dp, color = colors.separator)
        BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = 8.dp).padding(top = 6.dp, bottom = 4.dp).height(40.dp)) {
            val total = maxWidth
            val gap = 2.dp
            val icon = 26.dp
            // What the chosen tab adds to its width: the gap, its name and the pill's padding.
            val extras = MainTab.entries.map { tab ->
                with(density) { measurer.measure(tab.title, labelStyle).size.width.toDp() } + 6.dp + 28.dp
            }
            val grown = MainTab.entries.indices.sumOf { (extras[it] * open[it]).value.toDouble() }.dp
            val base = (total - gap * (MainTab.entries.size - 1) - grown) / MainTab.entries.size
            val widths = MainTab.entries.indices.map { base + extras[it] * open[it] }
            val lefts = widths.runningFold(0.dp) { x, w -> x + w + gap }
            // The pill: where the tabs are open, weighted by how open they are.
            val weight = open.sum().coerceAtLeast(0.001f)
            val pillLeft = MainTab.entries.indices.sumOf { (lefts[it] * open[it]).value.toDouble() }.dp / weight
            val pillWidth = MainTab.entries.indices.sumOf { (widths[it] * open[it]).value.toDouble() }.dp / weight
            Box(Modifier.offset(x = pillLeft).width(pillWidth).fillMaxHeight().clip(RoundedCornerShape(20.dp)).background(colors.accent.copy(alpha = 0.16f)))
            MainTab.entries.forEachIndexed { i, tab ->
                val p = open[i]
                val tint = lerp(colors.secondary, colors.accent, p)
                Box(
                    Modifier.offset(x = lefts[i]).width(widths[i]).fillMaxHeight()
                        .clickable(remember { MutableInteractionSource() }, null) {
                            haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                            select(tab)
                        },
                ) {
                    // Icon and name together, centred: the name grows in from the icon's side.
                    val content = icon + (extras[i] - 28.dp) * p
                    Box(Modifier.offset(x = (widths[i] - content) / 2).fillMaxHeight().width(content)) {
                        Box(Modifier.align(Alignment.CenterStart).size(icon, 24.dp), contentAlignment = Alignment.Center) {
                            Icon(tab.icon, tab.title, tint = tint, modifier = Modifier.size(24.dp))
                            if (tab == MainTab.Sessions && sessions > 0) Badge(sessions)
                        }
                        if (p > 0.01f) {
                            // Only the name is cut by the narrowing tab: the badge on the icon isn't.
                            Box(Modifier.align(Alignment.CenterStart).offset(x = icon + 6.dp).width(((extras[i] - 34.dp) * p).coerceAtLeast(0.dp)).clipToBounds()) {
                            Text(
                                tab.title, style = labelStyle, color = colors.accent, maxLines = 1, softWrap = false,
                                modifier = Modifier.wrapContentWidth(Alignment.Start, unbounded = true)
                                    .graphicsLayer {
                                        // In late, out early: never cut by the narrowing tab.
                                        alpha = ((p - 0.3f) / 0.7f).coerceIn(0f, 1f)
                                        val s = 0.8f + 0.2f * p
                                        scaleX = s
                                        scaleY = s
                                        transformOrigin = TransformOrigin(0f, 0.5f)
                                    },
                            )
                            }
                        }
                    }
                }
            }
        }
    }
}

/** The number of open sessions on the Sessions icon: a red dot with the count. */
@Composable
private fun BoxScope.Badge(count: Int) {
    Box(
        Modifier.align(Alignment.TopEnd).offset(x = 7.dp, y = (-5).dp)
            .height(16.dp).widthIn(min = 16.dp).clip(CircleShape).background(colors.red).padding(horizontal = 4.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            if (count > 99) "99+" else "$count", color = Color.White,
            style = TextStyle(fontSize = 10.sp, fontWeight = FontWeight.Bold, lineHeight = 10.sp, platformStyle = PlatformTextStyle(includeFontPadding = false)),
        )
    }
}
