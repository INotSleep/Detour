import AVFoundation
import SwiftUI
import VisionKit

/// A code for joining sync, as another device shows it: `detour://join?code=…&repo=…` (its QR)
/// or the code alone.
struct JoinLink: Equatable {
    let code: String
    let repo: String?

    init?(_ text: String) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if let c = URLComponents(string: t), c.scheme == "detour", c.host == "join" {
            let items = c.queryItems ?? []
            guard let code = items.first(where: { $0.name == "code" })?.value, !code.isEmpty else { return nil }
            self.code = code
            repo = items.first(where: { $0.name == "repo" })?.value.flatMap { $0.isEmpty ? nil : $0 }
        } else if t.range(of: #"^[A-Za-z0-9]{4}(-[A-Za-z0-9]{4}){3,}$"#, options: .regularExpression) != nil {
            code = t
            repo = nil
        } else {
            return nil
        }
    }
}

/// The camera, looking for the other device's QR.
struct JoinScanSheet: View {
    let found: (JoinLink) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var access: Bool?

    static var supported: Bool { DataScannerViewController.isSupported }

    var body: some View {
        NavigationStack {
            Group {
                switch access {
                case nil:
                    ProgressView()
                case true?:
                    QRScanner { link in
                        found(link)
                        dismiss()
                    }
                    .ignoresSafeArea()
                case false?:
                    ContentUnavailableView {
                        Label("No access to the camera", systemImage: "camera")
                    } actions: {
                        Button("Open Settings") {
                            if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                        }
                        .buttonStyle(.borderedProminent)
                    }
                }
            }
            .navigationTitle("Scan the code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            }
        }
        .task {
            switch AVCaptureDevice.authorizationStatus(for: .video) {
            case .authorized: access = true
            case .notDetermined: access = await AVCaptureDevice.requestAccess(for: .video)
            default: access = false
            }
        }
    }
}

private struct QRScanner: UIViewControllerRepresentable {
    let found: (JoinLink) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(found: found) }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let vc = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced, isHighlightingEnabled: true)
        vc.delegate = context.coordinator
        return vc
    }

    func updateUIViewController(_ vc: DataScannerViewController, context: Context) {
        if !vc.isScanning { try? vc.startScanning() }
    }

    @MainActor
    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let found: (JoinLink) -> Void
        private var done = false

        init(found: @escaping (JoinLink) -> Void) { self.found = found }

        nonisolated func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            MainActor.assumeIsolated {
                guard !done else { return }
                for item in addedItems {
                    if case .barcode(let b) = item, let s = b.payloadStringValue, let link = JoinLink(s) {
                        done = true
                        dataScanner.stopScanning()
                        UINotificationFeedbackGenerator().notificationOccurred(.success)
                        found(link)
                        return
                    }
                }
            }
        }
    }
}
