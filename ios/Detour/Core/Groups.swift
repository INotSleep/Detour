import Foundation

/// A group as the hosts screen shows it: its record if it has one, its hosts in order.
struct ListedGroup: Identifiable {
    /// Tells groups apart, including a group of mine and a shared one of the same name.
    let key: String
    /// The group's name; "" for hosts without one.
    let name: String
    var record: HostGroup?
    /// A group of this person's own grouping.
    var mine: MyGroup?
    let hosts: [Host]

    var id: String { key }
}

/// Groups of hosts in their order and moving hosts around, as the desktop app does
/// (`src/lib/groups.ts`): the same vault looks and changes the same on every device.
enum Groups {
    typealias JSON = VaultData.JSON

    static func sharedKey(_ name: String) -> String { "s:\(name)" }
    static func mineKey(_ id: String) -> String { "m:\(id)" }
    /// The section of hosts in none of this person's groups.
    static let mineNone = mineKey("none")

    static func isNoGroup(_ g: String) -> Bool { g.isEmpty || g == "Без группы" || g == "No group" }
    private static func nameOf(_ h: Host) -> String { isNoGroup(h.group) ? "" : h.group }
    private static func nameOf(_ h: JSON) -> String { let g = h["group"] as? String ?? ""; return isNoGroup(g) ? "" : g }

    /// Hosts by their order; those without one keep the order they come in, after the rest.
    static func sortHosts(_ hosts: [Host]) -> [Host] {
        hosts.enumerated().sorted { a, b in
            let x = a.element.order ?? .infinity, y = b.element.order ?? .infinity
            return x != y ? x < y : a.offset < b.offset
        }.map(\.element)
    }

    /// The groups of a vault in order: those with a record by their order (empty ones too, unless
    /// `withEmpty` is off), then groups only hosts name, by name, then hosts without a group.
    static func list(_ hosts: [Host], _ groups: [HostGroup], withEmpty: Bool = true) -> [ListedGroup] {
        var byName: [String: [Host]] = [:]
        var names: [String] = []
        for h in hosts {
            let n = nameOf(h)
            if byName[n] == nil { names.append(n) }
            byName[n, default: []].append(h)
        }
        let recorded = groups.enumerated().sorted { ($0.element.order ?? 0, $0.offset) < ($1.element.order ?? 0, $1.offset) }.map(\.element)
        var out = recorded.filter { withEmpty || byName[$0.name] != nil }.map {
            ListedGroup(key: sharedKey($0.name), name: $0.name, record: $0, hosts: sortHosts(byName[$0.name] ?? []))
        }
        let known = Set(recorded.map(\.name))
        for n in names.filter({ !$0.isEmpty && !known.contains($0) }).sorted(by: { $0.localizedCompare($1) == .orderedAscending }) {
            out.append(ListedGroup(key: sharedKey(n), name: n, hosts: sortHosts(byName[n]!)))
        }
        if let none = byName[""] { out.append(ListedGroup(key: sharedKey(""), name: "", hosts: sortHosts(none))) }
        return out
    }

