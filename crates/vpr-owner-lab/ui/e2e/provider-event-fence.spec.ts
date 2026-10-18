import { expect, test } from "@playwright/test";
import { providerEventStillCurrent } from "../src/provider-event-fence.js";

test("delayed D-ID completion from old voice never belongs to a new voice", () => {
  const oldVoice = {};
  const newVoice = {};
  expect(providerEventStillCurrent(oldVoice, oldVoice, 7, 7, -1)).toBe(true);
  expect(providerEventStillCurrent(oldVoice, newVoice, 7, 7, -1)).toBe(false);
  expect(providerEventStillCurrent(oldVoice, null, 7, 7, -1)).toBe(false);
});

test("provider receipts after revoke or across sessions cannot advance playback", () => {
  const voice = {};
  expect(providerEventStillCurrent(voice, voice, 7, 8, -1)).toBe(false);
  expect(providerEventStillCurrent(voice, voice, 7, 7, 7)).toBe(false);
  expect(providerEventStillCurrent(null, null, 7, 7, 7)).toBe(false);
  expect(providerEventStillCurrent(null, null, 7, 7, -1)).toBe(true);
});

test("new active turn after an earlier cancellation accepts only its own events", () => {
  const cancelled = {};
  const active = {};
  expect(providerEventStillCurrent(cancelled, active, 10, 10, -1)).toBe(false);
  expect(providerEventStillCurrent(active, active, 10, 10, -1)).toBe(true);
});


test("rejected provider normalization after turn replacement or revoke cannot affect new evidence", async () => {
  const oldVoice = { providerParseFailurePosted: false };
  const currentVoice = { providerParseFailurePosted: false };
  await Promise.reject(new Error("provider unavailable")).catch(() => {
    if (providerEventStillCurrent(oldVoice, currentVoice, 12, 12, -1)) {
      currentVoice.providerParseFailurePosted = true;
    }
  });
  expect(currentVoice.providerParseFailurePosted).toBe(false);
  await Promise.reject(new Error("session revoked")).catch(() => {
    if (providerEventStillCurrent(oldVoice, oldVoice, 12, 12, 12)) {
      oldVoice.providerParseFailurePosted = true;
    }
  });
  expect(oldVoice.providerParseFailurePosted).toBe(false);
});
