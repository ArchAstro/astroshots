# Friction logs

A friction log is a user-perspective walkthrough of your product: ordered steps,
visual evidence, a spoken transcript, and what was good or should improve. The
tray reads them from a reserved namespace under `.astroshot/`.

---

## Layout

`friction-logs` is a reserved namespace, not a Shots feature. Scenario prompts
and every non-empty attempt stay together:

```text
<project>/.astroshot/friction-logs/checkout-as-new-user/
  prompt.md
  meta.json
  runs/20260811T153000Z/
    log.jsonl
    0001-choose-plan.png
    0002-confirm-checkout.png
  runs/20260810T180000Z/
    log.jsonl
    0001-choose-plan.png
```

---

## Step schema (`log.jsonl`)

Each line is one user-visible step:

```json
{
  "step": 1,
  "id": "choose-plan",
  "title": "Choose a plan",
  "description": "Compared plans from a clean session.",
  "transcript": "I arrive at pricing and compare the plans. The differences are easy to scan, but annual savings need a clearer explanation. I choose the team plan and continue to checkout.",
  "screenshots": ["0001-choose-plan.png"],
  "good": ["Plan differences are easy to scan"],
  "improve": ["Annual savings need a clearer explanation"],
  "url": "/pricing"
}
```

New runs require a short spoken `transcript` per step. Read all transcripts in
order and they should form one continuous narration: action taken, what worked,
what did not, and a transition into the next step.

---

## How the tray reads it

- Loads every non-empty run **newest-first** and hides empty stubs.
- Rolls up all `improve` notes into a per-run improvement list.
- Lets the reviewer switch runs and step through evidence with ← →.
- Pairs each step's screenshots with its transcript, **Looks good**, and
  **Can improve** notes.

<p align="center">
  <img src="images/friction-logs.png" alt="Friction Logs tab listing a completed checkout scenario with two retained runs and two improvement notes" width="280" />
  &nbsp;
  <img src="images/friction-run.png" alt="Friction-log run detail with run history, Make narrated video, improvement rollup, and two steps" width="280" />
  &nbsp;
  <img src="images/friction-step.png" alt="Friction-log step detail pairing visual evidence with transcript, Looks good, and Can improve notes" width="280" />
</p>

---

## Narrated video (optional)

On Apple Silicon, enable Settings → Narration to generate an on-device MP4 from
step screenshots and transcripts with Qwen3-TTS. Models download only after
opt-in. Astroshots derives the video **after** the run; agents still write the
screenshots and transcripts, never TTS output.

---

## Authoring

Install and use the **friction-log** skill to author, list, or execute this
contract — see [`docs/skills.md`](skills.md).
