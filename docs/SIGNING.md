# Signing & notarization (Gatekeeper-clean DMGs)

Astroshots CI produces a **Developer ID signed + notarized** DMG so users can
double-click install without right-click → Open.

This guide is end-to-end: Apple side, then `gh secret set` commands for the
`ArchAstro/astroshots` repo.

**Prereqs on your Mac**

```bash
# GitHub CLI logged into an account that can set secrets on ArchAstro/astroshots
gh auth status
gh repo view ArchAstro/astroshots >/dev/null

# Optional: pin the repo for shorter commands
export GH_REPO=ArchAstro/astroshots
```

---

## Step 0 — Team ID

1. Open [Membership details](https://developer.apple.com/account#MembershipDetailsCard).
2. Copy **Team ID** (10 characters, e.g. `ABCDE12345`).

Set it (replace the value):

```bash
export TEAM_ID='ABCDE12345'   # your Team ID
export GH_REPO=ArchAstro/astroshots

gh secret set MACOS_DEVELOPMENT_TEAM --repo "$GH_REPO" --body "$TEAM_ID"
```

Optional mirror used by notarization if you don’t use the API key path later:

```bash
gh secret set APPLE_TEAM_ID --repo "$GH_REPO" --body "$TEAM_ID"
```

---

## Step 1 — Developer ID Application certificate

### 1a. Certificate Signing Request (CSR)

1. Open **Keychain Access**.
2. **Keychain Access → Certificate Assistant → Request a Certificate From a Certificate Authority…**
3. Email = your Apple ID; Common Name = e.g. `Astroshots Developer ID`.
4. **Saved to disk** → save `CertificateSigningRequest.certSigningRequest`.

### 1b. Request the cert from Apple

1. [Certificates list](https://developer.apple.com/account/resources/certificates/list) → **+**.
2. **Developer ID Application** → Continue.
3. Upload the CSR → download the `.cer`.

### 1c. Install + confirm identity

```bash
# Double-click the .cer first, then:
security find-identity -v -p codesigning | grep "Developer ID Application"
```

You want a line like:

```text
1) AABBCCDD… "Developer ID Application: Your Name (ABCDE12345)"
```

Copy the string **inside the quotes** (include the team id in parentheses).

```bash
# Paste your exact identity:
export CODE_SIGN_IDENTITY='Developer ID Application: Your Name (ABCDE12345)'

gh secret set MACOS_CODE_SIGN_IDENTITY --repo "$GH_REPO" --body "$CODE_SIGN_IDENTITY"
```

---

## Step 2 — Export `.p12` for CI

1. **Keychain Access → My Certificates**.
2. Select **Developer ID Application: … (TEAMID)** (ensure the private key is nested under it).
3. Right-click → **Export…** → format **.p12**.
4. Save e.g. `~/Desktop/DeveloperID-Astroshots.p12`.
5. Set a strong export password (password manager).

Then:

```bash
export P12_PATH="$HOME/Desktop/DeveloperID-Astroshots.p12"
# You will be prompted for the p12 password twice if you use --body from stdin carefully;
# easiest is interactive for the password secret:

# 1) Certificate blob
base64 -i "$P12_PATH" | gh secret set MACOS_CERTIFICATE_P12_BASE64 --repo "$GH_REPO"

# 2) Certificate password (interactive — paste password, Enter, Ctrl-D)
gh secret set MACOS_CERTIFICATE_PASSWORD --repo "$GH_REPO"
```

Non-interactive password (only if your shell history is safe / you use a throwaway shell):

```bash
# Prefer the interactive form above.
printf '%s' 'YOUR_P12_PASSWORD' | gh secret set MACOS_CERTIFICATE_PASSWORD --repo "$GH_REPO"
```

---

## Step 3 — App Store Connect API key (notarization)

1. [App Store Connect → Users and Access → Integrations → App Store Connect API](https://appstoreconnect.apple.com/access/integrations/api).
2. Note **Issuer ID** (top of the page).
3. **Generate API Key** → name e.g. `Astroshots CI` → Access **Developer** (or Admin).
4. Copy **Key ID**.
5. **Download** `AuthKey_XXXXXXXXXX.p8` (once only).

```bash
export API_KEY_PATH="$HOME/Downloads/AuthKey_XXXXXXXXXX.p8"  # fix filename
export API_KEY_ID='XXXXXXXXXX'                               # Key ID
export API_ISSUER_ID='xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx' # Issuer ID

gh secret set APPLE_API_KEY_ID --repo "$GH_REPO" --body "$API_KEY_ID"
gh secret set APPLE_API_ISSUER_ID --repo "$GH_REPO" --body "$API_ISSUER_ID"
base64 -i "$API_KEY_PATH" | gh secret set APPLE_API_KEY_BASE64 --repo "$GH_REPO"
```

### Alternative: Apple ID + app-specific password

Skip the three `APPLE_API_*` secrets and instead:

1. [appleid.apple.com](https://appleid.apple.com) → Sign-In and Security → **App-Specific Passwords** → generate one.
2. Run:

```bash
gh secret set APPLE_ID --repo "$GH_REPO" --body 'you@example.com'
gh secret set APPLE_APP_SPECIFIC_PASSWORD --repo "$GH_REPO"   # interactive paste
gh secret set APPLE_TEAM_ID --repo "$GH_REPO" --body "$TEAM_ID"
```

If both API key and Apple ID secrets exist, **API key wins** in `package-dmg.sh`.

---

## Step 4 — Verify secrets are present

```bash
export GH_REPO=ArchAstro/astroshots

gh secret list --repo "$GH_REPO"
```

You should see at least:

```text
MACOS_CERTIFICATE_P12_BASE64
MACOS_CERTIFICATE_PASSWORD
MACOS_CODE_SIGN_IDENTITY
MACOS_DEVELOPMENT_TEAM
APPLE_API_KEY_ID
APPLE_API_ISSUER_ID
APPLE_API_KEY_BASE64
```

(`gh secret list` shows **names only**, not values.)

One-shot checklist:

```bash
export GH_REPO=ArchAstro/astroshots
need=(
  MACOS_CERTIFICATE_P12_BASE64
  MACOS_CERTIFICATE_PASSWORD
  MACOS_CODE_SIGN_IDENTITY
  MACOS_DEVELOPMENT_TEAM
  APPLE_API_KEY_ID
  APPLE_API_ISSUER_ID
  APPLE_API_KEY_BASE64
)
have="$(gh secret list --repo "$GH_REPO" --json name --jq '.[].name')"
for s in "${need[@]}"; do
  if printf '%s\n' "$have" | grep -qx "$s"; then
    echo "OK  $s"
  else
    echo "MISS $s"
  fi
done
```

---

## Step 5 — Re-run CI / local smoke

### Re-run the PR DMG job

```bash
# Latest PR on the repo
gh pr checks 1 --repo ArchAstro/astroshots

# Re-run failed jobs on the latest workflow run for the PR branch
gh run list --repo ArchAstro/astroshots --branch feat/v0-macos-app-and-skills --limit 5

# Replace RUN_ID from the list:
gh run rerun RUN_ID --repo ArchAstro/astroshots --failed

# Or trigger a fresh push:
# git commit --allow-empty -m "chore: re-run signed DMG CI" && git push
```

Download the artifact when green:

```bash
gh run download RUN_ID --repo ArchAstro/astroshots -n Astroshots-dmg -D /tmp/astroshots-dmg
open /tmp/astroshots-dmg
```

### Local release build (same credentials on your Mac)

```bash
export CODE_SIGN_IDENTITY='Developer ID Application: Your Name (ABCDE12345)'
export DEVELOPMENT_TEAM='ABCDE12345'
export APPLE_API_KEY_PATH="$HOME/Downloads/AuthKey_XXXXXXXXXX.p8"
export APPLE_API_KEY_ID='XXXXXXXXXX'
export APPLE_API_ISSUER_ID='xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx'

cd macos
./scripts/package-dmg.sh
# → build/Astroshots.dmg  (signed + notarized + stapled)
```

Ad-hoc only (Gatekeeper will warn):

```bash
./scripts/package-dmg.sh --adhoc
```

---

## Copy-paste: full `gh secret set` block

After you have files + values ready, fill the `export` lines and run:

```bash
export GH_REPO=ArchAstro/astroshots

export TEAM_ID='ABCDE12345'
export CODE_SIGN_IDENTITY='Developer ID Application: Your Name (ABCDE12345)'
export P12_PATH="$HOME/Desktop/DeveloperID-Astroshots.p12"
export API_KEY_PATH="$HOME/Downloads/AuthKey_XXXXXXXXXX.p8"
export API_KEY_ID='XXXXXXXXXX'
export API_ISSUER_ID='xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx'

gh secret set MACOS_DEVELOPMENT_TEAM --repo "$GH_REPO" --body "$TEAM_ID"
gh secret set MACOS_CODE_SIGN_IDENTITY --repo "$GH_REPO" --body "$CODE_SIGN_IDENTITY"
base64 -i "$P12_PATH" | gh secret set MACOS_CERTIFICATE_P12_BASE64 --repo "$GH_REPO"
gh secret set MACOS_CERTIFICATE_PASSWORD --repo "$GH_REPO"   # paste password, Enter, Ctrl-D

gh secret set APPLE_API_KEY_ID --repo "$GH_REPO" --body "$API_KEY_ID"
gh secret set APPLE_API_ISSUER_ID --repo "$GH_REPO" --body "$API_ISSUER_ID"
base64 -i "$API_KEY_PATH" | gh secret set APPLE_API_KEY_BASE64 --repo "$GH_REPO"

gh secret list --repo "$GH_REPO"
```

---

## CI flow (what the secrets enable)

1. Import `.p12` into a temporary keychain (`macos/scripts/ci-import-certificate.sh`)
2. `package-dmg.sh` builds Release, signs app (hardened runtime + timestamp) + DMG
3. `notarytool submit --wait` → `stapler staple`
4. Upload **Astroshots-dmg** artifact; on `v*` tags, attach to the GitHub Release

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `No identity found` | Identity must match Keychain exactly; cert type must be **Developer ID Application** |
| `errSecInternalComponent` | Re-export p12; re-run import script |
| Notary `Invalid` | `xcrun notarytool log <submission-id> --key …` |
| Staple fails | Notarization not `Accepted` yet |
| `gh secret set` permission denied | `gh auth status` — need write access to `ArchAstro/astroshots` |
| Still Gatekeeper after download | Confirm staple: `xcrun stapler validate Astroshots.dmg` |

## Bundle id

`ai.archastro.Astroshots` — no App ID is required for Developer ID distribution
(only for Mac App Store). Notarization uses team credentials regardless.
