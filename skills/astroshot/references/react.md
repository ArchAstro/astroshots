# React mode

Use `astroshot react` when fixed props and local providers can reproduce the
state without a running application.

## Capture

Generate a starting fixture:

```bash
npx astroshot init react ./fixtures/account-dialog.tsx
```

A fixture default-exports the component and capture metadata:

```tsx
import type { ReactShotFixture } from "@archastro/astroshot/react";
import { AccountDialog } from "../src/AccountDialog";

export default {
  width: 960,
  height: 900,
  background: "transparent",
  selector: "[role=dialog]",
  waitFor: "text=Save",
  settleMs: 200,
  component: <AccountDialog accountName="Acme" onClose={() => {}} />,
} satisfies ReactShotFixture;
```

Capture it:

```bash
npx astroshot react ./fixtures/account-dialog.tsx \
  -o ./screenshots/account-dialog.png
```

## Fixture design

- Import the production component rather than copying its markup.
- Supply stable synthetic props and local providers.
- Mock network calls and server-only modules.
- Select the smallest meaningful element; avoid unrelated canvas.
- Wait for distinctive product content, fonts, and layout.
- Use `settleMs` only for a short animation or paint settle after readiness.

For aliases, styles, stubs, and React deduplication, place
`react-shot.config.ts` near the application package. Use `--root` when package
root discovery is ambiguous. See `packages/react-shot/README.md` when working
inside the Astroshots repository for the complete configuration contract.

Use `agent-browser` instead when authentication, routing, live backend data, or
the complete application shell is part of the proof.
