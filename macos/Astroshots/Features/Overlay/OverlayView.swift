import SwiftUI

struct OverlayView: View {
    let shot: Shot
    let onOpen: () -> Void

    var body: some View {
        Button(action: onOpen) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    ZStack {
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .fill(
                                LinearGradient(
                                    colors: [Theme.blue, Theme.purple],
                                    startPoint: .topLeading,
                                    endPoint: .bottomTrailing
                                )
                            )
                        Image(systemName: "sparkles.rectangle.stack.fill")
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(.white)
                    }
                    .frame(width: 30, height: 30)

                    VStack(alignment: .leading, spacing: 2) {
                        Text(titleLine)
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(Theme.ink)
                            .lineLimit(1)
                        Text("\(shot.worktreeShort) · \(shot.feature)")
                            .font(.system(size: 10.5, weight: .medium))
                            .foregroundStyle(Theme.muted)
                            .lineLimit(1)
                    }

                    Spacer(minLength: 8)

                    Text("Open in Astroshots")
                        .font(.system(size: 10.5, weight: .semibold))
                        .foregroundStyle(Theme.blue)

                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(Theme.blue.opacity(0.8))
                }
                .padding(12)

                ShotThumbnail(path: shot.path, contentMode: .fit)
                    .frame(maxWidth: .infinity)
                    .frame(height: 210)
                    .clipped()
                    .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .stroke(Theme.line, lineWidth: 1)
                    )
                    .padding(.horizontal, 10)

                Text(shot.description.isEmpty ? shot.title : shot.description)
                    .font(.system(size: 10.5))
                    .foregroundStyle(Theme.muted)
                    .lineLimit(1)
                    .padding(.horizontal, 12)
                    .padding(.top, 9)
                    .padding(.bottom, 11)
            }
            .frame(width: 396, height: 316, alignment: .top)
            .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        }
        .buttonStyle(.plain)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(Theme.paper.opacity(0.97))
                .overlay(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .stroke(Color.white.opacity(0.72), lineWidth: 0.75)
                )
                .shadow(color: .black.opacity(0.2), radius: 24, y: 10)
        )
        .padding(2)
        .help("Open \(shot.title) in Astroshots")
        .accessibilityLabel("Open \(shot.title) in Astroshots")
        .accessibilityIdentifier("overlay.open.\(shot.fileName)")
    }

    private var titleLine: String {
        if let sequence = shot.sequence {
            return "\(sequence) · \(shot.title)"
        }
        return shot.title
    }
}
