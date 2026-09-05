# @archastro/astroshot-review

The Astroshots review tray in your terminal. Streams `.astroshot/` captures
across worktrees newest-first, draws them with the Kitty graphics protocol,
plays movies through ffmpeg, and writes the same `review.json` Seen state and
feedback as the macOS app.

```bash
npx astroshot review              # via the unified CLI (recommended)
npx astroshot-review ~/projects   # this package's own bin
```

- Terminals: Ghostty, kitty, WezTerm (anything with the Kitty graphics protocol).
  Others get labeled placeholders.
- Movies need `ffmpeg` on `PATH`.
- Keys: `↑↓` move · `⏎` open · `f` full screen · `s` seen · `c` feedback ·
  `u` history · `m` movies · `1`/`2` tabs · `,` settings · `?` help · `q` quit.

Full guide: [docs/review-tui.md](https://github.com/ArchAstro/astroshots/blob/main/docs/review-tui.md).
