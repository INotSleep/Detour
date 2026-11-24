package dev.detour.android.core

import java.text.Collator

/** A group as the hosts screen shows it: its record if it has one, its hosts in order. */
data class ListedGroup(
    /** Tells groups apart, including a group of mine and a shared one of the same name. */
    val key: String,
    /** The group's name; "" for hosts without one. */
    val name: String,
    val record: HostGroup? = null,
    /** A group of this person's own grouping. */
    val mine: MyGroup? = null,
    val hosts: List<Host>,
)

/**
 * Groups of hosts in their order and moving hosts around, as the desktop app does
 * (`src/lib/groups.ts`): the same vault looks and changes the same on every device.
 */
object Groups {
    fun sharedKey(name: String) = "s:$name"
    fun mineKey(id: String) = "m:$id"

    /** The section of hosts in none of this person's groups. */
    val MINE_NONE = mineKey("none")

    private val collator = Collator.getInstance().apply { strength = Collator.SECONDARY }

    fun isNoGroup(g: String) = g.isEmpty() || g == "Без группы" || g == "No group"
    private fun nameOf(h: Host) = if (isNoGroup(h.group)) "" else h.group
    private fun nameOf(h: Map<String, Any?>) = (h.str("group") ?: "").let { if (isNoGroup(it)) "" else it }

    /** Hosts by their order; those without one keep the order they come in, after the rest. */
    fun sortHosts(hosts: List<Host>): List<Host> =
        hosts.withIndex().sortedWith(compareBy<IndexedValue<Host>> { it.value.order ?: Double.POSITIVE_INFINITY }.thenBy { it.index }).map { it.value }

    /**
     * The groups of a vault in order: those with a record by their order (empty ones too, unless
     * `withEmpty` is off), then groups only hosts name, by name, then hosts without a group.
     */
    fun list(hosts: List<Host>, groups: List<HostGroup>, withEmpty: Boolean = true): List<ListedGroup> {
        val byName = LinkedHashMap<String, MutableList<Host>>()
        for (h in hosts) byName.getOrPut(nameOf(h)) { mutableListOf() }.add(h)
        val recorded = groups.sortedBy { it.order }
        val out = recorded.filter { withEmpty || byName.containsKey(it.name) }
            .map { ListedGroup(sharedKey(it.name), it.name, record = it, hosts = sortHosts(byName[it.name] ?: emptyList())) }
            .toMutableList()
        val known = recorded.map { it.name }.toSet()
        byName.keys.filter { it.isNotEmpty() && it !in known }.sortedWith(collator).forEach { out += ListedGroup(sharedKey(it), it, hosts = sortHosts(byName.getValue(it))) }
        byName[""]?.let { out += ListedGroup(sharedKey(""), "", hosts = sortHosts(it)) }
        return out
    }

    /**
     * Groups of mine in their order, then my own "No group": its hosts in my order, then hosts
     * placed nowhere yet (new ones) in the shared order. Shared groups are not mixed in.
     */
    fun listMine(hosts: List<Host>, layout: List<MyGroup>, groups: List<HostGroup>, withEmpty: Boolean = true): List<ListedGroup> {
        val byId = hosts.associateBy { it.id }
        val placed = HashSet<String>()
        fun take(ids: List<String>) = ids.mapNotNull { byId[it] }.filter { placed.add(it.id) }
        val out = ArrayList<ListedGroup>()
        for (g in layout) {
            if (g.name.isEmpty()) continue
            val list = take(g.hosts)
            if (withEmpty || list.isNotEmpty()) out += ListedGroup(mineKey(g.id), g.name, mine = g, hosts = list)
        }
        val none = layout.firstOrNull { it.name.isEmpty() }
        val ordered = take(none?.hosts ?: emptyList())
        val rest = list(hosts.filter { it.id !in placed }, groups, false).flatMap { it.hosts }
        val all = ordered + rest
        if (all.isNotEmpty()) out += ListedGroup(MINE_NONE, "", mine = none, hosts = all)
        return out
    }

    /** A grouping of mine that starts as the shared one. */
    fun layoutFrom(hosts: List<Host>, groups: List<HostGroup>): List<MyGroup> =
        list(hosts, groups, true).filter { it.name.isNotEmpty() }.map { g -> MyGroup(PersonalVault.newId(), g.name, g.hosts.map { it.id }) }

    /** A number between two places (either may be missing). */
    private fun between(a: Double?, b: Double?): Double = when {
        a == null && b == null -> 1.0
        a == null -> b!! - 1
        b == null -> a + 1
        else -> (a + b) / 2
    }

    /**
     * Moving a host into `group` before `before` (or last): the hosts whose group or place change,
     * by id. Hosts of that group without a place get one first, in the order they are shown, so
     * the move lands where it was dropped.
     */
    fun moved(hosts: List<Host>, hostId: String, group: String, before: String?): Map<String, Pair<String, Double>> {
        val target = sortHosts(hosts.filter { it.id != hostId && nameOf(it) == group })
        val places = HashMap<String, Double>()
        if (target.any { it.order == null }) target.forEachIndexed { i, h -> places[h.id] = (i + 1).toDouble() }
        fun placeOf(h: Host) = places[h.id] ?: h.order
        val at = if (before != null) target.indexOfFirst { it.id == before } else target.size
        val i = if (at < 0) target.size else at
        val order = between(if (i > 0) placeOf(target[i - 1]) else null, if (i < target.size) placeOf(target[i]) else null)
        return places.mapValues { group to it.value } + (hostId to (group to order))
    }

