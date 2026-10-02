// WebKit tests for the note editor (`pnpm test:editor`). Vite serves the
// harness page in e2e/editor (its own root, so the app's Vite config is not
// loaded), importing the editor straight from src.
import { defineConfig } from "@playwright/test";

const port = 1430;

export default defineConfig({
  testDir: "e2e",
  timeout: 120_000,
  reporter: process.env.CI ? "github" : "list",
  use: {
    browserName: "webkit",
    baseURL: `http://localhost:${port}/`,
    viewport: { width: 900, height: 700 },
  },
  webServer: {
    command: `pnpm exec vite e2e/editor --port ${port} --strictPort`,
    url: `http://localhost:${port}/`,
    reuseExistingServer: !process.env.CI,
  },
});
