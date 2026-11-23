package dev.detour.android.core

/** A command kept to run again. */
data class Snippet(val id: String, val name: String, val command: String) {
    companion object {
        fun from(m: Map<String, Any?>) = Snippet(m.str("id") ?: "", m.str("name") ?: "", m.str("command") ?: "")
    }
}

/** Snippets of one vault: the personal one (no name) or a team's. */
data class SnippetGroup(val id: String, val name: String?, val snippets: List<Snippet>) {
    companion object {
        /** The personal vault's snippets, if it has any, then each team's that has some. */
        suspend fun load(): List<SnippetGroup> {
            val mine = personal()
            return (if (mine.isEmpty()) emptyList() else listOf(SnippetGroup("personal", null, mine))) + teams()
        }

        /** The personal vault's snippets; none when it can't be read. */
        suspend fun personal(): List<Snippet> =
            runCatching { Core.call("vault_get").obj()["snippets"].list().map { Snippet.from(it.obj()) } }.getOrDefault(emptyList())

        /** The teams that have snippets, each with its own. */
        suspend fun teams(): List<SnippetGroup> =
            runCatching { Core.call("teams_list").list() }.getOrDefault(emptyList()).mapNotNull { t ->
                val view = t.obj()["view"]?.obj() ?: return@mapNotNull null
                val id = view.str("id") ?: return@mapNotNull null
                val snippets = view["data"].obj()["snippets"].list().map { Snippet.from(it.obj()) }
                if (snippets.isEmpty()) null else SnippetGroup(id, view.str("name") ?: "", snippets)
            }
    }
}
