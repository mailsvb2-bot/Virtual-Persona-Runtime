import { defineConfig, devices } from "@playwright/test";

const ownerLabPort = 18_787;
const providerPort = 18_788;
const ownerLabUrl = `http://127.0.0.1:${ownerLabPort}`;

export default defineConfig({
  testDir: "./e2e",
  testMatch: "backend-owner-journey.spec.ts",
  timeout: 45_000,
  fullyParallel: false,
  retries: 0,
  workers: 1,
  reporter: "line",
  use: {
    baseURL: ownerLabUrl,
    trace: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
  ],
  webServer: [
    {
      command: "node e2e/backend-provider.mjs",
      url: `http://127.0.0.1:${providerPort}/health`,
      reuseExistingServer: false,
    },
    {
      command: "python3 ../../../tests/e2e/run_owner_lab_backend.py",
      url: `${ownerLabUrl}/api/bootstrap`,
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
});
