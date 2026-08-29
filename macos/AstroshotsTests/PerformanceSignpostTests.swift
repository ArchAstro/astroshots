import Testing
@testable import Astroshots

struct PerformanceSignpostTests {
    @Test
    func intervalNamesMatchAcceptance() {
        #expect(
            PerformanceLog.intervalNames == [
                "tray.click_to_shown",
                "tray.pane_switch",
                "tray.image_decode",
                "review.open",
                "review.navigate",
            ]
        )
    }

    @Test
    func intervalHelperRunsWork() {
        var ran = false
        let value = PerformanceLog.interval(PerformanceLog.clickToShown) {
            ran = true
            return 42
        }
        #expect(ran)
        #expect(value == 42)
    }

    @Test
    func intervalHelperRethrows() {
        enum Sample: Error { case boom }
        #expect(throws: Sample.boom) {
            try PerformanceLog.interval(PerformanceLog.paneSwitch) {
                throw Sample.boom
            }
        }
    }
}