    /** The hosts with `moved`'s changes, to show them at once. */
    fun applied(hosts: List<Host>, changes: Map<String, Pair<String, Double>>) =
        hosts.map { h -> changes[h.id]?.let { (g, o) -> h.copy(group = g, order = o) } ?: h }

    /** Saves `moved`'s changes in a vault's data. */
    suspend fun moveHosts(vault: String, changes: Map<String, Pair<String, Double>>) = VaultData.edit(vault) { v ->
        v["hosts"] = v["hosts"].list().map { h ->
            h.obj().toMutableMap().also { m -> changes[m.str("id")]?.let { (g, o) -> m["group"] = g; m["order"] = o } }
        }
    }

    /** Records for every group shown, so each has a place to keep. */
    private fun withRecords(v: MutableMap<String, Any?>): MutableList<MutableMap<String, Any?>> {
        val groups = v["groups"].list().map { it.obj().toMutableMap() }.toMutableList()
        val known = groups.map { it.str("name") }.toSet()
        val missing = v["hosts"].list().map { nameOf(it.obj()) }.filter { it.isNotEmpty() && it !in known }.distinct().sortedWith(collator)
        var last = groups.maxOfOrNull { it.double("order") ?: 0.0 }?.coerceAtLeast(0.0) ?: 0.0
        for (name in missing) groups += mutableMapOf("id" to PersonalVault.newId(), "name" to name, "order" to ++last)
        return groups
    }

    /** A new empty group, last. */
    suspend fun addGroup(vault: String, name: String) = VaultData.edit(vault) { v ->
        val groups = withRecords(v)
        val last = groups.maxOfOrNull { it.double("order") ?: 0.0 }?.coerceAtLeast(0.0) ?: 0.0
        groups += mutableMapOf("id" to PersonalVault.newId(), "name" to name, "order" to last + 1)
        v["groups"] = groups
    }

    /** Renames a group and the group of its hosts. */
    suspend fun renameGroup(vault: String, from: String, to: String) = VaultData.edit(vault) { v ->
        v["groups"] = v["groups"].list().map { g -> g.obj().toMutableMap().also { if (it.str("name") == from) it["name"] = to } }
        v["hosts"] = v["hosts"].list().map { h -> h.obj().toMutableMap().also { if (nameOf(it) == from) it["group"] = to } }
    }

    /** Removes a group; its hosts go to no group. Returns its record and its hosts, for Undo. */
    suspend fun removeGroup(vault: String, name: String): Pair<Map<String, Any?>?, List<String>> {
        var record: Map<String, Any?>? = null
        val members = ArrayList<String>()
        VaultData.edit(vault) { v ->
            record = v["groups"].list().map { it.obj() }.firstOrNull { it.str("name") == name }
            v["groups"] = v["groups"].list().filter { it.obj().str("name") != name }
            v["hosts"] = v["hosts"].list().map { h ->
                h.obj().toMutableMap().also { if (nameOf(it) == name) { members += it.str("id") ?: ""; it["group"] = "" } }
            }
        }
        return record to members
    }

    /** Undoes `removeGroup`. */
    suspend fun restoreGroup(vault: String, name: String, record: Map<String, Any?>?, members: List<String>) = VaultData.edit(vault) { v ->
        val groups = v["groups"].list().map { it.obj() }
        if (record != null && groups.none { it.str("name") == name }) v["groups"] = groups + record
        v["hosts"] = v["hosts"].list().map { h -> h.obj().toMutableMap().also { if (it.str("id") in members) it["group"] = name } }
    }

    // This person's own grouping of a team's hosts

    suspend fun saveLayout(team: String, layout: List<MyGroup>) {
        Core.call("layout_set", mapOf("id" to team, "layout" to mapOf("groups" to layout.map { it.json })))
    }

    /** Moves a host into a group of mine, or into my "No group", before `before` (or last). */
    fun mineMoveHost(layout: List<MyGroup>, listed: List<ListedGroup>, hostId: String, key: String, before: String?): List<MyGroup> {
        val groups = layout.map { g -> g.copy(hosts = g.hosts.filter { it != hostId }) }.toMutableList()
        var i = groups.indexOfFirst { it.name.isNotEmpty() && mineKey(it.id) == key }
        if (key == MINE_NONE) {
            // The whole section keeps the order shown, so the host lands where it was dropped.
            val shown = (listed.firstOrNull { it.key == MINE_NONE }?.hosts ?: emptyList()).map { it.id }.filter { it != hostId }
            i = groups.indexOfFirst { it.name.isEmpty() }
            if (i >= 0) groups[i] = groups[i].copy(hosts = shown) else { groups += MyGroup(PersonalVault.newId(), "", shown); i = groups.lastIndex }
        }
        if (i < 0) return layout
        val hosts = groups[i].hosts.toMutableList()
        val at = before?.let { hosts.indexOf(it) } ?: -1
        hosts.add(if (at < 0) hosts.size else at, hostId)
        groups[i] = groups[i].copy(hosts = hosts)
        return groups
    }

    fun mineAdd(layout: List<MyGroup>, name: String) = layout + MyGroup(PersonalVault.newId(), name, emptyList())
    fun mineRename(layout: List<MyGroup>, id: String, name: String) = layout.map { if (it.id == id) it.copy(name = name) else it }
    fun mineRemove(layout: List<MyGroup>, id: String) = layout.filter { it.id != id }
}
