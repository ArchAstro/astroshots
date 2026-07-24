import SwiftUI

struct OverlayView: View {
    let shot: Shot
    let onDismiss: () -> Void
    let onOpen: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("\(shot.worktreeShort) · \(shot.feature)")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(.white.opacity(0.55))
                        .textCase(.uppercase)
                        .tracking(0.3)
                    Text(titleLine)
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(Color(hex: 0xF4F1EA))
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                if let sequence = shot.sequence {
                    Text(sequence)
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .foregroundStyle(Color(hex: 0xD7D2C8))
                        .padding(.horizontal, 7)
                        .padding(.vertical, 2)
                        .background(Color.white.opacity(0.08), in: Capsule())
                }
                Button(action: onDismiss) {
                    Image(systemName: "xmark")
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(.white.opacity(0.55))
                        .frame(width: 24, height: 24)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .background(Color.clear, in: RoundedRectangle(cornerRadius: 7))
            }
            .padding(.horizontal, 12)
            .padding(.top, 10)
            .padding(.bottom, 8)

            ShotThumbnail(path: shot.path)
                .frame(maxWidth: .infinity)
                .aspectRatio(16 / 10, contentMode: .fit)
                .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .stroke(Color.white.opacity(0.1), lineWidth: 1)
                )
                .padding(.horizontal, 10)

            HStack(spacing: 8) {
                Text(shot.description.isEmpty ? shot.title : shot.description)
                    .font(.system(size: 11))
                    .foregroundStyle(Color(hex: 0xF4F1EA).opacity(0.72))
                    .lineLimit(1)
                Spacer(minLength: 8)
                Button("Open", action: onOpen)
                    .buttonStyle(.plain)
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(Theme.ink)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .background(Color.white, in: RoundedRectangle(cornerRadius: 7))
            }
            .padding(.horizontal, 12)
            .padding(.top, 9)
            .padding(.bottom, 11)
        }
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(Color(hex: 0x161513).opacity(0.92))
                .overlay(
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .stroke(Color.white.opacity(0.14), lineWidth: 1)
                )
                .shadow(color: .black.opacity(0.4), radius: 28, y: 12)
        )
        .padding(2)
    }

    private var titleLine: String {
        if let sequence = shot.sequence {
            return "\(sequence) · \(shot.title)"
        }
        return shot.title
    }
}
