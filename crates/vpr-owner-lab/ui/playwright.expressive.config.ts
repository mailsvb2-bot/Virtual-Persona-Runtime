import { defineConfig, devices } from "@playwright/test";

const ownerLabPort = 18_791;
const providerPort = 18_790;
const ownerLabUrl = `http://127.0.0.1:${ownerLabPort}`;
const browserUrl = "http://127.0.0.1:18792";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "backend-expressive-journey.spec.ts",
  timeout: 120_000,
  fullyParallel: false,
  retries: 0,
  workers: 1,
  reporter: "line",
  use: {
    baseURL: browserUrl,
    trace: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
  ],
  webServer: [
    {
      command: "VPR_JOURNEY_UPSTREAM=http://127.0.0.1:18791 node e2e/journey-mailbox-fixture.mjs",
      url: "http://127.0.0.1:18792/health",
      reuseExistingServer: false,
    },
    {
      command: "node e2e/voice-provider-fixture.mjs",
      url: `http://127.0.0.1:${providerPort}/health`,
      reuseExistingServer: false,
    },
    {
      command: "python3 ../../../tests/e2e/run_owner_lab_expressive_backend.py",
      url: `${ownerLabUrl}/api/bootstrap`,
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
});
