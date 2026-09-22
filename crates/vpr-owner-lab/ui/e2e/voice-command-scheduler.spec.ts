import { expect, test } from "@playwright/test";

import { PlaybackAwareCommandScheduler } from "../src/voice-command-scheduler.js";

test("playback scheduler waits for previous phrase before dispatching the next", async () => {
  const sent: string[] = [];
  const scheduler = new PlaybackAwareCommandScheduler<string>(async (command) => {
    sent.push(command);
  });

  await expect(scheduler.dispatch("first")).resolves.toBeTruthy();
  const second = scheduler.dispatch("second");
  await new Promise((resolve) => setTimeout(resolve, 25));
  expect(sent).toEqual(["first"]);

  scheduler.playbackDone();
  await expect(second).resolves.toBeTruthy();
  expect(sent).toEqual(["first", "second"]);
});

test("interrupt releases waiters and prevents queued speech from being sent", async () => {
  const sent: string[] = [];
  const scheduler = new PlaybackAwareCommandScheduler<string>(async (command) => {
    sent.push(command);
  });

  await scheduler.dispatch("first");
  const queued = scheduler.dispatch("second");
  expect(scheduler.hasActivePlayback).toBeTruthy();

  scheduler.interrupt();

  await expect(queued).resolves.toBeFalsy();
  expect(sent).toEqual(["first"]);
  expect(scheduler.hasActivePlayback).toBeFalsy();
});
