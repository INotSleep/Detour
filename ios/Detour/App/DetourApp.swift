import SwiftUI

@main
struct DetourApp: App {
    @StateObject private var app = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(app)
                .task { await app.start() }
                .onOpenURL { app.open(link: $0) }
        }
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .active: app.resumed()
            case .background: app.backgrounded()
            default: break
            }
        }
    }
}

struct RootView: View {
    @EnvironmentObject var app: AppModel

    var body: some View {
        Group {
            switch app.phase {
            case .loading: ProgressView()
            case .setup: LockView(setup: true)
            case .locked: LockView(setup: false)
            case .ready:
                HostsView()
                    .fullScreenCover(isPresented: $app.showTerminals) {
                        TerminalsView().environmentObject(app)
                    }
            }
        }
        .alert("Unlock with \(Keychain.biometryName) next time?", isPresented: $app.offerBiometry) {
            Button("Use \(Keychain.biometryName)") { app.rememberWithBiometry(true) }
            Button("Not now", role: .cancel) { app.rememberWithBiometry(false) }
        }
    }
}
