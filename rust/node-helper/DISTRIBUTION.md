# Embedded Node helper distribution

`astroshot-engine` builds a complete helper distribution from this directory's
`package.json` and `package-lock.json`. Cargo's build script runs `npm ci` in
`OUT_DIR`, with install scripts disabled and the target OS, CPU and libc selected
explicitly. Consumers building the Rust crate need Node >=22.14.0 and npm >=11
on PATH, plus registry access or an npm cache containing the locked packages.
Use npm's normal cache/offline configuration for offline builds. No root workspace
`node_modules` or TypeScript package build is needed to build the distribution.

The compiled engine contains a gzip tar archive. The archive includes the helper,
lockfile, manifest, target triple, dependencies and their license files. It excludes
npm executable shims and npm's generated install metadata. Entries are sorted and
have normalized timestamps/modes. SHA-256 of the archive identifies the dependency
contents, helper version and target together. Rust consumers need no sidecar files.
Updating this lockfile changes the distribution identity; old cache entries remain
usable by older binaries. Cache pruning is an explicit user operation.

A Darwin arm64 installation measured 58 MiB unpacked and approximately 16 MiB
compressed before the small helper/manifest additions. Sizes vary by target.
The pinned direct tools are tsx 4.23.1, Vite 6.4.3, React plugin 5.2.0 and
playwright-core 1.61.1. This lock resolves esbuild 0.28.2 for tsx, esbuild 0.25.12
for Vite, and Rollup 4.66.0. Both esbuild trees must retain their own native package.
The plugin requires Node >=22.12 on the supported Node 22 line; the engine enforces
the repository's stricter >=22.14.0 minimum. Node 24 is also supported.

| Rust target | npm OS / CPU | Rollup suffix |
| --- | --- | --- |
| aarch64-apple-darwin | darwin / arm64 | darwin-arm64 |
| x86_64-apple-darwin | darwin / x64 | darwin-x64 |
| aarch64-unknown-linux-gnu | linux / arm64 | linux-arm64-gnu |
| x86_64-unknown-linux-gnu | linux / x64 | linux-x64-gnu |
| x86_64-unknown-linux-musl | linux / x64 | linux-x64-musl |
| x86_64-pc-windows-msvc | win32 / x64 | win32-x64-msvc |
| aarch64-pc-windows-msvc | win32 / arm64 | win32-arm64-msvc |

esbuild uses the corresponding OS/CPU pair (no libc distinction). The build checks
that both esbuild native packages and the selected Rollup package were installed;
npm silently skipping optional dependencies cannot produce a successful bundle.
Cross-compilation installs target binaries without executing them. Node must match
that target's OS, architecture and (on Linux) libc. The helper reports an explicit
compatibility error before capture for mixed runtimes, including x64 Rust with
arm64 Node under Rosetta and musl Rust with glibc Node. Install matching Node or
provide a custom helper with matching dependencies for those configurations.

Measured compressed payload sizes (2026-10-29, built by the production build script):

| Target | Bytes |
| --- | ---: |
| aarch64-apple-darwin | 16119806 |
| x86_64-apple-darwin | 16917767 |
| aarch64-unknown-linux-gnu | 15827685 |
| x86_64-unknown-linux-gnu | 17418382 |
| x86_64-unknown-linux-musl | 16795292 |
| x86_64-pc-windows-msvc | 17935850 |
| aarch64-pc-windows-msvc | 15906509 |

Two consecutive Darwin arm64 builds produced identical archive bytes. This is a
local reproducibility check; native execution was verified on Darwin arm64 only.

## Runtime and compatibility

`node_helper_distribution::materialize(cache)` and the compatibility entry point
`node_helper::write_embedded_helper(cache)` return the complete helper path.
`NodeHelper::check_dependencies()` imports the four helper-owned tools using the
helper's own resolver; callers do not need to duplicate Node resolution.
Chrome and ffmpeg remain separate prerequisites and are never bundled here.
React, React DOM, Ink and fixture imports resolve from each user's project.

Default helper lookup retains explicit `ASTROSHOT_NODE_HELPER` and adjacent sidecar
locations, then uses the embedded distribution. A missing explicit helper retains
the existing fallback behavior. Source development must select its helper explicitly;
merely having a Cargo source checkout no longer changes runtime resolution. Custom
helpers retain their own dependency management and are never rewritten. Older custom
helpers may not support the new optional `check-dependencies` command.

The cache is `<cache>/astroshot/node-helper/<target>/<sha256>/`. Cold starts extract
into a unique staging directory and atomically publish the complete directory;
concurrent starts converge on the same verified result. Interrupted staging directories
are not selected. Warm starts verify all packaged file contents and executable bits.
Missing or altered files produce an error naming the affected file and cache directory.
Stop captures and remove that directory to repair it on the next run, entirely offline.
The engine does not delete or rewrite a corrupt directory that another process may use.

## Verification and limits

Canonical proof: `rust/astroshot-engine/tests/node_helper.rs` test
`packaged_helper_runs_project_fixtures_without_source_checkout` runs the production
Rust materializer and real Node helper, installs two isolated fixture projects from
committed locks, renders React 18/19 in real Chrome and Ink 5/7 in real Node, then
executes a Playwright script and checks the resulting page DOM. Fixture npm installation
is test setup; materialization and capture do not use npm or download dependencies.

Integration matrix: React, Ink and scripted browser commands cross the Rust/Node
process boundary; React/browser script also cross HTTP and Chrome CDP. The canonical
proof covers separate project dependency versions and cold/warm cache. The distribution
unit tests cover concurrent first publication, interrupted staging and altered dependency
files. Node tests cover missing executables and unsupported version boundaries.
Fidelity: all helper tools, engine code, npm fixture trees and Chrome run for real.
No production service or ffmpeg is involved.

Automatic path: `.github/workflows/ci.yml`, PRs to main and pushes to main/feat branches,
`rust` job on Ubuntu/macOS, `cargo test --workspace --no-fail-fast`, discovers the
canonical test without filters. The new canonical test requires Chrome and does not
silently skip. `rust-target-check` compiles and packages all seven target distributions.
Not proven here: native execution on all seven targets, clean installed ArchDev release
lifecycle and upgrade orchestration remain the epic's `release_proof` task (`tsk_034o1cUK08f1RmTu2Lw6tn`). Windows
currently runs PTY proofs only. The release-readiness task
`tsk_034o1cY3iGfygl7g8S3erV` depends on that release-proof gate. No claim of all-target runtime coverage is made.
