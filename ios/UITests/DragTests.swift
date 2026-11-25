import XCTest

/// Drags hosts into empty groups the way a person does (hold, move, let go) and writes down where
/// each one landed: the app runs with `-shot drag`, a sample vault whose groups are
/// Empty A, Production, Empty B, Staging, Empty C, Lab (App/Shots.swift). The app's own record of
/// the list's moves is in its Documents/drag.txt; screenshots and results go to $DRAG_OUT.
final class DragTests: XCTestCase {
    private var app: XCUIApplication!
    private var out: URL?
    private var lines: [String] = []

    override func setUp() {
        continueAfterFailure = true
        app = XCUIApplication()
        app.launchArguments = ["-shot", "drag", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
        if let dir = ProcessInfo.processInfo.environment["DRAG_OUT"] { out = URL(fileURLWithPath: dir) }
    }

    override func tearDown() {
        guard let out else { return }
        let url = out.appendingPathComponent("result.txt")
        let old = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
        try? (old + lines.joined(separator: "\n") + "\n").write(to: url, atomically: true, encoding: .utf8)
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

    /// Which group's header is the nearest above a host.
    private func groupOf(_ label: String) -> String {
        let y = host(label).frame.minY
        let names = ["Empty A", "Production", "Empty B", "Staging", "Empty C", "Lab"]
        return names.filter { header($0).exists && header($0).frame.maxY < y }.max { header($0).frame.maxY < header($1).frame.maxY } ?? "?"
    }

    private func drag(_ label: String, to target: XCUICoordinate, into group: String, name: String) {
        let h = host(label)
        XCTAssertTrue(h.waitForExistence(timeout: 40), "\(label) shown")
        sleep(2)
        shot("\(name)-0-before")
        let before = hints.count
        h.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5))
            .press(forDuration: 1.2, thenDragTo: target, withVelocity: .slow, thenHoldForDuration: 1.0)
        sleep(3)
        shot("\(name)-1-after")
        let landed = groupOf(label)
        lines.append("\(name): \(label) -> \(landed) (wanted \(group)); hints \(before) -> \(hints.count)")
        XCTAssertEqual(landed, group, name)
    }

    /// Up, onto the middle of an empty group's hint.
    func test1_upOntoHint() {
        XCTAssertTrue(host("web-1").waitForExistence(timeout: 40))
        guard let hint = hint(under: "Empty A") else { return XCTFail("no hint under Empty A") }
        drag("web-1", to: hint.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)), into: "Empty A", name: "1-up-onto-hint")
    }

    /// Down, onto the middle of an empty group's hint.
    func test2_downOntoHint() {
        XCTAssertTrue(host("web-2").waitForExistence(timeout: 40))
        guard let hint = hint(under: "Empty B") else { return XCTFail("no hint under Empty B") }
        drag("web-2", to: hint.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)), into: "Empty B", name: "2-down-onto-hint")
    }

    /// Down, onto the lower half of an empty group's hint.
    func test3_downOntoHintBottom() {
        XCTAssertTrue(host("staging").waitForExistence(timeout: 40))
        guard let hint = hint(under: "Empty C") else { return XCTFail("no hint under Empty C") }
        drag("staging", to: hint.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9)), into: "Empty C", name: "3-down-onto-hint-bottom")
    }
}
