import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testIgnore: [
    "backend-owner-journey.spec.ts",
    "backend-voice-journey.spec.ts",
    "backend-expressive-journey.spec.ts",
    "rt1-local-canary.spec.ts",
    // Requires the actual RT1 loopback owner server, not the static UI fixture.
    "rt1-ice-failure-cleanup.spec.ts",
    "rt1-late-microphone-revoke.spec.ts",
    "rt1-late-livekit-connect.spec.ts",
    "rt1-voice-inflight-stop.spec.ts",
    "rt1-livekit-provider-lifecycle.spec.ts",
    "rt1-canonical-provider.spec.ts",
    // This journey owns its authenticated worker/provider fixtures and runs
    // only under playwright.rt1.voice.config.ts, never the static UI server.
    "rt1-worker-voice-journey.spec.ts",
    // This real-worker journey has its own three-service harness and must not run against the static UI server.
    "rt1-worker-owner-visitor-isolation.spec.ts",
  ],
  timeout: 30_000,
  fullyParallel: false,
  retries: 0,
  workers: 1,
  reporter: "line",
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
  ],
  webServer: {
    command: "node e2e/server.mjs",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: false,
  },
});
