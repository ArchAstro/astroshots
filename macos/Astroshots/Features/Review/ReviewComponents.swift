import SwiftUI

struct ReviewBadge: View {
    let state: ReviewState

    private var presentation: (title: String, symbol: String, foreground: Color, background: Color) {
        switch state {
        case .pending:
            return ("Unseen", "circle.dashed", Theme.amber, Theme.amberSoft)
        case .seen:
            return ("Seen", "checkmark.circle.fill", Theme.blue, Theme.blue.opacity(0.1))
        }
    }

    private var stateIdentifier: String {
        switch state {
        case .pending: "pending"
        case .seen: "seen"
        }
    }

    var body: some View {
        let style = presentation
        Label(style.title, systemImage: style.symbol)
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(style.foreground)
            .padding(.horizontal, 9)
            .padding(.vertical, 5)
            .background(style.background, in: Capsule())
            .accessibilityIdentifier("review.status.\(stateIdentifier)")
    }
}

struct ReviewCommentView: View {
    let comment: ReviewComment

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 7) {
                Image(systemName: "person.crop.circle.fill")
                    .font(.system(size: 16))
                    .foregroundStyle(Theme.purple.opacity(0.82))

                Text("Reviewer")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Theme.ink)

                Spacer(minLength: 8)

                Text(displayDate)
                    .font(.system(size: 9.5))
                    .foregroundStyle(Theme.muted2)
            }

            Text(comment.body)
                .font(.system(size: 12))
                .foregroundStyle(Theme.ink2)
                .lineSpacing(2)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 11, style: .continuous)
                .fill(Color.white.opacity(0.78))
                .overlay(
                    RoundedRectangle(cornerRadius: 11, style: .continuous)
                        .stroke(Theme.line, lineWidth: 1)
                )
        )
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("review.comment.\(comment.id)")
    }

    private var displayDate: String {
        if let date = parseISO8601(comment.createdAt) {
            return date.formatted(date: .abbreviated, time: .shortened)
        }
        return comment.createdAt
    }

    private func parseISO8601(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) {
            return date
        }

        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: value)
    }
}

struct ReviewActionButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled

    enum Tone {
        case quiet
        case primary
    }

    let tone: Tone

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(foreground)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 9)
            .background(background(configuration: configuration), in: RoundedRectangle(cornerRadius: 9))
            .overlay(
                RoundedRectangle(cornerRadius: 9)
                    .stroke(border, lineWidth: tone == .quiet ? 1 : 0)
            )
            .scaleEffect(configuration.isPressed ? 0.985 : 1)
            .saturation(isEnabled ? 1 : 0.2)
            .opacity(isEnabled ? (configuration.isPressed ? 0.86 : 1) : 0.42)
            .animation(.easeOut(duration: 0.1), value: configuration.isPressed)
            .animation(.easeOut(duration: 0.12), value: isEnabled)
    }

    private var foreground: Color {
        switch tone {
        case .quiet: Theme.ink2
        case .primary: .white
        }
    }

    private func background(configuration: Configuration) -> Color {
        switch tone {
        case .quiet:
            return configuration.isPressed ? Theme.surface : Color.white.opacity(0.7)
        case .primary:
            return Theme.blue
        }
    }

    private var border: Color {
        tone == .quiet ? Theme.lineStrong : .clear
    }
}

#Preview("Review badges") {
    HStack(spacing: 10) {
        ReviewBadge(state: .pending)
        ReviewBadge(state: .seen)
    }
    .padding(20)
    .background(Theme.paper)
}
