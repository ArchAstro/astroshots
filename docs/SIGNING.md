# Signing & notarization (Gatekeeper-clean DMGs)

Astroshots CI produces a **Developer ID signed + notarized** DMG so users can
double-click install without right-click → Open.

## What you need from Apple Developer

1. **Developer ID Application** certificate  
   [Certificates, Identifiers & Profiles](https://developer.apple.com/account/resources/certificates/list)  
   → **+** → **Developer ID Application**  
   Export from Keychain as `.p12` (set a password).

2. **Team ID** (10 characters) — Membership details in the developer account.

3. **App Store Connect API key** (recommended for CI)  
   [Users and Access → Integrations → App Store Connect API](https://appstoreconnect.apple.com/access/integrations/api)  
   → Generate key with **Developer** (or Admin) access  
   → Download `AuthKey_XXXXXXXXXX.p8` once  
   → Note **Key ID** and **Issuer ID**

   *Alternative:* Apple ID + [app-specific password](https://appleid.apple.com)  
   (works; API key is cleaner for GitHub Actions.)

## GitHub repository secrets

Settings → Secrets and variables → Actions → **New repository secret**.

| Secret | Value |
|--------|--------|
| `MACOS_CERTIFICATE_P12_BASE64` | `base64 -i DeveloperID.p12 \| pbcopy` |
| `MACOS_CERTIFICATE_PASSWORD` | Password you set when exporting the .p12 |
| `MACOS_CODE_SIGN_IDENTITY` | Exact identity string, e.g. `Developer ID Application: ArchAstro Inc (ABCDE12345)` |
| `MACOS_DEVELOPMENT_TEAM` | Team ID, e.g. `ABCDE12345` |
| `APPLE_API_KEY_ID` | Key ID from App Store Connect |
| `APPLE_API_ISSUER_ID` | Issuer UUID from App Store Connect |
| `APPLE_API_KEY_BASE64` | `base64 -i AuthKey_XXX.p8 \| pbcopy` |

### Optional (Apple ID notarization instead of API key)

| Secret | Value |
|--------|--------|
| `APPLE_ID` | Your Apple ID email |
| `APPLE_APP_SPECIFIC_PASSWORD` | App-specific password |
| `APPLE_TEAM_ID` | Same as team ID (defaults to `MACOS_DEVELOPMENT_TEAM`) |

If API key secrets are set, they win over Apple ID.

### Finding the exact identity string

On a Mac that has the cert installed:

```bash
security find-identity -v -p codesigning | grep "Developer ID Application"
```

Copy the quoted name **including** the team id in parentheses.

## Local release build

```bash
export CODE_SIGN_IDENTITY="Developer ID Application: … (TEAMID)"
export DEVELOPMENT_TEAM=TEAMID
export APPLE_API_KEY_PATH=~/Downloads/AuthKey_XXX.p8
export APPLE_API_KEY_ID=XXX
export APPLE_API_ISSUER_ID=xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx

cd macos
./scripts/package-dmg.sh
# → build/Astroshots.dmg  (signed + notarized + stapled)
```

Ad-hoc (dev only, Gatekeeper warns):

```bash
./scripts/package-dmg.sh --adhoc
```

## CI flow

1. Import `.p12` into a temporary keychain (`scripts/ci-import-certificate.sh`)
2. `package-dmg.sh` builds Release, signs the app (hardened runtime + timestamp),
   builds the DMG, signs the DMG, submits to Apple notary, staples the ticket
3. Upload **Astroshots-dmg** artifact / attach to release on `v*` tags

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `No identity found` | Identity string must match Keychain exactly; cert must be **Developer ID Application** (not Mac Development) |
| `errSecInternalComponent` | Re-import p12; check `security set-key-partition-list` ran |
| Notary `Invalid` | Hardened runtime off, unsigned nested code, or wrong entitlements — check notary log: `xcrun notarytool log <id> …` |
| Staple fails | Notarization did not succeed; wait for `Accepted` status |
| Still Gatekeeper on download | Browser quarantine — `xattr -cr Astroshots.app` after copy, or ensure staple succeeded: `stapler validate Astroshots.dmg` |

## Bundle id

`ai.archastro.Astroshots` — no App ID registration is required for Developer ID
distribution (only for Mac App Store). Notarization uses your team credentials
regardless.
