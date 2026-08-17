# Unscoped `astroshot` npm package — design notes

## Problem

`npx -y @archastro/astroshot@latest` fails with **E404** on a normal developer
machine whose `~/.npmrc` maps the scope to GitHub Packages:

```
@archastro:registry=https://npm.pkg.github.com
```

The README worked around this six times with
`npx --@archastro:registry=https://registry.npmjs.org …`, which reads to a new
user as "this package is hard to install".

## Disproven approach — do not retry

A thin wrapper that merely **depends** on `@archastro/astroshot` cannot work.
Measured under a hijack environment
(`@archastro:registry=http://127.0.0.1:9/`, `registry=https://registry.npmjs.org/`,
`--userconfig` neutralized):

1. Plain dependency → npm resolves the scoped dep through the *user's* scope
   registry: `FetchError request to http://127.0.0.1:9/@archastro%2fastroshot`.
2. Pinning the dep to an npmjs tarball URL fails one level **deeper**:
   `FetchError … @archastro%2fmovie-harness, requiredBy:
   node_modules/@archastro/astroshot` — the CLI's own engine deps get
   redirected.

**Conclusion:** any design that fetches a scoped package *at install time* can
be hijacked by user-level scope config.

## Shipped design

Unscoped workspace `astroshot` (directory `packages/astroshot-unscoped`):

1. **Bundles** `@archastro/astroshot` and its three engines through
   `bundleDependencies`, so nothing scoped is fetched at install time.
2. Declares the engines' **unscoped** runtime dependencies as its own, so they
   resolve from the default registry, which scope config cannot touch.
3. Keeps `node-pty` **optional** (it is `optionalDependencies` in `tui-shot`
   and `movie-harness`; making it required breaks installs that cannot build
   the native addon).
4. Deliberately does **not** absorb `peerDependencies` (`react`, `react-dom`,
   `ink`) — those are unscoped, unhijackable, and correctly the consumer's
   choice of framework version.

## Hazards guarded in CI

| Hazard | Guard |
|--------|-------|
| Engine `dist/` missing at pack time (npm bundles it silently, no error, no warning) | `prepack` builds every engine; `scripts/verify-packages.mjs` asserts the packed tarball **contains** each engine's built `dist/` output |
| An engine gains an unscoped runtime dep the wrapper does not declare | `packages/astroshot-unscoped/scripts/assert-bundled-deps.mjs` union assertion; names the offending package and dependency |
| Regression to fetching scoped packages at install time | hijack proof in `scripts/verify-packages.mjs` |

## Publish order

engines → `@archastro/astroshot` → `astroshot` (last; it bundles the CLI, which
depends on the engines).
