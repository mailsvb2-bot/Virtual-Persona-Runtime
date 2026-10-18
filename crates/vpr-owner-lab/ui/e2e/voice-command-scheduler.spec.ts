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

test("late send completion after STOP cannot revive playback or dispatch stale queue", async () => {
  let releaseSend!: () => void;
  const gate = new Promise<void>((resolve) => { releaseSend = resolve; });
  const sent: string[] = [];
  const scheduler = new PlaybackAwareCommandScheduler<string>(async (command) => {
    sent.push(command);
    if (command === "old-first") await gate;
  });
  const first = scheduler.dispatch("old-first");
  const second = scheduler.dispatch("old-second");
  await expect.poll(() => sent.length).toBe(1);
  scheduler.interrupt();
  const newTurn = scheduler.dispatch("new-turn");
  releaseSend();
  await expect(first).resolves.toBe(false);
  await expect(second).resolves.toBe(false);
  await expect(newTurn).resolves.toBe(true);
  expect(sent).toEqual(["old-first", "new-turn"]);
  expect(scheduler.hasActivePlayback).toBe(true);
  scheduler.playbackDone();
  expect(scheduler.hasPendingPlayback).toBe(false);
});

test("each playback barrier release advances exactly one queued phrase", async () => {
  const sent: string[] = [];
  const scheduler = new PlaybackAwareCommandScheduler<string>(async (command) => {
    sent.push(command);
  });
  await expect(scheduler.dispatch("one")).resolves.toBe(true);
  const second = scheduler.dispatch("two");
  const third = scheduler.dispatch("three");
  scheduler.playbackDone();
  await expect(second).resolves.toBe(true);
  // Scheduler barriers are deliberately receipt-agnostic: provider correlation must
  // happen at the caller boundary before playbackDone() is invoked.
  expect(sent).toEqual(["one", "two"]);
  scheduler.playbackDone();
  await expect(third).resolves.toBe(true);
  expect(sent).toEqual(["one", "two", "three"]);
  scheduler.playbackDone();
  scheduler.playbackDone();
  expect(scheduler.hasPendingPlayback).toBe(false);
});

test("repeated STOP is idempotent and a new turn is not poisoned by cancelled older work", async () => {
  const sent: number[] = [];
  const scheduler = new PlaybackAwareCommandScheduler<number>(async (command) => {
    sent.push(command);
  });
  await expect(scheduler.dispatch(1)).resolves.toBe(true);
  const cancelled = scheduler.dispatch(2);
  scheduler.interrupt();
  scheduler.interrupt();
  scheduler.playbackDone();
  await expect(cancelled).resolves.toBe(false);
  await expect(scheduler.dispatch(3)).resolves.toBe(true);
  expect(sent).toEqual([1, 3]);
  scheduler.playbackDone();
  expect(scheduler.hasPendingPlayback).toBe(false);
});

test("intermittent transport error with queued commands preserves FIFO and recovery", async () => {
  const sent: number[] = [];
  const scheduler = new PlaybackAwareCommandScheduler<number>(async (command) => {
    if (command === 2) throw new Error("injected transport outage");
    sent.push(command);
  });
  await expect(scheduler.dispatch(1)).resolves.toBe(true);
  const failed = scheduler.dispatch(2);
  const recovered = scheduler.dispatch(3);
  scheduler.playbackDone();
  await expect(failed).rejects.toThrow("injected transport outage");
  await expect(recovered).resolves.toBe(true);
  expect(sent).toEqual([1, 3]);
  scheduler.playbackDone();
  expect(scheduler.hasPendingPlayback).toBe(false);
});
