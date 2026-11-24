package dev.detour.android.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/** A sheet over the screen, as iOS shows forms: the whole height, rounded on top. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun Sheet(onDismiss: () -> Unit, full: Boolean = true, content: @Composable ColumnScope.() -> Unit) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = state,
        shape = RoundedCornerShape(topStart = 12.dp, topEnd = 12.dp),
        containerColor = colors.background,
        dragHandle = null,
    ) {
        // A fixed height from the screen: a fraction of the sheet's own height changes while the
        // sheet is dragged, and the two chased each other (the sheet bounced up and down).
        val screen = androidx.compose.ui.platform.LocalConfiguration.current.screenHeightDp.dp
        Column(Modifier.fillMaxWidth().then(if (full) Modifier.height(screen * 0.94f) else Modifier), content = content)
    }
}
