import { defineConfig, devices } from "@playwright/test";

const ownerLabUrl = "http://127.0.0.1:18787";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "rt1-local-canary.spec.ts",
  timeout: 120_000,
  fullyParallel: false,
  retries: 0,
  workers: 1,
  reporter: "line",
  use: {
    baseURL: ownerLabUrl,
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
    {
      command: "node e2e/backend-provider.mjs",
      url: "http://127.0.0.1:18788/health",
      reuseExistingServer: false,
      timeout: 120_000,
    },
    {
      command: "python3 ../../../tests/e2e/run_rt1_worker_canary.py",
      url: "http://127.0.0.1:18789/healthz",
      reuseExistingServer: false,
      timeout: 120_000,
    },
    {
      command: "python3 ../../../tests/e2e/run_rt1_owner_lab.py",
      url: ownerLabUrl + "/api/bootstrap",
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
});
