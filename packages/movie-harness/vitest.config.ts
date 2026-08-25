import { defineConfig } from "vitest/config";

// Unit vs e2e: movie-harness has no separate e2e config. Encode / PTY movie
// cases live here with an explicit 120s budget (never Vitest's 5s default).
// Do not put takeShot / takeTuiShot in a unit suite — those belong in the
// react-shot / tui-shot e2e configs. desktop-macos.test.ts is skippable (TCC)
// and Factory-excluded via `vitest run --exclude src/desktop-macos.test.ts`.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    testTimeout: 120_000,
    hookTimeout: 120_000,
  },
});
