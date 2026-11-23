package dev.detour.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Code
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.detour.android.AppModel
import dev.detour.android.MainTab
import dev.detour.android.core.CoreFailure
import dev.detour.android.core.PersonalVault
import dev.detour.android.core.Snippet
import dev.detour.android.core.SnippetGroup
import dev.detour.android.core.t
import dev.detour.android.terminal.Clipboard
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Snippets of the vault and its teams. A tap opens one: to change, for the personal vault's; to
 * read and copy, for a team's. Deleting one can be undone for a few seconds.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SnippetsScreen() {
    /** Null until the first load. */
    var mine by remember { mutableStateOf<List<Snippet>?>(null) }
    var teams by remember { mutableStateOf<List<SnippetGroup>>(emptyList()) }
    /** The editor: a snippet to change, or one with no id for a new one. */
    var editing by remember { mutableStateOf<Snippet?>(null) }
    /** A team's snippet, shown to read and copy. */
    var viewing by remember { mutableStateOf<Snippet?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    /** A snippet just deleted, for Undo. */
    var deleted by remember { mutableStateOf<Snippet?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val refresh = rememberPullToRefreshState()

    suspend fun load() {
        mine = SnippetGroup.personal()
        teams = SnippetGroup.teams()
    }

    fun failure(e: Exception) = if (e is CoreFailure) e["message"] ?: e.kind else e.message

    fun delete(s: Snippet) {
        deleted = s
        scope.launch {
            try {
                PersonalVault.remove("snippets", s.id)
                error = null
            } catch (e: Exception) {
                error = failure(e)
                deleted = null
            }
            load()
        }
    }

    fun undo() {
        val d = deleted ?: return
        deleted = null
        scope.launch {
            try {
                PersonalVault.save("snippets", mapOf("id" to d.id, "name" to d.name, "command" to d.command))
            } catch (e: Exception) {
                error = failure(e)
            }
            load()
        }
    }

    // The tab stays alive behind the others: read again when it comes back and when the vault
    // changes (a sync, another device), as the host list does.
    LaunchedEffect(AppModel.sections, AppModel.tab == MainTab.Snippets) { load() }
    LaunchedEffect(deleted) {
        if (deleted != null) { delay(5000); deleted = null }
    }

    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.windowInsetsPadding(WindowInsets.statusBars)) {
                SheetBar(t("Snippets"), right = { BarIcon(Icons.Outlined.Add, t("New snippet")) { editing = Snippet("", "", "") } })
            }
            PullToRefreshBox(
                refreshing,
                onRefresh = { scope.launch { refreshing = true; load(); refreshing = false } },
                Modifier.weight(1f).fillMaxWidth(),
                state = refresh,
                indicator = {
                    PullToRefreshDefaults.Indicator(refresh, refreshing, Modifier.align(Alignment.TopCenter), containerColor = colors.cell, color = colors.accent)
                },
            ) {
                LazyColumn(Modifier.fillMaxSize()) {
                    val m = mine
                    if (m == null) {
                        item {
                            Box(Modifier.fillMaxWidth().padding(top = 160.dp), contentAlignment = Alignment.Center) {
                                CircularProgressIndicator(color = colors.secondary)
                            }
                        }
                    } else if (m.isEmpty() && teams.isEmpty()) {
                        item { EmptyState(Icons.Outlined.Code, t("No snippets"), t("New snippet")) { editing = Snippet("", "", "") } }
                    }
                    if (!m.isNullOrEmpty()) {
                        section(if (teams.isEmpty()) null else t("My snippets"), key = "personal") {
                            m.forEachIndexed { i, s ->
                                SnippetRow(s, last = i == m.lastIndex, open = { editing = s }, edit = { editing = s }, delete = { delete(s) })
                            }
                        }
                    }
                    for (g in teams) {
                        section(g.name, key = g.id) {
                            g.snippets.forEachIndexed { i, s ->
                                SnippetRow(s, last = i == g.snippets.lastIndex, open = { viewing = s })
                            }
                        }
                    }
                    error?.let { e -> item { Text(e, color = colors.red, fontSize = 13.sp, modifier = Modifier.padding(32.dp, 8.dp)) } }
                    item { Spacer(Modifier.height(80.dp)) }
                }
            }
        }
        UndoBar(deleted?.let { t("Deleted %@", it.name) }, Modifier.align(Alignment.BottomCenter)) { undo() }
    }
    editing?.let { s -> SnippetEditor(s.takeIf { it.id.isNotEmpty() }, saved = { load() }) { editing = null } }
    viewing?.let { s -> SnippetViewer(s) { viewing = null } }
}

/** A snippet's name and its command under it; a long press shows what iOS puts in a swipe. */
@Composable
private fun SnippetRow(s: Snippet, last: Boolean, open: () -> Unit, edit: (() -> Unit)? = null, delete: (() -> Unit)? = null) {
    var menu by remember { mutableStateOf(false) }
    Box {
        SwipeActions(listOfNotNull(delete?.let { deleteAction(it) }, edit?.let { editAction(it) })) {
            Cell(last = last, onClick = open, onLongClick = { menu = true }) {
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                    Text(s.name, color = colors.label, fontSize = 17.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(s.command, color = colors.secondary, fontSize = 12.sp, fontFamily = FontFamily.Monospace, maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        DropdownMenu(menu, { menu = false }) {
            if (edit != null) DropdownMenuItem(leadingIcon = menuIcon("Edit"), text = { Text(t("Edit")) }, onClick = { menu = false; edit() })
            DropdownMenuItem(leadingIcon = menuIcon("Copy"), text = { Text(t("Copy")) }, onClick = { menu = false; Clipboard.copy(s.command) })
            if (delete != null) DropdownMenuItem(leadingIcon = menuIcon("Delete"), text = { Text(t("Delete"), color = colors.red) }, onClick = { menu = false; delete() })
        }
    }
}
