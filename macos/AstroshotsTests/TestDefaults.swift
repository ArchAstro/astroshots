import Foundation

/// Throwaway `UserDefaults` for tests, backed purely by memory.
///
/// Use this anywhere a test needs a `UserDefaults` to hand to
/// `Preferences(defaults:)`. There is nothing to clean up: no `defer`, no
/// suite name, no cleanup call. The storage dies with the instance.
///
/// ```swift
/// let defaults = TestDefaults()
/// let preferences = Preferences(defaults: defaults)
/// ```
///
/// # Why this is in-memory and not a real preference suite
///
/// **Do not "simplify" this back to `UserDefaults(suiteName:)`. That is the bug
/// this type exists to fix, and the failure is invisible for the first few
/// seconds.**
///
/// `Preferences.init` calls `migrateFirstRunStateIfNeeded()`, which
/// unconditionally writes `firstRunMigrationVersion`. So merely *constructing*
/// `Preferences(defaults:)` dirties whatever domain it is given. When that
/// domain was a real suite, every test run left a
/// `~/Library/Preferences/astroshots-<uuid>.plist` behind — 631 had piled up
/// before this type existed, and the count was still climbing daily.
///
/// The intuitive fix does not work. `cfprefsd`, not the test process, owns a
/// registered suite; it re-flushes its cached copy to disk on its own schedule,
/// *including after the client process exits*, recreating the file no matter
/// what the client deleted first. Measured, 50 suites across 10 short-lived
/// processes:
///
/// | cleanup attempt | right after run | 3s later |
/// | --- | --- | --- |
/// | `removePersistentDomain` (what the tests used to do) | 50 leaked | 50 leaked |
/// | `removePersistentDomain` + `removeSuite(named:)` + `CFPreferencesAppSynchronize` + `unlink` | **0 leaked** | **50 leaked** |
/// | the above + `CFPreferencesSetMultiple` key-nuking across Any/Current × User/Host + `unlink` retry loop | 0 leaked | leaked |
///
/// The middle row is the trap: it passes an assertion made right after the run
/// and fails seconds later, so it would have shipped a green regression guard
/// over a live leak. No in-process cleanup sequence can win that race.
///
/// This type therefore never registers a suite at all. Nothing reaches
/// `cfprefsd`, nothing reaches disk, and the leak is structurally impossible
/// rather than cleaned up after the fact — a strictly stronger guarantee than
/// "remove the plist afterwards."
///
/// Production behaviour is deliberately untouched: the real app still uses
/// `UserDefaults.standard` and still performs the init-time migration write.
/// The tests isolate that write; the app keeps doing it.
///
/// # Verified properties
///
/// Asserted by `PreferenceLeakGuardTests`, not merely assumed:
///
/// 1. Every typed accessor (`bool`/`integer`/`double`/`float`/`string`/
///    `stringArray`/`url`) funnels through the overridden `object(forKey:)`, and
///    every typed setter — including the `Bool`, `Int`, `Double`, `Float`, and
///    `URL` overloads and the KVC `setValue(_:forKey:)` path — funnels through
///    the overridden `set(_:forKey:)`.
/// 2. No write escapes to the standard domain.
/// 3. Reads are truly isolated, not shadowed: a value present in the standard
///    domain is invisible through this instance.
/// 4. Separate instances share no state, so each test starts genuinely empty.
final class TestDefaults: UserDefaults {
    /// `UserDefaults` is documented as thread-safe, and tests reach these
    /// accessors from `@MainActor` tests and watcher callbacks alike, so the
    /// backing store is lock-guarded to match.
    private let lock = NSLock()
    private var storage: [String: Any] = [:]

    /// Creates an empty, memory-only defaults domain.
    ///
    /// `suiteName: nil` is what keeps this out of `cfprefsd`: no suite is
    /// registered, so no domain is ever scheduled for a disk flush. Every
    /// accessor below is overridden to consult `storage` instead.
    init() {
        super.init(suiteName: nil)!
    }

    // MARK: - Primitive storage
    //
    // The whole class funnels through this trio, so overriding it redirects
    // every typed accessor onto in-memory storage.

    override func object(forKey defaultName: String) -> Any? {
        lock.withLock { storage[defaultName] }
    }

    override func set(_ value: Any?, forKey defaultName: String) {
        lock.withLock {
            guard let value else {
                storage.removeValue(forKey: defaultName)
                return
            }
            storage[defaultName] = value
        }
    }

    override func removeObject(forKey defaultName: String) {
        lock.withLock { storage.removeValue(forKey: defaultName) }
    }

    // MARK: - Domain-level APIs
    //
    // No test needs these today, but leaving them inherited would let a future
    // test reach the real on-disk domain through this instance and quietly
    // reintroduce the leak. Each is redirected to the in-memory store.

    override func dictionaryRepresentation() -> [String: Any] {
        lock.withLock { storage }
    }

    override func persistentDomain(forName domainName: String) -> [String: Any]? {
        dictionaryRepresentation()
    }

    override func setPersistentDomain(_ domain: [String: Any], forName domainName: String) {
        lock.withLock { storage = domain }
    }

    override func removePersistentDomain(forName domainName: String) {
        lock.withLock { storage.removeAll() }
    }

    /// No-ops: composing another suite into this one would pull real, on-disk
    /// preferences into a supposedly isolated domain.
    override func addSuite(named suiteName: String) {}

    override func removeSuite(named suiteName: String) {}

    /// Always succeeds; there is no backing file to flush.
    override func synchronize() -> Bool { true }
}
