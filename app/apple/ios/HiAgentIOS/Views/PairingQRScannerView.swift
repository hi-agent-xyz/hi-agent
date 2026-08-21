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
                    VStack(spacing: 14) {
                        ProgressView()
                        Text("Preparing camera…")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .hiCanvas()
                case .ready:
                    scanner
                case .denied:
                    StatusScreen(
                        symbol: "camera.fill",
                        title: "Camera access is off",
                        message: "Allow camera access in Settings, or type the pairing code instead.",
                        primary: .init(title: "Open Settings", action: {
                            guard let settingsURL = URL(string: UIApplication.openSettingsURLString)
                            else {
                                return
                            }
                            openURL(settingsURL)
                        }),
                        secondary: .init(title: "Type it instead", action: { dismiss() })
                    )
                case .unavailable:
                    StatusScreen(
                        symbol: "qrcode.viewfinder",
                        title: "Scanning isn't available",
                        message: "This device can't scan a code. Enter the core address and pairing code by hand.",
                        primary: .init(title: "Type it instead", action: { dismiss() })
                    )
                }
            }
            .navigationTitle("Scan pairing code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(cameraState == .ready ? .hidden : .automatic, for: .navigationBar)
            .toolbarColorScheme(cameraState == .ready ? .dark : nil, for: .navigationBar)
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

            ScannerReticle()
                .ignoresSafeArea()
                .allowsHitTesting(false)

            VStack(spacing: 10) {
                if let scanError {
                    Label(scanError, systemImage: "exclamationmark.triangle.fill")
                        .font(.footnote.weight(.medium))
                        .foregroundStyle(.white)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)
                        .background(
                            RoundedRectangle(cornerRadius: 14, style: .continuous)
                                .fill(Color.red.opacity(0.85))
                        )
                        .transition(.opacity.combined(with: .scale(scale: 0.96)))
                }

                Text("Point the camera at the pairing code your core is showing.")
                    .font(.subheadline)
                    .foregroundStyle(.white)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 18)
                    .padding(.vertical, 12)
                    .background(
                        RoundedRectangle(cornerRadius: 16, style: .continuous)
                            .fill(.ultraThinMaterial)
                            .environment(\.colorScheme, .dark)
                    )
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 44)
            .animation(.smooth(duration: 0.25), value: scanError)
        }
        .background(Color.black)
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

/// Dims everything except the square you are meant to aim with. Without it the
/// screen is a live camera feed with a caption — nothing says where to point.
private struct ScannerReticle: View {
    private let side: CGFloat = 250

    var body: some View {
        ZStack {
            Color.black.opacity(0.45)
                .mask {
                    Rectangle()
                        .overlay {
                            RoundedRectangle(cornerRadius: 28, style: .continuous)
                                .frame(width: side, height: side)
                                .blendMode(.destinationOut)
                        }
                        .compositingGroup()
                }

            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .strokeBorder(.white.opacity(0.85), lineWidth: 2)
                .frame(width: side, height: side)
        }
        .accessibilityHidden(true)
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
            // The reticle is the guidance; the system's own hint text on top of
            // it would be two sets of instructions in the same frame.
            isGuidanceEnabled: false,
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
