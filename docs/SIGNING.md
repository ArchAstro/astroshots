# Signing setup (one list, top to bottom)

Do **each step in order**. Don’t skip ahead. When a step says “run this,” run it before the next step.

Repo: `ArchAstro/astroshots`

---

### 1. Open a terminal and set the repo

```bash
export GH_REPO=ArchAstro/astroshots
gh auth status
```

You should be logged in as someone who can edit secrets on that repo. If not: `gh auth login`.

---

### 2. Get your Team ID

1. Open: https://developer.apple.com/account#MembershipDetailsCard  
2. Copy **Team ID** (10 characters).

```bash
export TEAM_ID='PASTE_TEAM_ID_HERE'

gh secret set MACOS_DEVELOPMENT_TEAM --repo "$GH_REPO" --body "$TEAM_ID"
```

---

### 3. Make a Certificate Signing Request (CSR)

1. Open **Keychain Access** (Spotlight → Keychain Access).  
2. Menu: **Keychain Access → Certificate Assistant → Request a Certificate From a Certificate Authority…**  
3. **User Email** = your Apple ID email.  
4. **Common Name** = `Astroshots Developer ID`.  
5. Select **Saved to disk**.  
6. Save the file (Desktop is fine). Leave CA email blank.

No terminal command for this step.

---

### 4. Create the Developer ID Application certificate

1. Open: https://developer.apple.com/account/resources/certificates/list  
2. Click **+**.  
3. Choose **Developer ID Application** → Continue.  
4. Upload the CSR from step 3 → Continue.  
5. **Download** the `.cer` file.  
6. **Double-click** the `.cer` so it installs in Keychain.

---

### 5. Copy the signing identity and save it as a secret

```bash
security find-identity -v -p codesigning | grep "Developer ID Application"
```

You should see a line like:

```text
1) AABBCC… "Developer ID Application: Your Name (ABCDE12345)"
```

Copy **everything inside the quotes** (including the team id in parentheses).

```bash
export CODE_SIGN_IDENTITY='PASTE_THE_QUOTED_IDENTITY_HERE'

gh secret set MACOS_CODE_SIGN_IDENTITY --repo "$GH_REPO" --body "$CODE_SIGN_IDENTITY"
```

If `grep` printed nothing: the cert isn’t installed or the private key is missing — redo steps 3–4 on **this** Mac.

---

### 6. Export a .p12 file

1. **Keychain Access → My Certificates** (login keychain).  
2. Find **Developer ID Application: …** (expand it — private key should be underneath).  
3. Right-click the certificate → **Export…**.  
4. Format: **Personal Information Exchange (.p12)**.  
5. Save as: `~/Desktop/DeveloperID-Astroshots.p12`.  
6. Set a **password** for the file. Remember it (password manager).  
7. Allow Keychain access if macOS asks.

---

### 7. Upload the .p12 to GitHub

```bash
export P12_PATH="$HOME/Desktop/DeveloperID-Astroshots.p12"

base64 -i "$P12_PATH" | gh secret set MACOS_CERTIFICATE_P12_BASE64 --repo "$GH_REPO"
```

Then set the password (type/paste password, press Enter, then **Ctrl-D**):

```bash
gh secret set MACOS_CERTIFICATE_PASSWORD --repo "$GH_REPO"
```

---

### 8. Create an App Store Connect API key

1. Open: https://appstoreconnect.apple.com/access/integrations/api  
2. At the top, copy **Issuer ID** (a UUID).  
3. Click **Generate API Key** / **+**.  
4. Name: `Astroshots CI`.  
5. Access: **Developer** (or Admin).  
6. Generate.  
7. Copy **Key ID**.  
8. **Download** the `AuthKey_XXXX.p8` file (you only get one chance — keep it).

---

### 9. Upload the API key secrets

Fix the three values, then run all three commands:

