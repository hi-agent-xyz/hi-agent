import SwiftUI
import UIKit

/// How to put "show my screen" on a button.
///
/// This screen exists because the setup is four steps in two other apps and none of
/// them can be done for you. iOS has no API to install a shortcut, and no way for an
/// app to ask for the Action Button — so the honest thing is to say plainly what to
/// do and open the app where it starts, rather than to imply the app arranged
/// anything.
///
/// **Follow-up worth having:** a signed `.shortcut` file served from
/// `hi.xiaoyuanzhu.com` turns steps 1–3 into one tap on an import sheet. It needs a
/// Mac to sign the file and a URL to serve it from, so it is not in this change.
struct ShowScreenSetupView: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Text(
                        """
                        Press a button inside any app and Hi Agent gets a picture of \
                        what you were looking at, then opens on the conversation.
                        """
                    )
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                    Card {
                        VStack(alignment: .leading, spacing: 16) {
                            Step(
                                number: 1,
                                title: "Make a shortcut",
                                detail: "In Shortcuts, tap + to start a new one."
                            )
                            Step(
                                number: 2,
                                title: "Add Take Screenshot",
                                detail: "It has to come first — otherwise the picture is of Hi Agent."
                            )
                            Step(
                                number: 3,
                                title: "Add Show My Screen",
                                detail: "Under Hi Agent. Leave it set to the screenshot from step 2."
                            )
                            Step(
                                number: 4,
                                title: "Give it the Action Button",
                                detail: "Settings → Action Button → Shortcut, then pick it."
                            )
                        }
                    }

                    Text(
                        """
                        No Action Button on this iPhone? The same shortcut works from \
                        Back Tap (Settings → Accessibility → Touch), from a button on \
                        the Lock Screen, or from Control Centre.
                        """
                    )
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                    Text(
                        """
                        The first time it runs, iOS asks whether the shortcut may take \
                        a screenshot. It goes to the core you are attached to, and \
                        nowhere else.
                        """
                    )
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal, Theme.gutter)
                .padding(.top, 8)
                .padding(.bottom, 24)
            }
            .hiCanvas()
            .navigationTitle("Show your screen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .font(.body.weight(.semibold))
                }
            }
            .safeAreaInset(edge: .bottom) {
                Button {
                    // Shortcuts' own scheme. Absent only if the app was deleted, in
                    // which case there is nothing useful to open anyway.
                    if let url = URL(string: "shortcuts://") {
                        UIApplication.shared.open(url)
                    }
                } label: {
                    Label("Open Shortcuts", systemImage: "arrow.up.forward.app")
                        .font(.body.weight(.semibold))
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .buttonBorderShape(.roundedRectangle(radius: 14))
                .padding(.horizontal, 16)
                .padding(.top, 10)
                .padding(.bottom, 8)
                .background(.bar)
            }
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
    }
}

private struct Step: View {
    let number: Int
    let title: String
    let detail: String

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Text("\(number)")
                .font(Theme.display(14, .bold))
                .foregroundStyle(Theme.ink)
                .frame(width: 26, height: 26)
                .background(Circle().fill(Theme.ink.opacity(0.12)))

            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                Text(detail)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
    }
}
