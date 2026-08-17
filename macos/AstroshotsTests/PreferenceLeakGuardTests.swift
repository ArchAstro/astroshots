import Foundation
import Testing
@testable import Astroshots

/// Regression guard for the preference-plist leak, plus the assertions backing
/// `TestDefaults`' documented isolation claims.
///
/// # Defeating the timing trap
///
/// Sampling `~/Library/Preferences` right after a test run is **not** a valid
/// guard. `cfprefsd` re-flushes a registered suite to disk on its own schedule,
/// after the writing process has exited, so a broken cleanup shows zero leaked
/// files immediately and 50 leaked files three seconds later. A guard built on
/// an immediate sample would go green over a live leak — that is exactly how
/// this bug survived 21 `removePersistentDomain` calls.
///
/// So the guard does not race `cfprefsd` at all. It asserts the strictly
/// stronger, timing-independent invariant: **no test ever registers a
/// persistent suite in the first place.** If nothing is registered, there is
/// nothing for `cfprefsd` to flush, whenever it decides to run.
///
/// That invariant is enforced at three layers:
///
/// 1. `testDefaultsCannotReachDisk` — proves the helper every test now uses
///    keeps writes in memory, including the `Preferences.init` migration write
///    that caused the leak. Timing-independent: it inspects behaviour, not the
///    clock.
/// 2. `noSuiteRegisteringTestSourceRemains` — proves no test source contains the
///    `UserDefaults(suiteName:)` construction that creates a registered suite.
///    Timing-independent: it catches a reintroduced leak at the source, even if
///    that test never runs.
/// 3. `macos/scripts/check-preference-leaks.sh` — the filesystem backstop, run
///    by CI *after* `xcodebuild test` with a settle delay so a late `cfprefsd`
///    flush cannot hide. Swift Testing runs tests in parallel, so no in-process
///    test can be scheduled last; only the script can speak for the whole run.
///
/// The filesystem check below is kept as a fast local signal for pre-existing
/// residue, but layers 1–3 are what make the guard sound.
struct PreferenceLeakGuardTests {
    private static let leakedPrefix = "astroshots-"

