import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import test from "node:test";

// Exercise the ACTUAL JS embedded into Owner Lab, not a reimplementation.
// The selector is deliberately pure; isolate its compiled function, leaving
// the UI's DOM/provider side effects out of this low-level media contract.
const source = fs.readFileSync(new URL("../dist/app.js", import.meta.url), "utf8");
const from = source.indexOf("const selectPlayoutTimestamp = ");
const to = source.indexOf("const mediaElementAvSyncFallback = ", from);
assert.ok(from >= 0 && to > from, "RT0 RTP selector must exist in shipped app.js");
const sandbox = {};
vm.runInNewContext(source.slice(from, to)
  + "\nthis.select = selectPlayoutTimestamp; this.requireProgress = requireAdvancingPlayoutClock;", sandbox);
const select = sandbox.select;
const requireProgress = sandbox.requireProgress;

const report = (...items) => new Map(items.map((s) => [s.id, { type: "inbound-rtp", kind: "audio", ...s }]));

test("one live RTP stream requires advancing packets for every subsequent sample", () => {
  const first = select(report({ id: "rtp1", packetsReceived: 10, estimatedPlayoutTimestamp: 1200 }), "audio", null);
  assert.equal(first.timestamp, 1200);
  const stalled = select(report({ id: "rtp1", packetsReceived: 10, estimatedPlayoutTimestamp: 1200 }), "audio", first.packetCounts);
  assert.equal(stalled.timestamp, null);
  assert.equal(stalled.issue, "no_unique_active_stream");
  const advanced = select(report({ id: "rtp1", packetsReceived: 11, estimatedPlayoutTimestamp: 1250 }), "audio", stalled.packetCounts);
  assert.equal(advanced.timestamp, 1250);
});


test("advancing RTP packets with a frozen playout clock cannot prove another A/V sample", () => {
  const initial = select(report({ id: "audio", packetsReceived: 10, estimatedPlayoutTimestamp: 5000 }), "audio", null);
  assert.equal(requireProgress(initial, null, "audio").timestamp, 5000);
  const morePackets = select(report({ id: "audio", packetsReceived: 20, estimatedPlayoutTimestamp: 5000 }), "audio", initial.packetCounts);
  assert.equal(morePackets.timestamp, 5000, "packet counter by itself looks active");
  const frozenClock = requireProgress(morePackets, 5000, "audio");
  assert.equal(frozenClock.timestamp, null, "strict A/V sample must be rejected");
  assert.equal(frozenClock.issue, "no_unique_active_stream");
  assert.match(frozenClock.diagnostic, /playout clock stalled/);
  const next = select(report({ id: "audio", packetsReceived: 30, estimatedPlayoutTimestamp: 5150 }), "audio", morePackets.packetCounts);
  assert.equal(requireProgress(next, 5000, "audio").timestamp, 5150);
});

test("RTP clock resetting backwards fails closed even with newer packets", () => {
  const first = select(report({ id: "video", kind: "video", packetsReceived: 14, estimatedPlayoutTimestamp: 9000 }), "video", null);
  const rollback = select(report({ id: "video", kind: "video", packetsReceived: 23, estimatedPlayoutTimestamp: 8900 }), "video", first.packetCounts);
  const rejected = requireProgress(rollback, 9000, "video");
  assert.equal(rejected.timestamp, null);
  assert.equal(rejected.issue, "no_unique_active_stream");
});

test("a switched RTP identifier cannot reuse stale timing as verified activity", () => {
  const first = select(report({ id: "old", packetsReceived: 5, estimatedPlayoutTimestamp: 2000 }), "audio", null);
  const switched = select(report({ id: "new", packetsReceived: 1, estimatedPlayoutTimestamp: 2000 }), "audio", first.packetCounts);
  assert.equal(switched.timestamp, null);
  assert.equal(switched.issue, "no_unique_active_stream");
});

test("missing RTP timestamp and competing advancing streams cannot prove sync", () => {
  const missing = select(report({ id: "x", packetsReceived: 4 }), "audio", null);
  assert.equal(missing.timestamp, null);
  assert.equal(missing.issue, "sender_report_timing_unavailable");
  const multi = report(
    { id: "a", packetsReceived: 9, estimatedPlayoutTimestamp: 1000 },
    { id: "b", packetsReceived: 9, estimatedPlayoutTimestamp: 1200 },
  );
  const initial = select(multi, "audio", null);
  assert.equal(initial.timestamp, null);
  assert.equal(initial.issue, "ambiguous_streams");
  const previous = new Map([["a", 8], ["b", 8]]);
  const ambiguous = select(multi, "audio", previous);
  assert.equal(ambiguous.timestamp, null);
  assert.equal(ambiguous.issue, "no_unique_active_stream");
});