```bash
export API_KEY_PATH="$HOME/Downloads/AuthKey_XXXXXXXXXX.p8"
export API_KEY_ID='PASTE_KEY_ID'
export API_ISSUER_ID='PASTE_ISSUER_UUID'

gh secret set APPLE_API_KEY_ID --repo "$GH_REPO" --body "$API_KEY_ID"
gh secret set APPLE_API_ISSUER_ID --repo "$GH_REPO" --body "$API_ISSUER_ID"
base64 -i "$API_KEY_PATH" | gh secret set APPLE_API_KEY_BASE64 --repo "$GH_REPO"
```

---

### 10. Confirm every secret exists

```bash
gh secret list --repo "$GH_REPO"
```

You need **all** of these names:

- `MACOS_DEVELOPMENT_TEAM`
- `MACOS_CODE_SIGN_IDENTITY`
- `MACOS_CERTIFICATE_P12_BASE64`
- `MACOS_CERTIFICATE_PASSWORD`
- `APPLE_API_KEY_ID`
- `APPLE_API_ISSUER_ID`
- `APPLE_API_KEY_BASE64`

Quick check:

```bash
need=(
  MACOS_DEVELOPMENT_TEAM
  MACOS_CODE_SIGN_IDENTITY
  MACOS_CERTIFICATE_P12_BASE64
  MACOS_CERTIFICATE_PASSWORD
  APPLE_API_KEY_ID
  APPLE_API_ISSUER_ID
  APPLE_API_KEY_BASE64
)
have="$(gh secret list --repo "$GH_REPO" --json name --jq '.[].name')"
for s in "${need[@]}"; do
  if printf '%s\n' "$have" | grep -qx "$s"; then echo "OK  $s"
  else echo "MISS $s"
  fi
done
```

Every line should say **OK**. If any say **MISS**, go back to that step.

---

### 11. Build a signed DMG (tags only — not PRs)

CI on PRs only runs **tests**. A Gatekeeper-clean DMG is built when you push a version tag:

```bash
# After the PR is on main (or from a release commit):
git checkout main && git pull
git tag v0.1.0
git push origin v0.1.0
```

Watch the **Release DMG** workflow:

```bash
gh run list --repo "$GH_REPO" --workflow "Release DMG" --limit 3
gh run watch PASTE_RUN_ID --repo "$GH_REPO"
```

When green, the DMG is on the GitHub Release and as artifact **Astroshots-dmg**:

```bash
gh release download v0.1.0 --repo "$GH_REPO" -p '*.dmg' -D /tmp/astroshots-dmg
open /tmp/astroshots-dmg
```

Manual re-run for an existing tag: Actions → **Release DMG** → **Run workflow** → enter tag.

---

### 12. (Optional) Build the same way on your Mac

Only if you want a local notarized DMG. Reuse the same values from earlier steps:

```bash
export CODE_SIGN_IDENTITY='PASTE_THE_SAME_IDENTITY_AS_STEP_5'
export DEVELOPMENT_TEAM="$TEAM_ID"
export APPLE_API_KEY_PATH="$API_KEY_PATH"
export APPLE_API_KEY_ID="$API_KEY_ID"
export APPLE_API_ISSUER_ID="$API_ISSUER_ID"

cd ~/archastro/astroshots/macos   # or your clone path
./scripts/package-dmg.sh
open build/Astroshots.dmg
```

---

## If something breaks

| What you see | What to do |
|--------------|------------|
| No “Developer ID Application” in certificates | Paid Apple Developer team; right role on the account |
| Step 5 `grep` empty | Cert not installed or CSR wasn’t made on this Mac — redo 3–4 |
| `gh secret set` denied | `gh auth login` with a user that has admin on `ArchAstro/astroshots` |
| CI still fails packaging | Run the check in step 10; re-run step 11 |
| Notary invalid | Wrong cert type or identity string — step 5 identity must match the p12 in step 6 |

---

## What this is for

On **version tags** (`v*`), the **Release DMG** workflow imports the `.p12`, builds Astroshots, **Developer ID signs** it, builds a DMG, **notarizes** with the API key, **staples** the ticket, and attaches it to the GitHub Release. That install should open without right-click → Open.

PRs only run unit tests (no notarization).
