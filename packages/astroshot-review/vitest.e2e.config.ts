import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["src/**/*.e2e.test.ts"],
    testTimeout: 90_000,
    hookTimeout: 90_000,
    maxWorkers: 1,
  },
});
