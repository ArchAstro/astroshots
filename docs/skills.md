# Agent skills

This repo ships **six** skills for coding agents. Install them with the
[skills](https://github.com/vercel-labs/skills) CLI.

| Skill | What it teaches |
|-------|-----------------|
| **astroshots-review** | Stream stills/movies under `.astroshot/` and read human feedback |
| **screenshot** | Plan, generate, review, and maintain documentation image sets |
| **astroshot** | Capture deterministic React, Ink, and PTY stills plus journey movies |
| **agent-browser** | Install & drive the agent-browser CLI |
| **browser-ui-harness** | Bash UI smoke harness design (runner vs cases, evidence, cleanup) |
| **friction-log** | Author, list, and run user-perspective UX scenarios with transcripts and evidence |

Skill sources: [`skills/`](../skills).

---

## Install matrix

### Install all skills (recommended)

**Global** (user-level, every project):

```bash
npx skills add ArchAstro/astroshots --skill '*' -g -y
```

**This git project only** (from that project's root — no `-g`):

```bash
cd /path/to/your/project
npx skills add ArchAstro/astroshots --skill '*' -y
```

`--skill '*'` installs **every** skill in this repo (`astroshot`,
`astroshots-review`, `screenshot`, `agent-browser`, `browser-ui-harness`, and
`friction-log`). Using a single name (for example,
`--skill astroshots-review`) installs just that one.

### Install one skill

```bash
# Global
npx skills add ArchAstro/astroshots --skill astroshots-review -g -y
npx skills add ArchAstro/astroshots --skill screenshot -g -y
npx skills add ArchAstro/astroshots --skill astroshot -g -y
npx skills add ArchAstro/astroshots --skill agent-browser -g -y
npx skills add ArchAstro/astroshots --skill browser-ui-harness -g -y
npx skills add ArchAstro/astroshots --skill friction-log -g -y

# This project only (from project root)
npx skills add ArchAstro/astroshots --skill agent-browser -y
```

### Agents, list, update

```bash
# Limit which coding agents receive the skills
npx skills add ArchAstro/astroshots --skill '*' -g -y -a claude-code -a cursor -a codex
npx skills add ArchAstro/astroshots --skill '*' -g -y -a '*'

# See what's in the package without installing
npx skills add ArchAstro/astroshots -l

# What's installed
npx skills list -g          # global
npx skills list             # this project

# Update
npx skills update -g -y     # global
npx skills update -y        # this project
```

---

## Proof

[`scripts/verify-skills.sh`](../scripts/verify-skills.sh) is the canonical
documentation and skill proof. CI runs its integration mode, which captures
React, Ink, and PTY images, streams them through `astroshot-capture`, and asserts
the resulting manifest.

```bash
npm run verify:skills
```
