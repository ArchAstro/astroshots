import AppKit
import SwiftUI

/// Design tokens from docs/mocks/astroshots-menubar.html — warm paper, sibling to Rooms.
enum Theme {
    static let ink = adaptive(light: 0x171716, dark: 0xF5F3EE)
    static let ink2 = adaptive(light: 0x3F3E3A, dark: 0xD6D2CA)
    static let muted = adaptive(light: 0x77736D, dark: 0xA9A49C)
    static let muted2 = adaptive(light: 0x6F6B65, dark: 0x918C85)

    static let paper = adaptive(light: 0xFAF9F6, dark: 0x191A1D)
    static let surface = adaptive(light: 0xF1EFEA, dark: 0x232428)
    static let elevated = adaptive(light: 0xFFFFFF, dark: 0x2B2C30)
    static let line = adaptive(light: 0x312E29, dark: 0xF5F3EE).opacity(0.12)
    static let lineStrong = adaptive(light: 0x312E29, dark: 0xF5F3EE).opacity(0.18)

    static let purple = adaptive(light: 0x6257D9, dark: 0x9B92FF)
    static let purpleSoft = adaptive(light: 0xEFEDFF, dark: 0x332F58)
    static let green = adaptive(light: 0x148266, dark: 0x4BC4A2)
    static let greenSoft = adaptive(light: 0xE7F5F0, dark: 0x183D34)
    static let amber = adaptive(light: 0xA96414, dark: 0xE2A85D)
    static let amberSoft = adaptive(light: 0xFBF0DC, dark: 0x49351E)
    static let red = adaptive(light: 0xB54848, dark: 0xE47C78)
    static let redSoft = adaptive(light: 0xF9E8E7, dark: 0x4B2728)
    static let blue = adaptive(light: 0x376E9C, dark: 0x73AFDB)
    static let blueSoft = adaptive(light: 0xE7F0F8, dark: 0x20394B)

    static let trayWidth: CGFloat = 430
    static let trayHeight: CGFloat = 640

    private static func adaptive(light: UInt32, dark: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return NSColor(hex: isDark ? dark : light)
        })
    }
}

extension NSColor {
    convenience init(hex: UInt32) {
        let red = CGFloat((hex >> 16) & 0xFF) / 255
        let green = CGFloat((hex >> 8) & 0xFF) / 255
        let blue = CGFloat(hex & 0xFF) / 255
        self.init(
            calibratedRed: red,
            green: green,
            blue: blue,
            alpha: 1
        )
    }
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
                isHovered ? Theme.ink.opacity(0.06) : .clear,
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