    private static var preferencesDirectory: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Preferences", isDirectory: true)
    }

    private static func leakedPlistNames() -> [String] {
        let names = try? FileManager.default.contentsOfDirectory(
            atPath: preferencesDirectory.path
        )
        return (names ?? [])
            .filter { $0.hasPrefix(leakedPrefix) && $0.hasSuffix(".plist") }
            .sorted()
    }

    // MARK: - Layer 1: the helper cannot reach disk

    /// The load-bearing assertion. `Preferences.init` performs the first-run
    /// migration write, which is the write that used to create a plist; through
    /// `TestDefaults` it must stay in memory.
    @Test @MainActor
    func testDefaultsCannotReachDisk() {
        let before = Set(Self.leakedPlistNames())

        let defaults = TestDefaults()
        let preferences = Preferences(defaults: defaults)
        preferences.watchRootPaths = ["/tmp/astroshots-leak-guard"]
        // Construct twice, as several tests do, to re-run the migration path.
        _ = Preferences(defaults: defaults)

        #expect(
            Set(Self.leakedPlistNames()) == before,
            "TestDefaults must never create a plist in the preferences directory"
        )
        // The migration write really happened — it just stayed in memory.
        #expect(defaults.integer(forKey: "firstRunMigrationVersion") == 1)
    }

    /// Writes through `TestDefaults` must not escape into the real standard
    /// domain, and real values in the standard domain must not leak in. Proves
    /// isolation is genuine rather than a shadowing layer over the live domain.
    @Test
    func testDefaultsIsIsolatedFromTheStandardDomain() {
        let key = "astroshotsLeakGuardProbe-\(UUID().uuidString)"
        let defaults = TestDefaults()

        defaults.set("in-memory", forKey: key)
        #expect(
            UserDefaults.standard.object(forKey: key) == nil,
            "a TestDefaults write escaped into UserDefaults.standard"
        )

        // Reads must not fall through to the real domain either.
        let standardOnlyKey = "astroshotsLeakGuardStandardOnly-\(UUID().uuidString)"
        UserDefaults.standard.set("real", forKey: standardOnlyKey)
        defer { UserDefaults.standard.removeObject(forKey: standardOnlyKey) }
        #expect(
            defaults.string(forKey: standardOnlyKey) == nil,
            "TestDefaults read fell through to UserDefaults.standard"
        )

        // Instances must not share state.
        #expect(TestDefaults().string(forKey: key) == nil)
    }

    /// Every typed setter must funnel through the overridden `set(_:forKey:)`,
    /// and every typed getter through the overridden `object(forKey:)`. If an
    /// overload bypassed them it would write to the real domain instead.
    /// Asserted rather than assumed, per the documented contract.
    @Test
    func testDefaultsRoutesEveryTypedAccessorThroughTheOverrides() {
        let defaults = TestDefaults()

        defaults.set(true, forKey: "bool")
        defaults.set(7, forKey: "int")
        defaults.set(1.25, forKey: "double")
        defaults.set(Float(2.5), forKey: "float")
        defaults.set(URL(fileURLWithPath: "/tmp/astroshots-url"), forKey: "url")
        defaults.set(["a", "b"], forKey: "stringArray")
        defaults.set("text", forKey: "string")
        defaults.setValue("kvc", forKey: "kvc")

        #expect(defaults.bool(forKey: "bool"))
        #expect(defaults.integer(forKey: "int") == 7)
        #expect(defaults.double(forKey: "double") == 1.25)
        #expect(defaults.float(forKey: "float") == 2.5)
        #expect(defaults.url(forKey: "url")?.path == "/tmp/astroshots-url")
        #expect(defaults.stringArray(forKey: "stringArray") == ["a", "b"])
        #expect(defaults.string(forKey: "string") == "text")
        #expect(defaults.string(forKey: "kvc") == "kvc")

        // Everything written must be visible in the in-memory domain, proving
        // no setter took a different path to storage.
        let representation = defaults.dictionaryRepresentation()
        for key in ["bool", "int", "double", "float", "url", "stringArray", "string", "kvc"] {
            #expect(representation[key] != nil, "\(key) bypassed set(_:forKey:)")
        }

        // Absent keys must report UserDefaults' documented zero values.
        #expect(defaults.bool(forKey: "missing") == false)
        #expect(defaults.integer(forKey: "missing") == 0)
        #expect(defaults.object(forKey: "missing") == nil)

        // Removal must clear in-memory state.
        defaults.removeObject(forKey: "string")
        #expect(defaults.string(forKey: "string") == nil)
        defaults.set(nil, forKey: "int")
        #expect(defaults.object(forKey: "int") == nil)
    }

    // MARK: - Layer 2: no test source registers a suite

    /// Fails if any test source reintroduces `UserDefaults(suiteName:)`, which
    /// registers a domain with `cfprefsd` and leaks a plist that cannot be
    /// reliably removed. This is the timing-independent guard: it fires on the
    /// source of the leak, regardless of when — or whether — that test runs.
    @Test
    func noSuiteRegisteringTestSourceRemains() throws {
        let thisFile = URL(fileURLWithPath: #filePath).lastPathComponent
        let testsDirectory = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        let sources = try FileManager.default.contentsOfDirectory(
            atPath: testsDirectory.path
        )
        .filter { $0.hasSuffix(".swift") }
        .sorted()

        // Guard the guard: if the layout changes so we scan nothing, say so
        // instead of passing vacuously.
        #expect(sources.count > 1, "expected to scan the test sources")

        var offenders: [String] = []
        for source in sources {
            // These two document the forbidden call in prose.
            guard source != thisFile, source != "TestDefaults.swift" else { continue }

            let contents = try String(
                contentsOf: testsDirectory.appendingPathComponent(source),
                encoding: .utf8
            )

            // Matched across newlines so a line-split call — `UserDefaults(`
            // with `suiteName:` on the following line — cannot evade the guard.
            let pattern = try NSRegularExpression(
                pattern: #"UserDefaults\s*\(\s*suiteName\s*:"#,
                options: .dotMatchesLineSeparators
            )
            let range = NSRange(contents.startIndex..<contents.endIndex, in: contents)
            for match in pattern.matches(in: contents, range: range) {
                guard let matched = Range(match.range, in: contents) else { continue }
                let line = contents[contents.startIndex..<matched.lowerBound]
                    .filter(\.isNewline)
                    .count + 1
                offenders.append("\(source):\(line)")
            }
        }

        #expect(
            offenders.isEmpty,
            """
            Test source registers a real UserDefaults suite: \
            \(offenders.joined(separator: ", ")).

            A registered suite is flushed to disk by cfprefsd — after the test \
            process exits — and cannot be reliably removed afterwards. Use \
            `TestDefaults()` instead; see TestDefaults.swift for the measurements.
            """
        )
    }

    // MARK: - Layer 3 companion: fast local residue check

    /// Fast signal for residue already on this machine, or from a leaking test
    /// that has already written its suite. Not authoritative for the run as a
    /// whole — parallel scheduling means a later test's leak may not be visible
    /// yet, which is why `scripts/check-preference-leaks.sh` exists.
    @Test
    func preferencesDirectoryHasNoAstroshotsPlists() {
        let leaked = Self.leakedPlistNames()

        #expect(
            leaked.isEmpty,
            """
            \(leaked.count) leaked preference plist(s) in \
            \(Self.preferencesDirectory.path).

            Clear the backlog with:
              rm -f ~/Library/Preferences/astroshots-*.plist

            If these keep coming back, a test is creating a real UserDefaults \
            suite instead of using TestDefaults().

            First 10: \(leaked.prefix(10).joined(separator: ", "))
            """
        )
    }
}
