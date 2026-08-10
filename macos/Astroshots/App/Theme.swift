import SwiftUI

/// Design tokens from docs/mocks/astroshots-menubar.html — warm paper, sibling to Rooms.
enum Theme {
    static let ink = Color(hex: 0x171716)
    static let ink2 = Color(hex: 0x3F3E3A)
    static let muted = Color(hex: 0x77736D)
    static let muted2 = Color(hex: 0x6F6B65)

    static let paper = Color(hex: 0xFAF9F6)
    static let surface = Color(hex: 0xF1EFEA)
    static let line = Color(hex: 0x312E29).opacity(0.12)
    static let lineStrong = Color(hex: 0x312E29).opacity(0.18)

    static let purple = Color(hex: 0x6257D9)
    static let purpleSoft = Color(hex: 0xEFEDFF)
    static let green = Color(hex: 0x148266)
    static let greenSoft = Color(hex: 0xE7F5F0)
    static let amber = Color(hex: 0xA96414)
    static let amberSoft = Color(hex: 0xFBF0DC)
    static let red = Color(hex: 0xB54848)
    static let redSoft = Color(hex: 0xF9E8E7)
    static let blue = Color(hex: 0x376E9C)
    static let blueSoft = Color(hex: 0xE7F0F8)

    static let trayWidth: CGFloat = 430
    static let trayHeight: CGFloat = 640
}

extension Color {
    init(hex: UInt32) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}

/// Flat hover feedback for tray chrome controls, shared with Agent Rooms.
private struct HoverHighlight: ViewModifier {
    let cornerRadius: CGFloat
    @State private var isHovered = false

    func body(content: Content) -> some View {
        content
            .background(
                isHovered ? Color(hex: 0x21201C).opacity(0.06) : .clear,
                in: RoundedRectangle(cornerRadius: cornerRadius)
            )
            .onHover { isHovered = $0 }
    }
}

extension View {
    func hoverHighlight(cornerRadius: CGFloat = 7) -> some View {
        modifier(HoverHighlight(cornerRadius: cornerRadius))
    }
}

struct WorktreeChip: View {
    let label: String

    private var colors: (Color, Color) {
        let lower = label.lowercased()
        if lower.contains("wt4") || lower.hasSuffix("4") {
            return (Theme.green, Theme.greenSoft)
        }
        if lower.contains("wt1") || lower.hasSuffix("1") {
            return (Theme.purple, Theme.purpleSoft)
        }
        if lower.contains("wt5") || lower.hasSuffix("5") {
            return (Theme.blue, Theme.blueSoft)
        }
        if lower.contains("wt2") || lower.hasSuffix("2") {
            return (Theme.amber, Theme.amberSoft)
        }
        return (Theme.muted, Theme.surface)
    }

    var body: some View {
        let (fg, bg) = colors
        Text(label)
            .font(.system(size: 9, weight: .bold, design: .monospaced))
            .foregroundStyle(fg)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(bg, in: Capsule())
    }
}
