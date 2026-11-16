import SwiftUI

/// The vault's first password, or unlocking it (Face ID when it is on).
struct LockView: View {
    @EnvironmentObject var app: AppModel
    let setup: Bool
    @State private var password = ""
    @State private var repeatPassword = ""
    @State private var error: String?
    @State private var busy = false
    @FocusState private var focused: Bool

    var body: some View {
        VStack(spacing: 18) {
            Spacer()
            Image(systemName: setup ? "terminal" : "lock.fill")
                .font(.system(size: 36, weight: .semibold))
                .foregroundStyle(.tint)
            Text(setup ? "Create your vault" : "Detour is locked").font(.title2.bold())
            VStack(spacing: 10) {
                SecureField(setup ? "New master password" : "Master password", text: $password)
                    .focused($focused)
                    .submitLabel(setup ? .next : .go)
                    .onSubmit { if !setup { go() } }
                if setup {
                    SecureField("Repeat it", text: $repeatPassword).onSubmit(go)
                }
            }
            .textFieldStyle(.roundedBorder)
            .frame(maxWidth: 360)
            if let error { Text(error).font(.footnote).foregroundStyle(.red) }
            Button(action: go) {
                if busy { ProgressView() } else { Text(setup ? "Create" : "Unlock").frame(maxWidth: 360) }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(password.isEmpty || busy)
            if !setup, Keychain.isSaved {
                Button {
                    Task { await biometry() }
                } label: {
                    Label("Unlock with \(Keychain.biometryName)", systemImage: "faceid")
                }
            }
            Spacer()
            Spacer()
        }
        .padding()
        .task {
            if !setup, Keychain.isSaved { await biometry() } else { focused = true }
        }
    }

    private func go() {
        guard !password.isEmpty, !busy else { return }
        if setup {
            if password.count < 8 { error = String(localized: "At least 8 characters"); return }
            if password != repeatPassword { error = String(localized: "The passwords differ"); return }
        }
        busy = true
        error = nil
        Task {
            do {
                if setup { try await app.create(password) } else { try await app.unlock(password, typed: true) }
            } catch let f as CoreFailure {
                error = f.kind == "wrongMasterPassword" ? String(localized: "Wrong master password") : f["message"] ?? f.kind
                password = ""
            } catch {
                self.error = error.localizedDescription
            }
            busy = false
        }
    }

    private func biometry() async {
        guard let saved = await Keychain.load(reason: String(localized: "Unlock Detour")) else {
            focused = true
            return
        }
        busy = true
        do {
            try await app.unlock(saved, typed: false)
        } catch {
            // Changed on another device: the password once more.
            Keychain.delete()
            self.error = String(localized: "Enter the master password")
            focused = true
        }
        busy = false
    }
}
