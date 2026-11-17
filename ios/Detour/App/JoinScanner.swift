import AVFoundation
import SwiftUI

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
    /// A QR that isn't Detour's was in view a moment ago.
    @State private var foreign = false
    @State private var foreignSince = Date.distantPast

    static var supported: Bool { AVCaptureDevice.default(for: .video) != nil }

    var body: some View {
        NavigationStack {
            Group {
                switch access {
                case nil:
                    ProgressView()
                case true?:
                    QRScanner(seen: seen)
                        .ignoresSafeArea()
                        .overlay {
                            RoundedRectangle(cornerRadius: 20)
                                .stroke(.white.opacity(0.8), lineWidth: 3)
                                .frame(width: 240, height: 240)
                                .allowsHitTesting(false)
                        }
                        .overlay(alignment: .bottom) {
                            if foreign {
                                Text("This isn't a Detour code")
                                    .font(.subheadline.weight(.medium))
                                    .padding(.horizontal, 16)
                                    .padding(.vertical, 10)
                                    .background(.regularMaterial, in: Capsule())
                                    .padding(.bottom, 40)
                                    .transition(.opacity)
                            }
                        }
                        .animation(.easeOut(duration: 0.2), value: foreign)
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

    private func seen(_ text: String) {
        if let link = JoinLink(text) {
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            found(link)
            dismiss()
            return
        }
        // Said once while that QR stays in view, gone a little after it leaves.
        if !foreign { UINotificationFeedbackGenerator().notificationOccurred(.warning) }
        foreign = true
        let at = Date()
        foreignSince = at
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            if foreignSince == at { foreign = false }
        }
    }
}

/// The camera's picture with QR codes read from it: AVFoundation's own detector, light enough
/// for older phones.
private struct QRScanner: UIViewRepresentable {
    let seen: (String) -> Void

    final class Preview: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
        var preview: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
    }

    func makeCoordinator() -> Coordinator { Coordinator(seen: seen) }

    func makeUIView(context: Context) -> Preview {
        let v = Preview()
        v.backgroundColor = .black
        v.preview.videoGravity = .resizeAspectFill
        context.coordinator.start(in: v)
        return v
    }

    func updateUIView(_ v: Preview, context: Context) {
        context.coordinator.seen = seen
    }

    static func dismantleUIView(_ v: Preview, coordinator: Coordinator) {
        coordinator.stop()
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate, @unchecked Sendable {
        private let session = AVCaptureSession()
        private let queue = DispatchQueue(label: "detour.qr")
        var seen: (String) -> Void
        private var done = false

        init(seen: @escaping (String) -> Void) { self.seen = seen }

        func start(in view: Preview) {
            guard let device = AVCaptureDevice.default(for: .video), let input = try? AVCaptureDeviceInput(device: device) else { return }
            session.beginConfiguration()
            session.sessionPreset = .high
            if session.canAddInput(input) { session.addInput(input) }
            let output = AVCaptureMetadataOutput()
            if session.canAddOutput(output) {
                session.addOutput(output)
                output.setMetadataObjectsDelegate(self, queue: .main)
                output.metadataObjectTypes = [.qr]
            }
            session.commitConfiguration()
            view.preview.session = session
            let s = session
            queue.async { s.startRunning() }
        }

        func stop() {
            let s = session
            queue.async { s.stopRunning() }
        }

        func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
            guard !done else { return }
            for o in objects {
                guard let text = (o as? AVMetadataMachineReadableCodeObject)?.stringValue else { continue }
                if JoinLink(text) != nil {
                    done = true
                    stop()
                }
                MainActor.assumeIsolated { seen(text) }
                return
            }
        }
    }
}
