import { defineConfig } from "vitest/config";

// Testy czystej logiki (bez DOM) – środowisko node.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
