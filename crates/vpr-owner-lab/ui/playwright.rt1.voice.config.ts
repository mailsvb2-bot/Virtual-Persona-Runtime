import { defineConfig, devices } from "@playwright/test";

const ownerLabUrl = "http://127.0.0.1:18787";
const providerUrl = "http://127.0.0.1:18790";
const workerUrl = "http://127.0.0.1:18789";

export default defineConfig({
  testDir: "./e2e",
  // These browser-only race tests use explicit fake SDK/media objects. They
  // complement, but never substitute for, the local real-RTP worker journey.
  testMatch: [
    "rt1-worker-owner-visitor-isolation.spec.ts",
    "rt1-worker-voice-journey.spec.ts",
    "rt1-late-livekit-connect.spec.ts",
    "rt1-late-microphone-revoke.spec.ts",
    "rt1-voice-inflight-stop.spec.ts",
  ],
  timeout: 150_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  retries: 0,
  workers: 1,
  reporter: "line",
  use: {
    baseURL: ownerLabUrl,
    trace: "retain-on-failure",
    permissions: ["microphone"],
    launchOptions: {
      args: [
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
        "--autoplay-policy=no-user-gesture-required",
      ],
    },
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: [
    {
      command: "VPR_RT1_E2E_AUDIO_TONE=true VPR_RT1_E2E_INTERRUPT_DELAY_MS=2000 node e2e/voice-provider-fixture.mjs",
      url: providerUrl + "/health",
      reuseExistingServer: false,
      timeout: 120_000,
    },
    {
      command: "python3 ../../../tests/e2e/run_rt1_persona_worker_fixture.py",
      url: workerUrl + "/healthz",
      reuseExistingServer: false,
      timeout: 120_000,
    },
    {
      command: "python3 ../../../tests/e2e/run_rt1_voice_owner_lab.py",
      url: ownerLabUrl + "/api/bootstrap",
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
});
