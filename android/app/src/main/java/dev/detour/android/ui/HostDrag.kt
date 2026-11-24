package dev.detour.android.ui

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.changedToUpIgnoreConsumed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import dev.detour.android.core.Host

/**
 * A host being dragged to another place in its vault: where the finger is, the rows and sections
 * it may land among (their bounds on the screen), and where it would land now.
 */
class HostDrag {
    /** Where a drop would put the host: into `group` before `before` (or last); the line is drawn at `y`. */
    data class Target(val group: String, val before: String?, val y: Float, val left: Float, val right: Float, val noop: Boolean = false)

    private class Section(var bounds: Rect, var hosts: List<String>, var top: Float)

    var host by mutableStateOf<Host?>(null)
        private set
    /** The group it was in. */
    var from = ""
        private set
    /** The finger, on the screen. */
    var pointer by mutableStateOf(Offset.Zero)
        private set
    /** The finger within the row when the drag began. */
    var grab = Offset.Zero
        private set
    var size by mutableStateOf(IntSize.Zero)
        private set
    /** The row's left edge: it moves up and down only, as in a list. */
    var left = 0f
        private set
    var target by mutableStateOf<Target?>(null)
        private set
    /** The vault chip under the finger: hosts can't go there. */
    var overVault by mutableStateOf<String?>(null)
        private set

    private val rows = HashMap<String, Rect>()
    private val sections = LinkedHashMap<String, Section>()
    private val vaults = HashMap<String, Rect>()

    val active: Boolean get() = host != null

    fun row(id: String, bounds: Rect) { rows[id] = bounds }
    fun rowGone(id: String) { rows.remove(id) }
    /** A section's box of rows, its hosts in order, and where its header starts. */
    fun section(key: String, bounds: Rect, hosts: List<String>, top: Float) {
        val s = sections[key]
        if (s != null) { s.bounds = bounds; s.hosts = hosts; s.top = top } else sections[key] = Section(bounds, hosts, top)
    }
    fun sectionGone(key: String) { sections.remove(key) }
    fun vault(id: String, bounds: Rect) { vaults[id] = bounds }

    fun start(h: Host, group: String, at: Offset) {
        val r = rows[h.id] ?: return
        host = h
        from = group
        grab = at - r.topLeft
        left = r.left
        size = IntSize(r.width.toInt(), r.height.toInt())
        pointer = at
        retarget()
    }

    /** Let go: the row goes to its place before the list changes under it. */
    var settling = false
        private set
    /** Where the open gap starts on the screen, while one is open. */
    var gapTop: Float? = null

    fun moveTo(at: Offset, currentVault: String) {
        if (settling) return
        pointer = at
        overVault = vaults.entries.firstOrNull { it.key != currentVault && it.value.contains(at) }?.key
        retarget()
    }

    /** Where it lands now: the section under the finger (or the nearest), before the first row below it. */
    fun retarget() {
        val h = host ?: return
        val y = pointer.y
        if (overVault != null) { target = null; return }
        val entry = sections.entries.firstOrNull { y >= it.value.top && y <= it.value.bounds.bottom }
            ?: sections.entries.minByOrNull { minOf(kotlin.math.abs(y - it.value.top), kotlin.math.abs(y - it.value.bounds.bottom)) }
            ?: run { target = null; return }
        val (key, s) = entry.key to entry.value
        val others = s.hosts.filter { it != h.id }
        var before: String? = null
        var lineY = s.bounds.bottom
        for (id in others) {
            val r = rows[id] ?: continue
            if (y < r.center.y) { before = id; lineY = r.top; break }
        }
        if (others.isEmpty()) lineY = s.bounds.center.y
        else if (before == null) lineY = others.lastOrNull()?.let { rows[it]?.bottom } ?: s.bounds.bottom
        // Back where it was: no move.
        if (key == from) {
            val i = s.hosts.indexOf(h.id)
            if (i >= 0 && s.hosts.getOrNull(i + 1) == before) { target = Target(key, before, lineY, s.bounds.left, s.bounds.right, noop = true); return }
        }
        target = Target(key, before, lineY, s.bounds.left, s.bounds.right)
    }

