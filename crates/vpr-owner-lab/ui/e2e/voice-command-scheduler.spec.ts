import { expect, test } from "@playwright/test";

import { PlaybackAwareCommandScheduler } from "../src/voice-command-scheduler.js";

test("playback scheduler dispatches three queued phrases in strict FIFO order", async () => {
  const sent: string[] = [];
  const scheduler = new PlaybackAwareCommandScheduler<string>(async (command) => {
    sent.push(command);
  });

  await expect(scheduler.dispatch("first")).resolves.toBeTruthy();
  const second = scheduler.dispatch("second");
  const third = scheduler.dispatch("third");

  expect(scheduler.hasPendingPlayback).toBeTruthy();
  await new Promise((resolve) => setTimeout(resolve, 25));
  expect(sent).toEqual(["first"]);

  scheduler.playbackDone();
  expect(scheduler.hasPendingPlayback).toBeTruthy();
  await expect(second).resolves.toBeTruthy();
  await new Promise((resolve) => setTimeout(resolve, 25));
  expect(sent).toEqual(["first", "second"]);

  scheduler.playbackDone();
  expect(scheduler.hasPendingPlayback).toBeTruthy();
  await expect(third).resolves.toBeTruthy();
  expect(sent).toEqual(["first", "second", "third"]);

  scheduler.playbackDone();
  expect(scheduler.hasPendingPlayback).toBeFalsy();
});

test("interrupt releases playback and cancels every queued phrase from the old generation", async () => {
  const sent: string[] = [];
  const scheduler = new PlaybackAwareCommandScheduler<string>(async (command) => {
    sent.push(command);
  });

  await scheduler.dispatch("first");
  const second = scheduler.dispatch("second");
  const third = scheduler.dispatch("third");
  expect(scheduler.hasActivePlayback).toBeTruthy();

  scheduler.interrupt();

  await expect(second).resolves.toBeFalsy();
  await expect(third).resolves.toBeFalsy();
  expect(sent).toEqual(["first"]);
  expect(scheduler.hasActivePlayback).toBeFalsy();
  expect(scheduler.hasPendingPlayback).toBeFalsy();
});

test("send failure rejects only that phrase and does not poison the FIFO queue", async () => {
  const sent: string[] = [];
  const scheduler = new PlaybackAwareCommandScheduler<string>(async (command) => {
    if (command === "broken") throw new Error("transport failed");
    sent.push(command);
  });

  await expect(scheduler.dispatch("broken")).rejects.toThrow("transport failed");
  expect(scheduler.hasPendingPlayback).toBeFalsy();
  await expect(scheduler.dispatch("recovered")).resolves.toBeTruthy();
  expect(scheduler.hasPendingPlayback).toBeTruthy();
  expect(sent).toEqual(["recovered"]);
  scheduler.playbackDone();
  expect(scheduler.hasPendingPlayback).toBeFalsy();
});
