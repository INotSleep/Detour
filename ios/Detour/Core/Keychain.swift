import Foundation
import LocalAuthentication
import Security

/// The master password kept on this device behind Face ID (or Touch ID), only if the person
/// turns it on. A new face or finger enrolled takes it away (`biometryCurrentSet`).
enum Keychain {
    private static let service = "dev.detour.ios.master"
    private static let account = "master-password"

    static var isSaved: Bool {
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecUseAuthenticationUI as String: kSecUseAuthenticationUIFail,
        ]
        let status = SecItemCopyMatching(q as CFDictionary, nil)
        return status == errSecSuccess || status == errSecInteractionNotAllowed
    }

    /// Whether this device can unlock with Face ID or Touch ID now.
    static var biometryAvailable: Bool {
        LAContext().canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
    }

    static var biometryName: String {
        let ctx = LAContext()
        _ = ctx.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
        return ctx.biometryType == .touchID ? "Touch ID" : "Face ID"
    }

    static func save(_ password: String) -> Bool {
        delete()
        guard let access = SecAccessControlCreateWithFlags(nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, .biometryCurrentSet, nil) else { return false }
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessControl as String: access,
            kSecValueData as String: Data(password.utf8),
        ]
        return SecItemAdd(q as CFDictionary, nil) == errSecSuccess
    }

    /// The saved password, after Face ID; nil if cancelled or gone.
    static func load(reason: String) async -> String? {
        await withCheckedContinuation { cont in
            DispatchQueue.global(qos: .userInitiated).async {
                let ctx = LAContext()
                ctx.localizedReason = reason
                let q: [String: Any] = [
                    kSecClass as String: kSecClassGenericPassword,
                    kSecAttrService as String: service,
                    kSecAttrAccount as String: account,
                    kSecReturnData as String: true,
                    kSecUseAuthenticationContext as String: ctx,
                ]
                var out: CFTypeRef?
                let status = SecItemCopyMatching(q as CFDictionary, &out)
                if status == errSecSuccess, let data = out as? Data {
                    cont.resume(returning: String(data: data, encoding: .utf8))
                } else {
                    cont.resume(returning: nil)
                }
            }
        }
    }

    static func delete() {
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(q as CFDictionary)
    }
}
