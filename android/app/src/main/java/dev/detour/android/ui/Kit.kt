package dev.detour.android.ui

import androidx.compose.foundation.background
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// iOS's list and bar pieces in Compose: a large title over grouped, rounded sections.

/** What a tab's large title tells the bar pinned over its list: the title, its buttons, where it is. */
class TitleBarState {
    var title by mutableStateOf<String?>(null)
    var actions by mutableStateOf<(@Composable RowScope.() -> Unit)?>(null)
    /** The large title's bottom on the screen; minus infinity once it has scrolled out of the list. */
    var titleBottom by mutableStateOf(Float.MAX_VALUE)
}

val LocalTitleBar = androidx.compose.runtime.staticCompositionLocalOf { TitleBarState() }

/**
 * A tab's large title, the first row of its list. Its buttons sit in the bar pinned over the list
 * (`TitleBar`), where the title shows small once the large one has scrolled under it, as on iOS.
 */
@Composable
fun LargeTitle(title: String, actions: @Composable RowScope.() -> Unit = {}) {
    val bar = LocalTitleBar.current
    androidx.compose.runtime.SideEffect {
        bar.title = title
        bar.actions = actions
    }
    androidx.compose.runtime.DisposableEffect(bar) { onDispose { bar.titleBottom = Float.NEGATIVE_INFINITY } }
    Column(Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.statusBars).padding(horizontal = 16.dp)) {
        Spacer(Modifier.height(44.dp))
        Text(
            title, color = colors.label, fontSize = 34.sp, fontWeight = FontWeight.Bold,
            modifier = Modifier.padding(bottom = 8.dp).onGloballyPositioned { bar.titleBottom = it.boundsInWindow().bottom },
        )
    }
}

/** The bar pinned over a tab's list: its buttons always, its title small once scrolled. */
@Composable
fun androidx.compose.foundation.layout.BoxScope.TitleBar() {
    val bar = LocalTitleBar.current
    var bottom by remember { mutableStateOf(0f) }
    val collapsed = bar.titleBottom < bottom + 2f
    val shown by androidx.compose.animation.core.animateFloatAsState(if (collapsed) 1f else 0f, label = "bar")
    Column(
        Modifier.align(Alignment.TopCenter).fillMaxWidth()
            .background(colors.background.copy(alpha = shown))
            .windowInsetsPadding(WindowInsets.statusBars)
            .onGloballyPositioned { bottom = it.boundsInWindow().bottom }
    ) {
        Box(Modifier.fillMaxWidth().height(44.dp).padding(horizontal = 16.dp)) {
            Text(
                bar.title ?: "", color = colors.label.copy(alpha = shown), fontSize = 17.sp, fontWeight = FontWeight.SemiBold,
                maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.align(Alignment.Center).padding(horizontal = 56.dp),
            )
            Row(Modifier.align(Alignment.CenterEnd), verticalAlignment = Alignment.CenterVertically) { bar.actions?.invoke(this) }
        }
        HorizontalDivider(thickness = 0.5.dp, color = colors.separator.copy(alpha = shown))
    }
}

/**
 * A small title bar: a button on the left (Cancel, Back), the title in the middle, the main action
 * on the right. The title stays centred and is cut short rather than run into the buttons; the
 * buttons get at most what the title leaves them.
 */
@Composable
fun SheetBar(title: String, left: (@Composable () -> Unit)? = null, right: (@Composable () -> Unit)? = null) {
    val gap = with(androidx.compose.ui.platform.LocalDensity.current) { 8.dp.roundToPx() }
    androidx.compose.ui.layout.Layout(
        content = {
            Box { left?.invoke() }
            Text(title, color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Box { right?.invoke() }
        },
        modifier = Modifier.fillMaxWidth().height(52.dp).padding(horizontal = 16.dp),
    ) { (l, t, r), c ->
        val w = c.maxWidth
        val loose = c.copy(minWidth = 0, minHeight = 0)
        // The title as wide as it wants, up to 60% of the bar.
        val titleWant = t.maxIntrinsicWidth(c.maxHeight).coerceAtMost(w * 6 / 10)
        val side = ((w - titleWant) / 2 - gap).coerceAtLeast(0)
        val lp = l.measure(loose.copy(maxWidth = side))
        val rp = r.measure(loose.copy(maxWidth = side))
        val titleMax = (w - 2 * maxOf(lp.width, rp.width) - 2 * gap).coerceAtLeast(0)
        val tp = t.measure(loose.copy(maxWidth = titleMax))
        layout(w, c.maxHeight) {
            lp.place(0, (c.maxHeight - lp.height) / 2)
            tp.place((w - tp.width) / 2, (c.maxHeight - tp.height) / 2)
            rp.place(w - rp.width, (c.maxHeight - rp.height) / 2)
        }
    }
}

/** A text button of a bar. */
@Composable
fun BarButton(text: String, bold: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    Text(
        text,
        color = if (!enabled) colors.tertiary else colors.accent.copy(alpha = if (pressed) 0.4f else 1f),
        fontSize = 17.sp,
        fontWeight = if (bold) FontWeight.SemiBold else FontWeight.Normal,
        modifier = Modifier.clickable(interaction, null, enabled = enabled, onClick = onClick).padding(vertical = 8.dp),
    )
}

/** An icon button of a bar. */
@Composable
fun BarIcon(icon: ImageVector, description: String, tint: Color = colors.accent, onClick: () -> Unit) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    Box(Modifier.size(44.dp).clickable(interaction, null, onClick = onClick), contentAlignment = Alignment.Center) {
        Icon(icon, description, tint = tint.copy(alpha = if (pressed) 0.4f else 1f), modifier = Modifier.size(24.dp))
    }
}

