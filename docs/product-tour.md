# Product tour

| Surface | Job |
|---------|-----|
| **Desktop overlay** | New frame above all windows; Open / dismiss |
| **Shots stream** | Newest-first stills and movies, worktree grouping, Unseen/History, Movies filter |
| **Movie detail** | Playback, chapters, duration/source metadata, feedback, Seen state |
| **Friction Logs** | Scenario prompts, run history, improvement rollup, step screenshots/transcripts/notes |
| **Narrated video** | Optional on-device transcript-to-MP4 generation on Apple Silicon |
| **Settings** | Watched folders, overlay behavior, narration, and software updates |

---

## Why Astroshots

| Problem | What Astroshots does |
|---------|----------------------|
| Screenshots and recordings buried in `/tmp` or CI artifacts | Live stream as soon as a still or movie lands |
| Jumping between projects to find shots | One tray, every project under your watched folders |
| "Did that step look right?" mid-run | Desktop overlay above all windows |
| Manual folder digging after a suite | Newest-first history with titles from a small manifest |
| UX findings scattered across chat and screenshots | Friction Logs pair every step with evidence, transcript, and good/improve notes |
| Repeating a walkthrough loses the earlier result | Per-scenario run history keeps every non-empty attempt switchable |

No project picker. No account. No cloud. Just files on disk and an Astroshots
icon in the menu bar.

<p align="center">
  <img src="images/overlay-card.png" alt="Desktop overlay for a newly captured onboarding movie with a direct Open in Astroshots action" width="420" />
</p>

---

## The full loop

1. **Watch** — On first launch you choose one or more folders; Astroshots
   recursively watches them for `.astroshot/` trees.
2. **Write** — A harness, agent, or script writes stills, movie poster+video
   pairs, and optional execution metadata under `.astroshot/`
   ([contract](contract.md)).
3. **See** — New review frames flash as desktop overlays. **Shots** combines
   stills and movies across worktrees; the Movies filter isolates recordings.
4. **Play** — Movie detail and full-screen review play WebM, MP4, and MOV with
   scrubbing, volume, full-screen controls, duration/source metadata, and
   chapters.
5. **Review** — Send feedback or mark the current poster/image Seen. Astroshots
   writes hash- and run-scoped human state to `review.json`.
6. **Walk the product** — **Friction Logs** lists agentic scenarios under the
   reserved `.astroshot/friction-logs/` tree
   ([friction logs](friction-logs.md)).
7. **Narrate (optional)** — On Apple Silicon, enable Settings → Narration to
   generate an on-device MP4 from step screenshots and transcripts with
   Qwen3-TTS. Models download only after opt-in.

---

## Movies in Shots

<p align="center">
  <img src="images/shots-movie-stream.png" alt="Shots stream with the Movies filter and a one-second onboarding movie ready for review" width="360" />
  &nbsp;
  <img src="images/movie-detail.png" alt="Movie detail showing playback actions, chapter metadata, feedback, and Seen acknowledgement" width="360" />
</p>

## Friction Logs

<p align="center">
  <img src="images/friction-logs.png" alt="Friction Logs tab listing a completed checkout scenario with two retained runs and two improvement notes" width="280" />
  &nbsp;
  <img src="images/friction-step.png" alt="Friction-log step detail pairing visual evidence with transcript, Looks good, and Can improve notes" width="280" />
</p>

Every image above comes from the current native Debug app and a synthetic local
fixture. Inventory: [`screenshots.json`](screenshots.json). Regenerate the
complete set with `bash scripts/capture-readme-screenshots.sh`.

---

## Requirements

- macOS 14 or later
- One or more folders of projects to watch (configure in-app)
- Tools that can write stills or use `astroshot movie` (any language or harness
  may write the on-disk contract)

The npm screenshot tools require Node.js 22.14 or later and a locally installed
Chromium managed by Playwright. Native-window movie capture needs macOS Screen
Recording permission. Optional narrated friction-log videos need Apple Silicon;
enabling Narration downloads the Qwen3-TTS model locally.
