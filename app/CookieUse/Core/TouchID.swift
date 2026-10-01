import Foundation
import LocalAuthentication

/// How often injecting a session asks for Touch ID.
enum UnlockPolicy: String, CaseIterable, Identifiable {
    case always
    case tenMinutes
    case never

    var id: String { rawValue }

    var title: String {
        switch self {
        case .always: return "Every time"
        case .tenMinutes: return "Once every 10 minutes"
        case .never: return "Never"
        }
    }
}

/// Native Touch ID / password gate. The GUI confirms the user here, then runs
/// the CLI unattended (`--no-confirm`). Falls back to the device password.
///
/// A quick switcher that prompts on every keystroke is unusable, so like a
/// password manager the default policy unlocks for a short window.
@MainActor
enum BiometricGate {
    private static var unlockedUntil: Date?

    static func confirm(reason: String, policy: UnlockPolicy) async -> Bool {
        switch policy {
        case .never:
            return true
        case .tenMinutes:
            if let until = unlockedUntil, until > Date() { return true }
        case .always:
            break
        }
        let ok = await evaluate(reason: reason)
        if ok, policy == .tenMinutes { unlockedUntil = Date().addingTimeInterval(600) }
        return ok
    }

    /// Forget the unlock window (e.g. when the Mac sleeps or the screen locks).
    static func lock() { unlockedUntil = nil }

    private static func evaluate(reason: String) async -> Bool {
        let context = LAContext()
        context.localizedFallbackTitle = "Use Password"
        var error: NSError?
        let policy: LAPolicy = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
            ? .deviceOwnerAuthenticationWithBiometrics
            : .deviceOwnerAuthentication
        // If no auth is configured at all, don't hard-block the user's own machine.
        guard context.canEvaluatePolicy(policy, error: &error) else { return true }
        return await withCheckedContinuation { continuation in
            context.evaluatePolicy(policy, localizedReason: reason) { success, _ in
                continuation.resume(returning: success)
            }
        }
    }
}
