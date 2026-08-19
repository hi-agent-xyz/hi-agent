import AVFoundation
import SwiftUI
import UIKit
import VisionKit

struct PairingQRScannerView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    let onScan: (PairingRequest) -> Void

    @State private var cameraState = CameraState.checking
    @State private var scanError: String?

    var body: some View {
        NavigationStack {
            Group {
                switch cameraState {
                case .checking:
                    ProgressView("Preparing camera...")
                case .ready:
                    scanner
                case .denied:
                    VStack(spacing: 18) {
                        ContentUnavailableView(
                            "Camera access is off",
                            systemImage: "camera.fill",
                            description: Text("Allow camera access in Settings, or enter the pairing code manually.")
                        )
                        Button {
                            guard let settingsURL = URL(string: UIApplication.openSettingsURLString)
                            else {
                                return
                            }
                            openURL(settingsURL)
                        } label: {
                            Label("Open Settings", systemImage: "gear")
                        }
                        .buttonStyle(.borderedProminent)
                    }
                case .unavailable:
                    ContentUnavailableView(
                        "QR scanning is unavailable",
                        systemImage: "qrcode.viewfinder",
                        description: Text("Enter the core address and pairing code manually.")
                    )
                }
            }
            .navigationTitle("Scan pairing code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        dismiss()
                    }
                }
            }
        }
        .task {
            await prepareCamera()
        }
    }

    private var scanner: some View {
        ZStack(alignment: .bottom) {
            PairingQRScannerController { payload in
                handle(payload)
            }
            .ignoresSafeArea()

            VStack(spacing: 8) {
                if let scanError {
                    Label(scanError, systemImage: "exclamationmark.triangle.fill")
                        .font(.callout)
                        .foregroundStyle(.white)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)
                        .background(.black.opacity(0.75), in: RoundedRectangle(cornerRadius: 8))
                }

                Text("Point the camera at a Hi Agent pairing QR code.")
                    .font(.callout)
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .background(.black.opacity(0.75), in: RoundedRectangle(cornerRadius: 8))
            }
            .padding()
        }
    }

    private func prepareCamera() async {
        guard DataScannerViewController.isSupported else {
            cameraState = .unavailable
            return
        }

        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            cameraState = DataScannerViewController.isAvailable ? .ready : .unavailable
        case .notDetermined:
            let granted = await AVCaptureDevice.requestAccess(for: .video)
            cameraState = granted && DataScannerViewController.isAvailable ? .ready : .denied
        case .denied, .restricted:
            cameraState = .denied
        @unknown default:
            cameraState = .unavailable
        }
    }

    private func handle(_ payload: String) {
        do {
            guard let url = URL(string: payload) else {
                throw PairingRequestError.invalidLink
            }
            let request = try PairingRequest(url: url)
            onScan(request)
            dismiss()
        } catch {
            scanError = error.localizedDescription
        }
    }

    private enum CameraState {
        case checking
        case ready
        case denied
        case unavailable
    }
}

private struct PairingQRScannerController: UIViewControllerRepresentable {
    let onPayload: (String) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onPayload: onPayload)
    }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let controller = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false,
            isPinchToZoomEnabled: true,
            isGuidanceEnabled: true,
            isHighlightingEnabled: true
        )
        controller.delegate = context.coordinator
        try? controller.startScanning()
        return controller
    }

    func updateUIViewController(_ controller: DataScannerViewController, context: Context) {
        context.coordinator.onPayload = onPayload
        if !controller.isScanning {
            try? controller.startScanning()
        }
    }

    static func dismantleUIViewController(
        _ controller: DataScannerViewController,
        coordinator: Coordinator
    ) {
        controller.stopScanning()
    }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        var onPayload: (String) -> Void
        private var lastPayload: String?

        init(onPayload: @escaping (String) -> Void) {
            self.onPayload = onPayload
        }

        func dataScanner(
            _ dataScanner: DataScannerViewController,
            didAdd addedItems: [RecognizedItem],
            allItems: [RecognizedItem]
        ) {
            for item in addedItems {
                guard case .barcode(let barcode) = item,
                      let payload = barcode.payloadStringValue,
                      payload != lastPayload
                else {
                    continue
                }
                lastPayload = payload
                onPayload(payload)
                return
            }
        }
    }
}
