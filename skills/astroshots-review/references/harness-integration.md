# Wiring harnesses to Astroshots

## agent-browser smoke (`services/agent_network/test-harness/agent-browser`)

Today `smoke_capture` writes to:

```text
$SMOKE_ARTIFACTS_DIR/screenshots/NN-slug.png
```

defaulting under `/tmp/archagents-browser-smoke/...`.

To also (or instead) feed Astroshots, in `smoke_capture`:

```bash
smoke_capture() {
  local slug="$1"
  local description="$2"
  local seq
  seq="$(printf '%02d' "$step_number")"
  local shot="$SMOKE_ARTIFACTS_DIR/screenshots/${seq}-${slug}.png"
  # …

  smoke_browser screenshot --full "$shot" >/dev/null

  # Astroshots dual-write (feature = case name)
  if command -v astroshot-capture >/dev/null 2>&1 || \
    [[ -x "${ASTROSHOT_CAPTURE:-}" ]]; then
    local capture="${ASTROSHOT_CAPTURE:-astroshot-capture}"
    "$capture" \
      --root "$REPO_ROOT" \
      --feature "$case_name" \
      --slug "$slug" \
      --description "$description" \
      --status running \
      --source "$shot" \
      --run-id "$SMOKE_RUN_ID" \
      >/dev/null || true
  fi
}
```

On pass/fail at end of `run.sh`:

```bash
astroshot-capture --root "$REPO_ROOT" --feature "$case_name" \
  --run-id "$SMOKE_RUN_ID" --status pass --finalize
# or fail
```

Install the skill (and helper) first:

```bash
npx skills add ArchAstro/astroshots --skill astroshots-review -g -y
```

Then point harnesses at the installed script (or copy it onto `PATH`):

```bash
export ASTROSHOT_CAPTURE="$(ls -d \
  ~/.agents/skills/astroshots-review/scripts/astroshot-capture \
  ~/.claude/skills/astroshots-review/scripts/astroshot-capture \
  ~/.codex/skills/astroshots-review/scripts/astroshot-capture \
  2>/dev/null | head -1)"
```

## Generic Bash harness

```bash
npx skills add ArchAstro/astroshots --skill astroshots-review -g -y

FEATURE=my-journey
RUN_ID="${FEATURE}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
CAPTURE="$(ls -d ~/.agents/skills/astroshots-review/scripts/astroshot-capture \
  ~/.claude/skills/astroshots-review/scripts/astroshot-capture \
  ~/.codex/skills/astroshots-review/scripts/astroshot-capture 2>/dev/null | head -1)"

# each step after UI is ready:
"$CAPTURE" --feature "$FEATURE" --slug step-name \
  --title "Step name" --description "What we proved" \
  --run-id "$RUN_ID" \
  --from-agent-browser "$SESSION"

"$CAPTURE" --feature "$FEATURE" --run-id "$RUN_ID" --status pass --finalize
```

## Where NOT to write

| Path | Seen by Astroshots? |
|------|---------------------|
| `<worktree>/.astroshot/<feature>/*.png` | Yes |
| `/tmp/archagents-browser-smoke/...` | No |
| `docs-content/public/screenshots/` | No (docs pipeline, not live) |
| `services/.../screenshots/` under app | No unless under a watched `.astroshot` |
