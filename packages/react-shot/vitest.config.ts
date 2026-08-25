import { defineConfig } from "vitest/config";

// Unit vs e2e: this config is unit only (meta, batch-paths, config, create-server).
// Do not put takeShot / Vite / Chromium journeys here — those belong in
// vitest.e2e.config.ts and run only via `test:e2e`. Explicit testTimeout so
// unit never inherits Vitest's surprise 5s default after a Chromium-heavy step.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
    exclude: ["src/**/*.e2e.test.ts"],
    testTimeout: 30_000,
  },
});
