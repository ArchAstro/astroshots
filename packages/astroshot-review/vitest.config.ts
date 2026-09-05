import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    exclude: ["src/**/*.e2e.test.ts"],
    testTimeout: 15_000,
  },
});
