package dev.detour.android.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ArrowDownward
import androidx.compose.material.icons.outlined.ArrowUpward
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.CreateNewFolder
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DriveFileRenameOutline
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.FileDownload
import androidx.compose.material.icons.outlined.FileUpload
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.ManageAccounts
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.ProvideTextStyle
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.background
import androidx.compose.material3.DropdownMenu as MaterialMenu

// iOS's context menu: a wide rounded card, each action a full row with its icon on the right,
// lines between them, the destructive one in red. Same calls as Material's menu.

/** Counts the rows as they are drawn: a line goes over every one but the first. */
private class MenuRows {
    var count = 0
}

private val LocalMenuRows = staticCompositionLocalOf { MenuRows() }

@Composable
fun DropdownMenu(
    expanded: Boolean,
    onDismissRequest: () -> Unit,
    modifier: Modifier = Modifier,
    offset: DpOffset = DpOffset(0.dp, 0.dp),
    content: @Composable ColumnScope.() -> Unit,
) {
    val rows = remember { MenuRows() }
    MaterialMenu(
        expanded = expanded,
        onDismissRequest = onDismissRequest,
        modifier = modifier.widthIn(min = 240.dp),
        offset = offset,
        shape = RoundedCornerShape(14.dp),
        containerColor = if (colors.dark) Color(0xFF2C2C2E) else Color(0xFFF9F9F9),
        tonalElevation = 0.dp,
        shadowElevation = 8.dp,
    ) {
        rows.count = 0
        CompositionLocalProvider(LocalMenuRows provides rows) { content() }
    }
}

@Composable
fun DropdownMenuItem(
    text: @Composable () -> Unit,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    leadingIcon: (@Composable () -> Unit)? = null,
    trailingIcon: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
) {
    val rows = LocalMenuRows.current
    val first = rows.count == 0
    rows.count++
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    if (!first) HorizontalDivider(thickness = 0.5.dp, color = colors.separator)
    Row(
        modifier.fillMaxWidth().heightIn(min = 44.dp)
            .background(if (pressed) colors.fill else Color.Transparent)
            .clickable(interaction, null, enabled = enabled, onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.weight(1f)) {
            CompositionLocalProvider(LocalContentColor provides if (enabled) colors.label else colors.tertiary) {
                ProvideTextStyle(LocalTextStyle.current.merge(TextStyle(fontSize = 17.sp, color = if (enabled) colors.label else colors.tertiary))) { text() }
            }
        }
        val icon = trailingIcon ?: leadingIcon
        if (icon != null) {
            Spacer(Modifier.width(12.dp))
            Box(Modifier.size(22.dp), contentAlignment = Alignment.Center) {
                CompositionLocalProvider(LocalContentColor provides colors.label) { icon() }
            }
        }
    }
}

/** The icon of a usual action, by its English name; null for others. */
fun menuIcon(action: String): (@Composable () -> Unit)? {
    val vector: ImageVector = when (action) {
        "Connect" -> Icons.Outlined.Terminal
        "Files" -> Icons.Outlined.Folder
        "Copy ssh command", "Copy", "Copy path", "Copy address" -> Icons.Outlined.ContentCopy
        "Edit" -> Icons.Outlined.Edit
        "Delete", "Remove" -> Icons.Outlined.Delete
        "Close" -> Icons.Outlined.Close
        "Rename" -> Icons.Outlined.DriveFileRenameOutline
        "Save" -> Icons.Outlined.FileDownload
        "Upload" -> Icons.Outlined.FileUpload
        "New folder" -> Icons.Outlined.CreateNewFolder
        "Move up" -> Icons.Outlined.ArrowUpward
        "Move down" -> Icons.Outlined.ArrowDownward
        "My login" -> Icons.Outlined.ManageAccounts
        "Settings" -> Icons.Outlined.Tune
        else -> return null
    }
    val red = action == "Delete" || action == "Remove" || action == "Close"
    return { Icon(vector, null, tint = if (red) colors.red else LocalContentColor.current, modifier = Modifier.size(20.dp)) }
}
