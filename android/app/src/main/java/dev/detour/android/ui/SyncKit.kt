package dev.detour.android.ui

import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.text.format.DateUtils
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBackIos
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.UnfoldMore
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.fragment.app.FragmentActivity
import dev.detour.android.core.L10n
import kotlinx.coroutines.launch

// Pieces the Teams, Sync and Settings screens share: pushed pages and the rows iOS forms have.

/** A page pushed over a screen: the back button with the previous title, the title, its list. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PushedPage(
    title: String,
    back: String,
    onBack: () -> Unit,
    right: (@Composable () -> Unit)? = null,
    refresh: (suspend () -> Unit)? = null,
    content: LazyListScope.() -> Unit,
) {
    Column(Modifier.fillMaxSize().background(colors.background)) {
        Box(Modifier.windowInsetsPadding(WindowInsets.statusBars)) {
            SheetBar(title, left = { BackButton(back, onBack) }, right = right)
        }
        if (refresh != null) {
            var refreshing by remember { mutableStateOf(false) }
            val scope = rememberCoroutineScope()
            PullToRefreshBox(refreshing, { scope.launch { refreshing = true; refresh(); refreshing = false } }, Modifier.fillMaxSize()) {
                LazyColumn(Modifier.fillMaxSize()) {
                    content()
                    item { Spacer(Modifier.height(40.dp)) }
                }
            }
        } else {
            LazyColumn(Modifier.fillMaxSize()) {
                content()
                item { Spacer(Modifier.height(40.dp)) }
            }
        }
    }
}

/** iOS's back button: a chevron and the previous screen's title. */
@Composable
fun BackButton(title: String, onClick: () -> Unit) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val tint = colors.accent.copy(alpha = if (pressed) 0.4f else 1f)
    // With little room the label is cut short, with less it goes and the chevron stays.
    androidx.compose.foundation.layout.BoxWithConstraints {
        val roomy = maxWidth >= 64.dp
        Row(Modifier.clickable(interaction, null, onClick = onClick).padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.AutoMirrored.Outlined.ArrowBackIos, title, tint = tint, modifier = Modifier.size(20.dp))
            if (roomy) Text(title, color = tint, fontSize = 17.sp, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
        }
    }
}

/** Shows `target`'s page sliding in from the right while it is set, and out after. */
@Composable
fun <T : Any> Pushed(target: T?, content: @Composable (T) -> Unit) {
    var last by remember { mutableStateOf(target) }
    if (target != null) last = target
    AnimatedVisibility(target != null, enter = slideInHorizontally { it }, exit = slideOutHorizontally { it }) {
        last?.let { content(it) }
    }
}

/** A row that opens a page: an optional icon, the title, its value, a chevron. */
@Composable
fun ChevronCell(title: String, value: String? = null, icon: ImageVector? = null, last: Boolean = false, onClick: () -> Unit) {
    Cell(last = last, onClick = onClick, inset = if (icon != null) 52 else 16) {
        if (icon != null) CellIcon(icon)
        Text(title, color = colors.label, fontSize = 17.sp, modifier = Modifier.weight(1f), maxLines = 1)
        if (value != null) Text(value, color = colors.secondary, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(start = 8.dp))
        Icon(Icons.Outlined.ChevronRight, null, tint = colors.tertiary, modifier = Modifier.padding(start = 4.dp).size(20.dp))
    }
}

