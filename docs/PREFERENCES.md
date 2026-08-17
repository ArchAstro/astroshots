# Astroshots preferences: canonical domain and watch-root read contract

Read this before any script, CLI, or agent reads Astroshots' settings.
`scripts/verify-preferences-contract.mjs` (run by `npm test`) keeps every claim
below in sync with `macos/project.yml` and the repository, so this page cannot
silently drift.

## 1. Canonical preferences domain

```
ai.archastro.Astroshots
```

1. The domain is the app's `PRODUCT_BUNDLE_IDENTIFIER`, defined once in
   [`macos/project.yml`](../macos/project.yml) (`bundleIdPrefix: ai.archastro`
   plus `PRODUCT_BUNDLE_IDENTIFIER`). `macos/Astroshots.xcodeproj` is generated
   from it by `xcodegen`; never hand-edit the identifier there.
2. The prefix is **`ai.`**. The plausible-but-wrong guess swaps it for `com.`,
   and that mistake is *dangerous* rather than loud: `defaults read` on a
   nonexistent domain fails with `The domain/default pair does not exist`, so a
   consumer that swallows the error reports "not configured" for a fully
   configured machine. The guard in
   `scripts/verify-preferences-contract.mjs` fails the build if any
   wrong-prefix `*.archastro` domain string appears in a tracked file — which
   is also why this page never spells the wrong form out.
3. Verify on a real machine:

   ```bash
   defaults read ai.archastro.Astroshots            # all keys
   defaults read ai.archastro.Astroshots watchRoots # current watch roots
   ```

## 2. Watch-root read contract

Source of truth: `Preferences.watchRootPaths` in
[`macos/Astroshots/App/Preferences.swift`](../macos/Astroshots/App/Preferences.swift).

Read the keys in this order and stop at the first one that is **present**:

| # | Key | Type | Role |
|---|-----|------|------|
| 1 | `watchRoots` | array of strings | Current key. Multi-root. Wins whenever it exists, even if it is an empty array. |
| 2 | `watchRoot` | **string (singular)** | Legacy pre-multi-root key. Honored only when `watchRoots` is absent, so an upgrader keeps the folder they already chose. |

```
read watchRoots ──present?──▶ yes ──▶ normalize(array)         ──▶ roots
        │
        no
        ▼
read watchRoot ──non-empty string?──▶ yes ──▶ normalize([value]) ──▶ roots
        │
        no
        ▼
      [] (no roots configured — there is genuinely no default root)
```

Consequences for consumers:

1. Reading only `watchRoots` misreports a correctly configured upgrader as
   unconfigured. Both keys must be read, in the order above.
2. The app writes both keys together when roots change (`watchRoots` = the full
   normalized list, `watchRoot` = its first element), so seeing both populated
   is normal and **not** a conflict. On this repo's dev machine, for example,
   `watchRoot` is one path and `watchRoots` is that path plus another.
3. Values are **plain absolute POSIX paths**, not security-scoped bookmark
   data. They can be passed straight to file APIs; no bookmark resolution is
   involved. (The app runs unsandboxed so FSEvents can watch worktrees.)

## 3. Normalization

`Preferences.normalizeWatchRootPaths` runs on both read and write:

1. Drops empty strings.
2. Expands a leading `~`.
3. Standardizes the path and resolves symlinks (`/tmp` → `/private/tmp`).
4. Removes nesting and duplicates: a root already covered by an ancestor in the
   list is dropped, and an ancestor added later evicts its descendants.

Stored values are already normalized, but a consumer that compares paths should
resolve symlinks on its own input before matching, or it will miss on
`/tmp`-style aliases.

## 4. First-run state is a separate flag

1. `hasCompletedFirstRunSetup` (bool) is the **only** source of truth for
   whether first-run folder setup happened.
2. Empty watch roots do **not** mean "first run never happened", and configured
   watch roots do not by themselves mean it did. Do not infer one from the
   other.
3. `firstRunMigrationVersion` (int) is internal migration bookkeeping. Treat it
   as read-only.

## 5. Other keys

`overlayEnabled`, `autoDismiss`, `autoDismissSeconds`, `hiddenFrictionLogIDs`,
`narrationEnabled`, `narrationModelReady`, `narrationVoice`, and
`narrationCaptionsEnabled` are app-owned UI settings; see the `Key` enum in
`Preferences.swift`. Several default to a non-`false`/non-zero value when the
key is absent (for example `overlayEnabled` and `autoDismiss` default to true,
`autoDismissSeconds` to `5.5`), so an absent key is not the same as `0`/`false`.

## 6. Do not write

Tools should treat this domain as **read-only**. The app owns writes, including
keeping `watchRoot` in step with `watchRoots`; a partial external write can
strand the two keys in disagreement.
