import AppKit
import SwiftUI
import Testing
@testable import Astroshots

struct ThemeTests {
    @Test @MainActor
    func trayPaletteAdaptsToAquaAndDarkAquaWithReadableContrast() throws {
        let lightPaper = try resolved(Theme.paper, appearance: .aqua)
        let lightInk = try resolved(Theme.ink, appearance: .aqua)
        let darkPaper = try resolved(Theme.paper, appearance: .darkAqua)
        let darkInk = try resolved(Theme.ink, appearance: .darkAqua)

        #expect(luminance(lightPaper) > luminance(darkPaper))
        #expect(contrast(lightPaper, lightInk) >= 7)
        #expect(contrast(darkPaper, darkInk) >= 7)
    }

    @Test @MainActor
    func elevatedCardsRemainDistinctInBothAppearances() throws {
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            let paper = try resolved(Theme.paper, appearance: appearance)
            let elevated = try resolved(Theme.elevated, appearance: appearance)
            #expect(abs(luminance(paper) - luminance(elevated)) >= 0.015)
        }
    }

    @MainActor
    private func resolved(_ color: Color, appearance name: NSAppearance.Name) throws -> NSColor {
        let appearance = try #require(NSAppearance(named: name))
        var resolved: NSColor?
        appearance.performAsCurrentDrawingAppearance {
            resolved = NSColor(color).usingColorSpace(.sRGB)
        }
        return try #require(resolved)
    }

    private func luminance(_ color: NSColor) -> CGFloat {
        func channel(_ value: CGFloat) -> CGFloat {
            value <= 0.04045
                ? value / 12.92
                : pow((value + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel(color.redComponent)
            + 0.7152 * channel(color.greenComponent)
            + 0.0722 * channel(color.blueComponent)
    }

    private func contrast(_ first: NSColor, _ second: NSColor) -> CGFloat {
        let lighter = max(luminance(first), luminance(second))
        let darker = min(luminance(first), luminance(second))
        return (lighter + 0.05) / (darker + 0.05)
    }
}