    /**
     * The finger is lifted: the row slides into the gap it opened (or back to where it was), so
     * that the list, changed after, shows it where it already is. False if it was let go already.
     */
    suspend fun settle(): Boolean {
        val h = host ?: return false
        if (settling) return false
        settling = true
        val to = (if (target != null) gapTop else null) ?: rows[h.id]?.top
        if (to != null) {
            val from = pointer.y
            androidx.compose.animation.core.animate(from, to + grab.y, animationSpec = androidx.compose.animation.core.tween(160)) { y, _ ->
                pointer = Offset(pointer.x, y)
            }
        }
        return true
    }

    /** Ends the drag; returns where it landed, if it moved. */
    fun end(): Pair<Host, Target>? {
        val h = host
        val t = target
        host = null
        target = null
        overVault = null
        settling = false
        gapTop = null
        return if (h != null && t != null && !t.noop) h to t else null
    }
}

/**
 * A row's touches: a tap, a hold (the menu: a light vibration says it took), and a hold that then
 * moves (`drag`, with where the finger is on the screen; the menu is closed by the caller). A
 * small slip of the finger while holding is still a hold.
 */
fun Modifier.tapHoldDrag(
    interaction: MutableInteractionSource,
    onClick: () -> Unit,
    onHold: () -> Unit,
    drag: ((Offset) -> Unit)?,
): Modifier = composed {
    val haptic = LocalHapticFeedback.current
    val coords = remember { arrayOfNulls<LayoutCoordinates>(1) }
    // The gesture runs on while the row shows another host: it calls what the row says now.
    val click by rememberUpdatedState(onClick)
    val hold by rememberUpdatedState(onHold)
    val dragTo by rememberUpdatedState(drag)
    this.onGloballyPositioned { coords[0] = it }.pointerInput(Unit) {
        val dragSlop = 12.dp.toPx()
        awaitEachGesture {
            val down = awaitFirstDown()
            val press = PressInteraction.Press(down.position)
            interaction.tryEmit(press)
            var tapped = false
            val held = withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) {
                var result = false
                while (true) {
                    val c = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: break
                    if (c.changedToUpIgnoreConsumed()) {
                        tapped = !c.isConsumed
                        if (tapped) c.consume()
                        break
                    }
                    // The list scrolls, or a swipe opens the row's actions.
                    if (c.isConsumed || (c.position - down.position).getDistance() > viewConfiguration.touchSlop) break
                }
                result
            } == null
            if (!held) {
                interaction.tryEmit(if (tapped) PressInteraction.Release(press) else PressInteraction.Cancel(press))
                if (tapped) click()
                return@awaitEachGesture
            }
            haptic.performHapticFeedback(HapticFeedbackType.LongPress)
            hold()
            while (true) {
                val c = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: break
                c.consume()
                if (!c.pressed) break
                val d = dragTo
                if (d != null && (c.position - down.position).getDistance() > dragSlop) {
                    interaction.tryEmit(PressInteraction.Cancel(press))
                    coords[0]?.let { d(it.localToRoot(c.position)) }
                    return@awaitEachGesture
                }
            }
            interaction.tryEmit(PressInteraction.Release(press))
        }
    }
}

/**
 * Over a list with rows that can be dragged: once a drag began in a row, the finger is followed
 * here (the row may scroll away and go), and nothing under it scrolls or swipes.
 */
fun Modifier.followDrag(drag: HostDrag, coords: () -> LayoutCoordinates?, move: (Offset) -> Unit, drop: () -> Unit): Modifier = composed {
    // What the screen says now (the vault shown, its groups), not what it said when this began.
    val moveTo by rememberUpdatedState(move)
    val dropNow by rememberUpdatedState(drop)
    val where by rememberUpdatedState(coords)
    this.pointerInput(drag) {
        awaitPointerEventScope {
            while (true) {
                val ev = awaitPointerEvent(PointerEventPass.Initial)
                if (!drag.active) continue
                ev.changes.forEach { it.consume() }
                val c = ev.changes.firstOrNull() ?: continue
                if (ev.changes.none { it.pressed }) dropNow()
                else where()?.let { moveTo(it.localToRoot(c.position)) }
            }
        }
    }
}