/** A button as a row of a list: accent text (red when destructive), a spinner while it works. */
@Composable
fun ButtonCell(
    text: String,
    icon: ImageVector? = null,
    destructive: Boolean = false,
    busy: Boolean = false,
    enabled: Boolean = true,
    last: Boolean = false,
    detail: String? = null,
    onClick: () -> Unit,
) {
    val tint = when {
        !enabled || busy -> colors.tertiary
        destructive -> colors.red
        else -> colors.accent
    }
    Cell(last = last, onClick = if (enabled && !busy) onClick else null, inset = if (icon != null) 52 else 16) {
        if (icon != null) CellIcon(icon, tint)
        Column(Modifier.weight(1f)) {
            Text(text, color = tint, fontSize = 17.sp, maxLines = 2)
            if (detail != null) Text(detail, color = colors.secondary, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (busy) CircularProgressIndicator(Modifier.size(18.dp), color = colors.secondary, strokeWidth = 2.dp)
    }
}

/** An error as a row of a section. */
@Composable
fun ErrorCell(text: String, last: Boolean = true) {
    Cell(last = last) { Text(text, color = colors.red, fontSize = 13.sp) }
}

/** An error under the sections. */
fun LazyListScope.errorItem(text: String?) {
    if (text != null) item { Text(text, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 32.dp, vertical = 8.dp)) }
}

/** iOS's menu picker in a form: the title, the chosen value, a menu of the others. */
@Composable
fun <T> MenuCell(title: String, options: List<Pair<T, String>>, selection: T, last: Boolean = false, onSelect: (T) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        Cell(last = last, onClick = { open = true }) {
            Text(title, color = colors.label, fontSize = 17.sp, modifier = Modifier.weight(1f))
            Text(options.firstOrNull { it.first == selection }?.second ?: "", color = colors.secondary, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Icon(Icons.Outlined.UnfoldMore, null, tint = colors.secondary, modifier = Modifier.padding(start = 4.dp).size(16.dp))
        }
        DropdownMenu(open, { open = false }, Modifier.align(Alignment.TopEnd)) {
            options.forEach { (v, name) ->
                DropdownMenuItem(
                    { Text(name) },
                    { open = false; onSelect(v) },
                    trailingIcon = if (v == selection) ({ Icon(Icons.Outlined.Check, null, tint = colors.accent) }) else null,
                )
            }
        }
    }
}

/** The copy icon, a green check once copied. */
@Composable
fun CopyIcon(copied: Boolean) {
    Icon(
        if (copied) Icons.Outlined.Check else Icons.Outlined.ContentCopy, null,
        tint = if (copied) colors.green else colors.accent, modifier = Modifier.size(20.dp),
    )
}

/** iOS's stepper: minus and plus on one rounded piece. */
@Composable
fun Stepper(canDown: Boolean, canUp: Boolean, down: () -> Unit, up: () -> Unit) {
    Row(Modifier.clip(RoundedCornerShape(8.dp)).background(colors.fill), verticalAlignment = Alignment.CenterVertically) {
        StepperHalf("−", canDown, down)
        Box(Modifier.width(0.5.dp).height(18.dp).background(colors.separator))
        StepperHalf("+", canUp, up)
    }
}

@Composable
private fun StepperHalf(text: String, enabled: Boolean, onClick: () -> Unit) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    Box(
        Modifier.background(if (pressed) colors.fill else Color.Transparent).clickable(interaction, null, enabled = enabled, onClick = onClick)
            .width(46.dp).height(32.dp),
        contentAlignment = Alignment.Center,
    ) { Text(text, color = if (enabled) colors.label else colors.tertiary, fontSize = 20.sp) }
}

/** The system's share sheet for a text. */
fun share(context: Context, text: String) {
    val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)
    context.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

/** "5 minutes ago", in the person's language. */
fun ago(ms: Double): String =
    DateUtils.getRelativeTimeSpanString(ms.toLong(), System.currentTimeMillis(), DateUtils.SECOND_IN_MILLIS).toString()

/** A text with numbers in it (`%lld`). */
fun tCount(key: String, vararg n: Int): String =
    L10n.get(key).replace(Regex("%(\\d+\\$)?(lld|ld|d|@)"), "%$1d").let { runCatching { String.format(it, *n.toTypedArray()) }.getOrDefault(it) }

/** The activity a composable is in. */
fun Context.fragmentActivity(): FragmentActivity? {
    var c: Context? = this
    while (c is ContextWrapper) {
        if (c is FragmentActivity) return c
        c = c.baseContext
    }
    return null
}
