import XCTest

/// Drags hosts into empty groups the way a person does (hold, move, let go) and writes down where
/// each one landed: the app runs with `-shot drag`, a sample vault whose groups are
/// Empty A, Production, Empty B, Staging, Empty C, Lab (App/Shots.swift), made again on every
/// launch. `-dragmode` picks a variant of the list. The app's own record of the list's moves is
/// in its Documents/drag.txt; screenshots and results go to $DRAG_OUT.
final class DragTests: XCTestCase {
    private var app: XCUIApplication!
    private var out: URL?

    override func setUp() {
        continueAfterFailure = true
        if let dir = ProcessInfo.processInfo.environment["DRAG_OUT"] { out = URL(fileURLWithPath: dir) }
    }

    private func launch(_ mode: String, _ name: String) {
        app = XCUIApplication()
        app.launchArguments = ["-shot", "drag", "-dragmode", mode, "-dragcase", name, "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
        XCTAssertTrue(host("web-1").waitForExistence(timeout: 40), "hosts shown")
        sleep(2)
    }

    private func record(_ line: String) {
        guard let out else { return }
        let url = out.appendingPathComponent("result.txt")
        let old = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
        try? (old + line + "\n").write(to: url, atomically: true, encoding: .utf8)
    }

    private func host(_ label: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", label + ",")).firstMatch
    }

    private func header(_ name: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label ==[c] %@", name)).firstMatch
    }

    private var hints: [XCUIElement] {
        app.staticTexts.matching(NSPredicate(format: "label == %@", "Drag hosts here")).allElementsBoundByIndex
    }

    /// The hint under a group's header.
    private func hint(under name: String) -> XCUIElement? {
        let top = header(name).frame.maxY
        return hints.filter { $0.frame.minY > top }.min { $0.frame.minY < $1.frame.minY }
    }

    private func shot(_ name: String) {
        guard let out else { return }
        try? XCUIScreen.main.screenshot().pngRepresentation.write(to: out.appendingPathComponent("\(name).png"))
    }

    private static let names = ["Empty A", "Production", "Empty B", "Staging", "Empty C", "Lab"]

    /// Which group's header is the nearest above a point.
    private func groupAt(_ y: CGFloat) -> String {
        Self.names.filter { header($0).exists && header($0).frame.maxY < y }.max { header($0).frame.maxY < header($1).frame.maxY } ?? "?"
    }

    /// Up onto an empty group's hint (1), down onto one (2), down onto its lower edge (3).
    private func drags(_ mode: String) {
        let cases: [(String, String, String, CGFloat)] = [
            ("1-up", "web-1", "Empty A", 0.5),
            ("2-down", "web-2", "Empty B", 0.5),
            ("3-down-low", "staging", "Empty C", 0.9),
        ]
        for (name, label, group, dy) in cases {
            let id = "\(mode)-\(name)"
            launch(mode, id)
            guard let target = hint(under: group) else { record("\(id): no hint under \(group)"); continue }
            shot("\(id)-0-before")
            let before = hints.count
            let y0 = host(label).frame.minY
            let how = drag(host(label), to: target.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: dy)), moved: { self.host(label).frame.minY != y0 })
            shot("\(id)-1-after")
            let landed = groupAt(host(label).frame.minY)
            record("\(id): \(label) -> \(landed) (wanted \(group)); hints \(before) -> \(hints.count); \(how)")
            XCTAssertEqual(landed, group, id)
        }
    }

    private var menuShown: Bool { app.buttons["Copy ssh command"].exists }

    /// Holds an element and moves it to `target`. A hold that opens the row's menu instead of
    /// lifting it is tried again, longer or shorter; says which one did something.
    @discardableResult
    private func drag(_ e: XCUIElement, to target: XCUICoordinate, moved: () -> Bool) -> String {
        var tried: [String] = []
        for (hold, speed) in [(0.6, XCUIGestureVelocity.default), (0.9, .slow), (0.4, .default), (1.5, .slow)] {
            e.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5))
                .press(forDuration: hold, thenDragTo: target, withVelocity: speed, thenHoldForDuration: 1.0)
            sleep(3)
            if menuShown {
                tried.append("\(hold)s: menu")
                app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.04)).tap()
                sleep(1)
                continue
            }
            let did = moved()
            tried.append("\(hold)s: \(did ? "moved" : "no move")")
            if did { break }
        }
        return tried.joined(separator: ", ")
    }

    /// Holding an empty group's hint and moving it: it should stay where it is.
    private func lift(_ mode: String) {
        let id = "\(mode)-lift"
        launch(mode, id)
        guard let h = hint(under: "Empty A") else { return record("\(id): no hint") }
        let y = h.frame.minY
        h.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 0.8, thenDragTo: host("db-primary").coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)),
                   withVelocity: .slow, thenHoldForDuration: 1.0)
        sleep(3)
        shot("\(id)-after")
        let now = hint(under: "Empty A")
        record("\(id): hint under Empty A at \(Int(y)) -> \(now.map { "\(Int($0.frame.minY))" } ?? "gone"), group of first hint now \(hints.first.map { groupAt($0.frame.minY) } ?? "-")")
    }

    /// Between groups that have hosts: shows the drag itself works.
    private func control(_ mode: String) {
        let id = "\(mode)-0-control"
        launch(mode, id)
        let y0 = host("web-1").frame.minY
        let how = drag(host("web-1"), to: host("ci-runner").coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)), moved: { self.host("web-1").frame.minY != y0 })
        shot("\(id)-after")
        record("\(id): web-1 -> \(groupAt(host("web-1").frame.minY)) (wanted Staging); \(how)")
    }

    func testG() { lift("G"); drags("G") }
    func testJ() { lift("J"); drags("J") }
    func testK() { lift("K"); drags("K") }
    func testL() { lift("L"); drags("L") }
    /// Lifted anyway: comes back.
    func testD() { lift("D") }
}
