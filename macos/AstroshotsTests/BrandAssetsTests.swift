import AppKit
import Testing

struct BrandAssetsTests {
    @Test @MainActor
    func menuBarMarkLoadsAsAResolutionIndependentTemplate() throws {
        let mark = try #require(NSImage(named: "AstroshotsMark"))

        #expect(mark.isTemplate)
        #expect(mark.size.width > 0)
        #expect(mark.size.height > 0)
    }
}
