import { defineConfig } from "vitest/config";

// Unit vs e2e: Vite / Chromium journeys (cli.e2e, shot-concurrency.e2e) live
// only here. Do not fold takeShot / published-bin captures into the unit
// config — `vitest run` / `test:unit` must stay Chromium-free.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.e2e.test.ts"],
    testTimeout: 90_000,
    hookTimeout: 90_000,
    maxWorkers: 1,
  },
});
