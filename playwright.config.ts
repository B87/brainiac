// WebKit tests (`pnpm test:editor`). Vite serves two pages: the note editor
// alone in e2e/editor (its own root, so the app's Vite config is not loaded),
// and the whole app over a fake backend in e2e/app (with the app's config, for
// React and Tailwind). Both import straight from src.
import { defineConfig } from "@playwright/test";

const port = 1430;
/** The app page; `e2e/app/app.spec.ts` uses it as its base URL. */
export const appPort = 1431;

export default defineConfig({
  testDir: "e2e",
  timeout: 120_000,
  reporter: process.env.CI ? "github" : "list",
  use: {
    browserName: "webkit",
    baseURL: `http://localhost:${port}/`,
    viewport: { width: 900, height: 700 },
  },
  webServer: [
    {
      command: `pnpm exec vite e2e/editor --port ${port} --strictPort`,
      url: `http://localhost:${port}/`,
      reuseExistingServer: !process.env.CI,
    },
    {
      command: `pnpm exec vite e2e/app --config vite.config.ts --port ${appPort} --strictPort`,
      url: `http://localhost:${appPort}/`,
      reuseExistingServer: !process.env.CI,
    },
  ],
});
