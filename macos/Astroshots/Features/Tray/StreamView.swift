import SwiftUI

struct StreamView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        if appState.isEmpty {
            EmptyStreamView()
        } else {
            ScrollView {
                LazyVStack(spacing: 1) {
                    ForEach(appState.shots) { shot in
                        ShotRow(shot: shot) {
                            appState.selectShot(shot)
                        }
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
            }
            .scrollIndicators(.hidden)
        }
    }
}

struct ShotRow: View {
    let shot: Shot
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .center, spacing: 10) {
                ShotThumbnail(path: shot.path)
                    .frame(width: 88, height: 56)
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 8, style: .continuous)
                            .stroke(Theme.line, lineWidth: 1)
                    )

                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        WorktreeChip(label: shot.worktreeShort)
                        if shot.isFailure {
                            Circle()
                                .fill(Theme.red)
                                .frame(width: 6, height: 6)
                        }
                        Text("\(shot.feature) · \(shot.title)")
                            .font(.system(size: 12, weight: .bold))
                            .foregroundStyle(Theme.ink)
                            .lineLimit(1)
                        Spacer(minLength: 4)
                        Text(Self.timeString(shot.capturedAt))
                            .font(.system(size: 9, weight: .medium, design: .monospaced))
                            .foregroundStyle(Theme.muted2)
                    }
                    Text(shot.description.isEmpty ? shot.fileName : shot.description)
                        .font(.system(size: 10))
                        .foregroundStyle(Theme.muted)
                        .lineLimit(1)
                }
            }
            .padding(8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(
            RoundedRectangle(cornerRadius: 11, style: .continuous)
                .fill(Color.clear)
        )
        #if os(macOS)
        .onHover { hovering in
            // Visual handled by plain button; could add highlight state later.
            _ = hovering
        }
        #endif
    }

    private static let timeFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm:ss"
        return f
    }()

    static func timeString(_ date: Date) -> String {
        timeFormatter.string(from: date)
    }
}

struct EmptyStreamView: View {
    @Environment(AppState.self) private var appState

    var body: some View {
        VStack(spacing: 0) {
            Spacer()
            Image(systemName: "camera")
                .font(.system(size: 18, weight: .medium))
                .foregroundStyle(Theme.muted)
                .frame(width: 40, height: 40)
                .background(Theme.surface, in: RoundedRectangle(cornerRadius: 12))
                .padding(.bottom, 12)
            Text(appState.isScanning ? "Scanning watch root…" : "Waiting for frames")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Theme.ink)
                .padding(.bottom, 5)
            Text(
                appState.isScanning
                    ? "First scan of a large folder can take a bit. The menu bar icon stays available."
                    : "When any project under the watch root writes to .astroshot/, shots land here and on the desktop."
            )
                .font(.system(size: 11))
                .foregroundStyle(Theme.muted)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 240)
                .padding(.bottom, 14)
            Button("Rescan") {
                appState.rescan()
            }
            .buttonStyle(.plain)
            .font(.system(size: 11, weight: .semibold))
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(Theme.lineStrong, lineWidth: 1)
                    .background(Color.white, in: RoundedRectangle(cornerRadius: 8))
            )
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(.horizontal, 28)
    }
}