    /// Groups of mine in their order, then my own "No group": its hosts in my order, then hosts
    /// placed nowhere yet (new ones) in the shared order. Shared groups are not mixed in.
    static func listMine(_ hosts: [Host], _ layout: [MyGroup], _ groups: [HostGroup], withEmpty: Bool = true) -> [ListedGroup] {
        let byId = Dictionary(hosts.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        var placed = Set<String>()
        func take(_ ids: [String]) -> [Host] { ids.compactMap { byId[$0] }.filter { placed.insert($0.id).inserted } }
        var out: [ListedGroup] = []
        for g in layout where !g.name.isEmpty {
            let list = take(g.hosts)
            if withEmpty || !list.isEmpty { out.append(ListedGroup(key: mineKey(g.id), name: g.name, mine: g, hosts: list)) }
        }
        let none = layout.first { $0.name.isEmpty }
        let ordered = take(none?.hosts ?? [])
        let rest = list(hosts.filter { !placed.contains($0.id) }, groups, withEmpty: false).flatMap(\.hosts)
        let all = ordered + rest
        if !all.isEmpty { out.append(ListedGroup(key: mineNone, name: "", mine: none, hosts: all)) }
        return out
    }

    /// A grouping of mine that starts as the shared one.
    static func layoutFrom(_ hosts: [Host], _ groups: [HostGroup]) -> [MyGroup] {
        list(hosts, groups).filter { !$0.name.isEmpty }.map { MyGroup(id: PersonalVault.newId(), name: $0.name, hosts: $0.hosts.map(\.id)) }
    }

    /// A number between two places (either may be missing).
    private static func between(_ a: Double?, _ b: Double?) -> Double {
        switch (a, b) {
        case (nil, nil): 1
        case (nil, let b?): b - 1
        case (let a?, nil): a + 1
        case (let a?, let b?): (a + b) / 2
        }
    }

    /// Moving a host into `group` before `before` (or last): the hosts whose group or place change,
    /// by id. Hosts of that group without a place get one first, in the order they are shown, so
    /// the move lands where it was dropped.
    static func moved(_ hosts: [Host], host: String, to group: String, before: String?) -> [String: (group: String, order: Double)] {
        let target = sortHosts(hosts.filter { $0.id != host && nameOf($0) == group })
        var places: [String: Double] = [:]
        if target.contains(where: { $0.order == nil }) { for (i, h) in target.enumerated() { places[h.id] = Double(i + 1) } }
        func placeOf(_ h: Host) -> Double? { places[h.id] ?? h.order }
        let at = before.flatMap { b in target.firstIndex { $0.id == b } } ?? target.count
        let order = between(at > 0 ? placeOf(target[at - 1]) : nil, at < target.count ? placeOf(target[at]) : nil)
        var out = places.mapValues { (group: group, order: $0) }
        out[host] = (group, order)
        return out
    }

    /// The hosts with `moved`'s changes, to show them at once.
    static func applied(_ hosts: [Host], _ changes: [String: (group: String, order: Double)]) -> [Host] {
        hosts.map { h in
            guard let c = changes[h.id] else { return h }
            var h = h
            h = Host(id: h.id, group: c.group, label: h.label, address: h.address, port: h.port, user: h.user, own: h.own, order: c.order, vaultId: h.vaultId, access: h.access, groupUser: h.groupUser)
            return h
        }
    }

    /// Saves `moved`'s changes in a vault's data.
    static func moveHosts(_ vault: String, _ changes: [String: (group: String, order: Double)]) async throws {
        try await VaultData.edit(vault) { v in
            v["hosts"] = (v["hosts"] as? [JSON] ?? []).map { h in
                var h = h
                if let id = h["id"] as? String, let c = changes[id] { h["group"] = c.group; h["order"] = c.order }
                return h
            }
        }
    }

    /// Records for every group shown, so each has a place to keep.
    private static func withRecords(_ v: JSON) -> [JSON] {
        var groups = v["groups"] as? [JSON] ?? []
        let known = Set(groups.compactMap { $0["name"] as? String })
        var seen = Set<String>()
        let missing = (v["hosts"] as? [JSON] ?? []).map(nameOf).filter { !$0.isEmpty && !known.contains($0) && seen.insert($0).inserted }
            .sorted { $0.localizedCompare($1) == .orderedAscending }
        var last = max(0, groups.compactMap { ($0["order"] as? NSNumber)?.doubleValue }.max() ?? 0)
        for name in missing {
            last += 1
            groups.append(["id": PersonalVault.newId(), "name": name, "order": last])
        }
        return groups
    }

    /// A new empty group, last.
    static func addGroup(_ vault: String, _ name: String) async throws {
        try await VaultData.edit(vault) { v in
            var groups = withRecords(v)
            let last = max(0, groups.compactMap { ($0["order"] as? NSNumber)?.doubleValue }.max() ?? 0)
            groups.append(["id": PersonalVault.newId(), "name": name, "order": last + 1])
            v["groups"] = groups
        }
    }

    /// Renames a group and the group of its hosts.
    static func renameGroup(_ vault: String, from: String, to: String) async throws {
        try await VaultData.edit(vault) { v in
            v["groups"] = (v["groups"] as? [JSON] ?? []).map { g in var g = g; if g["name"] as? String == from { g["name"] = to }; return g }
            v["hosts"] = (v["hosts"] as? [JSON] ?? []).map { h in var h = h; if nameOf(h) == from { h["group"] = to }; return h }
        }
    }

    /// Removes a group; its hosts go to no group. Returns its record and its hosts, for Undo.
    static func removeGroup(_ vault: String, _ name: String) async throws -> (record: JSON?, members: [String]) {
        var record: JSON?
        var members: [String] = []
        try await VaultData.edit(vault) { v in
            let groups = v["groups"] as? [JSON] ?? []
            record = groups.first { $0["name"] as? String == name }
            v["groups"] = groups.filter { $0["name"] as? String != name }
            v["hosts"] = (v["hosts"] as? [JSON] ?? []).map { h in
                var h = h
                if nameOf(h) == name { members.append(h["id"] as? String ?? ""); h["group"] = "" }
                return h
            }
        }
        return (record, members)
    }

    /// Undoes `removeGroup`.
    static func restoreGroup(_ vault: String, _ name: String, record: JSON?, members: [String]) async throws {
        try await VaultData.edit(vault) { v in
            let groups = v["groups"] as? [JSON] ?? []
            if let record, !groups.contains(where: { $0["name"] as? String == name }) { v["groups"] = groups + [record] }
            v["hosts"] = (v["hosts"] as? [JSON] ?? []).map { h in
                var h = h
                if let id = h["id"] as? String, members.contains(id) { h["group"] = name }
                return h
            }
        }
    }

    // MARK: this person's own grouping of a team's hosts

    static func saveLayout(_ team: String, _ layout: [MyGroup]) async throws {
        let groups: [[String: Any]] = layout.map { ["id": $0.id, "name": $0.name, "hosts": $0.hosts] }
        _ = try await Core.shared.call("layout_set", ["id": team, "layout": ["groups": groups]])
    }

    /// Moves a host into a group of mine, or into my "No group", before `before` (or last).
    static func mineMoveHost(_ layout: [MyGroup], _ listed: [ListedGroup], host: String, to key: String, before: String?) -> [MyGroup] {
        var groups = layout.map { g in MyGroup(id: g.id, name: g.name, hosts: g.hosts.filter { $0 != host }) }
        var i = groups.firstIndex { !$0.name.isEmpty && mineKey($0.id) == key }
        if key == mineNone {
            // The whole section keeps the order shown, so the host lands where it was dropped.
            let shown = (listed.first { $0.key == mineNone }?.hosts ?? []).map(\.id).filter { $0 != host }
            if let n = groups.firstIndex(where: { $0.name.isEmpty }) {
                groups[n].hosts = shown
                i = n
            } else {
                groups.append(MyGroup(id: PersonalVault.newId(), name: "", hosts: shown))
                i = groups.count - 1
            }
        }
        guard let i else { return layout }
        let at = before.flatMap { groups[i].hosts.firstIndex(of: $0) } ?? groups[i].hosts.count
        groups[i].hosts.insert(host, at: at)
        return groups
    }

    static func mineAdd(_ layout: [MyGroup], _ name: String) -> [MyGroup] { layout + [MyGroup(id: PersonalVault.newId(), name: name, hosts: [])] }
    static func mineRename(_ layout: [MyGroup], _ id: String, _ name: String) -> [MyGroup] { layout.map { $0.id == id ? MyGroup(id: $0.id, name: name, hosts: $0.hosts) : $0 } }
    static func mineRemove(_ layout: [MyGroup], _ id: String) -> [MyGroup] { layout.filter { $0.id != id } }
}