/** A grouped section: an optional header, then its rows on one rounded card. */
fun LazyListScope.section(header: String? = null, footer: String? = null, key: Any? = null, rows: @Composable () -> Unit) {
    item(key = key) {
        // Sections come, go and move smoothly; their rows change height smoothly too.
        Column(Modifier.animateItem().padding(horizontal = 16.dp).padding(top = if (header != null) 18.dp else 22.dp)) {
            if (header != null) {
                Text(header.uppercase(), color = colors.secondary, fontSize = 13.sp, modifier = Modifier.padding(start = 16.dp, bottom = 6.dp))
            }
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).background(colors.cell).animateContentSize()) { rows() }
            if (footer != null) {
                Text(footer, color = colors.secondary, fontSize = 13.sp, modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 6.dp))
            }
        }
    }
}

/** A row of a section, with the separator under it unless it is the last. */
@Composable
fun Cell(
    last: Boolean = false,
    onClick: (() -> Unit)? = null,
    onLongClick: (() -> Unit)? = null,
    inset: Int = 16,
    content: @Composable RowScope.() -> Unit,
) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    Column(Modifier.background(if (pressed) colors.fill else Color.Transparent)) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 44.dp)
                .then(if (onClick != null || onLongClick != null) Modifier.combinedClick(interaction, onClick, onLongClick) else Modifier)
                .padding(horizontal = 16.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            content = content,
        )
        if (!last) HorizontalDivider(Modifier.padding(start = inset.dp), thickness = 0.5.dp, color = colors.separator)
    }
}

/** A cell's leading icon. */
@Composable
fun CellIcon(icon: ImageVector, tint: Color = colors.accent) {
    Icon(icon, null, tint = tint, modifier = Modifier.size(22.dp))
    Spacer(Modifier.width(14.dp))
}

/** A cell's title and a line under it. */
@Composable
fun RowScope.CellText(title: String, detail: String? = null, mono: Boolean = false) {
    Column(Modifier.weight(1f)) {
        Text(title, color = colors.label, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (detail != null) {
            Text(
                detail, color = colors.secondary, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                fontFamily = if (mono) androidx.compose.ui.text.font.FontFamily.Monospace else null,
            )
        }
    }
}

/** A list with nothing in it yet: what it is, and the action that fills it. */
@Composable
fun EmptyState(icon: ImageVector, title: String, action: String? = null, onAction: () -> Unit = {}) {
    Column(Modifier.fillMaxWidth().padding(top = 120.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Icon(icon, null, tint = colors.secondary, modifier = Modifier.size(44.dp))
        Spacer(Modifier.height(12.dp))
        Text(title, color = colors.label, fontSize = 22.sp, fontWeight = FontWeight.Bold)
        if (action != null) {
            Spacer(Modifier.height(16.dp))
            ProminentButton(action, onClick = onAction)
        }
    }
}

/** A filled, rounded button: the screen's main action. */
@Composable
fun ProminentButton(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, busy: Boolean = false, onClick: () -> Unit) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    Box(
        modifier.clip(RoundedCornerShape(12.dp))
            .background(if (enabled) colors.accent.copy(alpha = if (pressed) 0.7f else 1f) else colors.fill)
            .clickable(interaction, null, enabled = enabled && !busy, onClick = onClick)
            .heightIn(min = 50.dp).padding(horizontal = 20.dp),
        contentAlignment = Alignment.Center,
    ) {
        if (busy) androidx.compose.material3.CircularProgressIndicator(Modifier.size(20.dp), color = Color.White, strokeWidth = 2.dp)
        else Text(text, color = if (enabled) Color.White else colors.tertiary, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
    }
}
