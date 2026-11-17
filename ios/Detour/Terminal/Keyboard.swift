import GameController
import UIKit

/// Where the on-screen keyboard is, and whether a hardware one is attached. The terminal lays
/// itself out from this instead of letting SwiftUI animate it along with the keyboard.
@MainActor
final class KeyboardObserver: ObservableObject {
    static let shared = KeyboardObserver()

    /// The keyboard's top edge in window points; infinity while it is hidden.
    @Published private(set) var top: CGFloat = .infinity
    @Published private(set) var duration: Double = 0.25
    @Published private(set) var hardware = KeyboardObserver.attached

    var shown: Bool { top.isFinite }

    private var observers: [NSObjectProtocol] = []

    private init() {
        let center = NotificationCenter.default
        observers.append(center.addObserver(forName: UIResponder.keyboardWillChangeFrameNotification, object: nil, queue: .main) { [weak self] n in
            let frame = (n.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue
            let duration = n.userInfo?[UIResponder.keyboardAnimationDurationUserInfoKey] as? Double
            MainActor.assumeIsolated { self?.changed(frame, duration) }
        })
        observers.append(center.addObserver(forName: UIResponder.keyboardWillHideNotification, object: nil, queue: .main) { [weak self] n in
            let duration = n.userInfo?[UIResponder.keyboardAnimationDurationUserInfoKey] as? Double
            MainActor.assumeIsolated { self?.changed(nil, duration) }
        })
        for name in [NSNotification.Name.GCKeyboardDidConnect, .GCKeyboardDidDisconnect] {
            observers.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.hardware = KeyboardObserver.attached }
            })
        }
    }

    /// A hardware keyboard is attached (never in screenshots: the simulator always has the Mac's).
    private static var attached: Bool {
        #if DEBUG
        if Shots.screen != nil { return false }
        #endif
        return GCKeyboard.coalesced != nil
    }

    private func changed(_ frame: CGRect?, _ duration: Double?) {
        if let duration, duration > 0 { self.duration = duration }
        let screen = UIScreen.main.bounds.height
        guard let frame, frame.minY < screen, frame.height > 0 else {
            top = .infinity
            return
        }
        top = frame.minY
    }
}
