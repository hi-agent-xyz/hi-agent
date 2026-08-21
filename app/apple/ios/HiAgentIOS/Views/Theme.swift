import SwiftUI
import UIKit

/// One place for the app's visual language, so a screen never invents its own
/// spacing scale, corner radius, or status colour.
///
/// The face a core renders is a web page we do not control. Everything the
/// native shell draws around it is deliberately quiet: matte surfaces, one
/// accent, no gloss — the chrome should recede until the moment it is needed.
enum Theme {
    // MARK: Colour

    /// The brand ink. Deep petrol in light appearance, lifted in dark so it
    /// still reads as an interactive colour against a near-black background.
    static let ink = Color.accentColor

    /// A very low-energy wash used behind native screens. Two stops only, both
    /// close to the system background — this is a surface, not a decoration.
    static var canvas: some ShapeStyle {
        LinearGradient(
            colors: [
                Color(uiColor: .systemBackground),
                Color(uiColor: UIColor { traits in
                    traits.userInterfaceStyle == .dark
                        ? UIColor(red: 0.063, green: 0.078, blue: 0.098, alpha: 1)
                        : UIColor(red: 0.937, green: 0.949, blue: 0.965, alpha: 1)
                })
            ],
            startPoint: .top,
            endPoint: .bottom
        )
    }

    /// Hairline used for card edges. Barely there in light, slightly lifted in
    /// dark where a shadow cannot do the separating.
    static var hairline: Color {
        Color(uiColor: UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(white: 1, alpha: 0.10)
                : UIColor(white: 0, alpha: 0.06)
        })
    }

    // MARK: Metrics

    static let cardRadius: CGFloat = 20
    static let tileRadius: CGFloat = 16
    static let gutter: CGFloat = 24

    // MARK: Type

    /// Rounded for anything that carries the product's voice — it is the
    /// closest system face to the Nunito mark the brand already uses.
    static func display(_ size: CGFloat, _ weight: Font.Weight = .bold) -> Font {
        .system(size: size, weight: weight, design: .rounded)
    }
}

extension HealthState {
    /// The dot colour. Only "here" earns the accent; everything else stays
    /// grey or amber so a roster of healthy cores is calm, not a traffic light.
    var tint: Color {
        switch self {
        case .here:
            return .green
        case .asleep:
            return .indigo
        case .unreachable:
            return .orange
        case .checking, .unknown:
            return .secondary
        }
    }

    var isLive: Bool {
        self == .here
    }
}

/// The status dot next to a core's name. It breathes only while the core is
/// answering; a still dot means the app is not claiming anything it hasn't
/// just checked.
struct StatusDot: View {
    let state: HealthState
    var diameter: CGFloat = 9

    @State private var breathing = false

    var body: some View {
        ZStack {
            if state.isLive {
                Circle()
                    .fill(state.tint.opacity(0.28))
                    .frame(width: diameter * 2.1, height: diameter * 2.1)
                    .scaleEffect(breathing ? 1 : 0.55)
                    .opacity(breathing ? 0 : 1)
            }
            Circle()
                .fill(state.tint)
                .frame(width: diameter, height: diameter)
        }
        .frame(width: diameter * 2.1, height: diameter * 2.1)
        .onAppear {
            guard state.isLive else { return }
            withAnimation(.easeOut(duration: 2.2).repeatForever(autoreverses: false)) {
                breathing = true
            }
        }
        .onChange(of: state) { _, newState in
            breathing = false
            guard newState.isLive else { return }
            withAnimation(.easeOut(duration: 2.2).repeatForever(autoreverses: false)) {
                breathing = true
            }
        }
        .accessibilityHidden(true)
    }
}

/// A matte card. Used for grouped content instead of `Form`, which drags its
/// own inset-grouped background in and fights the canvas.
struct Card<Content: View>: View {
    var padding: CGFloat = 18
    @ViewBuilder var content: Content

    var body: some View {
        content
            .padding(padding)
            .background(
                RoundedRectangle(cornerRadius: Theme.cardRadius, style: .continuous)
                    .fill(Color(uiColor: .secondarySystemGroupedBackground))
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.cardRadius, style: .continuous)
                    .strokeBorder(Theme.hairline, lineWidth: 1)
            )
    }
}

/// The product mark: the same three-node glyph the empty state used, but sized
/// and tinted as a mark rather than as a placeholder icon.
struct CoreMark: View {
    var size: CGFloat = 88

    var body: some View {
        Image(systemName: "point.3.connected.trianglepath.dotted")
            .font(.system(size: size * 0.46, weight: .medium))
            .foregroundStyle(Theme.ink)
            .frame(width: size, height: size)
            .background(
                RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
                    .fill(Theme.ink.opacity(0.10))
            )
            .accessibilityHidden(true)
    }
}

extension View {
    /// Full-bleed app canvas. Applied once per native screen.
    func hiCanvas() -> some View {
        background(Theme.canvas, ignoresSafeAreaEdges: .all)
    }
}

enum Haptic {
    static func success() {
        UINotificationFeedbackGenerator().notificationOccurred(.success)
    }

    static func failure() {
        UINotificationFeedbackGenerator().notificationOccurred(.error)
    }

    static func tap() {
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
    }
}
