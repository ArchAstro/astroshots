# `@archastro/astroshot`

One command for deterministic React component and Ink terminal UI screenshots.

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot react ./fixtures/account-dialog.tsx \
  -o ./screenshots/account-dialog.png

npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot tui ./fixtures/install-wizard.tsx \
  -o ./screenshots/install-wizard.png
```

Install the shared Chromium runtime once:

```bash
npx --@archastro:registry=https://registry.npmjs.org \
  @archastro/astroshot install-browser
```

Use `react batch <manifest>` or `tui batch <manifest>` for maintained image
sets. Run `react --help` or `tui --help` for mode-specific options.

Fixture types are available from the unified package:

```tsx
import type { ReactShotFixture } from "@archastro/astroshot/react";
import type { TuiShotFixture } from "@archastro/astroshot/tui";
```

Fixtures and their imports are executable code with the current user's
permissions. Review untrusted fixtures before capture and never include
credentials or private customer data in fixtures or generated images.
