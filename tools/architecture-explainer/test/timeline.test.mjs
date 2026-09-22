// Deterministic seeking and pause/resume behavior.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  TimelinePlayer,
  stateAt,
  shallowEqualState,
  stepCount,
  timeOfStep,
} from "../js/timeline.js";

test("seek is deterministic: same t yields the same state", () => {
  const samples = [0, 0.13, 0.33, 0.5, 0.66, 0.8, 0.95, 1];
  for (const t of samples) {
    const a = stateAt(t);
    const b = stateAt(t);
    assert.ok(shallowEqualState(a, b), `stateAt(${t}) not stable`);
    const p = new TimelinePlayer();
    p.seek(0.99);
    p.seek(t);
    const c = p.state();
    // t and step identity must match; playing flag may differ from pure state.
    assert.equal(c.stepId, a.stepId, `player step at ${t}`);
    assert.equal(c.chapterNumber, a.chapterNumber, `player chapter at ${t}`);
    assert.equal(c.kind, a.kind, `player kind at ${t}`);
    assert.deepEqual(c.focus, a.focus, `player focus at ${t}`);
  }
});

test("seeking away and back restores an identical state", () => {
  const p = new TimelinePlayer();
  p.seek(0.4);
  const first = p.state();
  p.seek(0.8);
  p.seek(0.4);
  const second = p.state();
  assert.ok(shallowEqualState({ ...first, t: 0.4 }, { ...second, t: 0.4 }));
});

test("scrubbing across step boundaries matches discrete steps", () => {
  // Middle of step 0 equals seekToStep(0).
  const p = new TimelinePlayer();
  p.seekToStep(0);
  const viaStep = p.state();
  p.seek(0);
  const atZero = p.state();
  assert.equal(viaStep.stepId, atZero.stepId);

  // A time inside step k's segment locates step k.
  for (let k = 0; k < stepCount(); k++) {
    const start = timeOfStep(k);
    const nextStart = k + 1 < stepCount() ? timeOfStep(k + 1) : 1.0000001;
    const mid = (start + Math.min(nextStart, 1)) / 2;
    const st = stateAt(Math.min(mid, 0.999999));
    const p2 = new TimelinePlayer();
    p2.seekToStep(k);
    assert.equal(st.stepId, p2.state().stepId, `step ${k} at mid time`);
  }
});

test("pause holds position and resume continues from it", () => {
  const p = new TimelinePlayer({ playing: true, speed: 1 });
  p.seek(0.25);
  const before = p.state();
  assert.equal(before.playing, true);

  p.pause();
  const paused = p.state();
  assert.equal(paused.playing, false);
  assert.equal(paused.t, before.t);

  // Time does not advance while paused.
  p.advance(5, { durationSeconds: 60 });
  assert.equal(p.t, before.t);

  p.play();
  const resumed = p.state();
  assert.equal(resumed.playing, true);
  assert.equal(resumed.t, before.t);

  p.advance(1, { durationSeconds: 60 });
  assert.ok(p.t > before.t, "t advances after resume");
});

test("advance is deterministic for a given dt and speed", () => {
  const a = new TimelinePlayer({ playing: true, speed: 1 });
  const b = new TimelinePlayer({ playing: true, speed: 1 });
  a.advance(0.7, { durationSeconds: 60 });
  b.advance(0.7, { durationSeconds: 60 });
  assert.equal(a.t, b.t);

  a.setSpeed(2);
  a.advance(0.7, { durationSeconds: 60 });
  b.setSpeed(1);
  b.advance(1.4, { durationSeconds: 60 });
  assert.ok(Math.abs(a.t - b.t) < 1e-12, "speed scales linearly");
});

test("advance stops at the end and clears playing", () => {
  const p = new TimelinePlayer({ playing: true });
  p.seek(0.999);
  p.advance(10, { durationSeconds: 60 });
  assert.equal(p.t, 1);
  assert.equal(p.playing, false);
});

test("play from the end restarts at zero", () => {
  const p = new TimelinePlayer();
  p.seek(1);
  p.pause();
  const st = p.play();
  assert.equal(st.playing, true);
  assert.equal(p.t, 0);
});

test("toggle flips playing", () => {
  const p = new TimelinePlayer();
  assert.equal(p.toggle().playing, true);
  assert.equal(p.toggle().playing, false);
});

test("reset returns to the start and pauses", () => {
  const p = new TimelinePlayer({ playing: true });
  p.seek(0.7);
  const st = p.reset();
  assert.equal(st.t, 0);
  assert.equal(st.playing, false);
  assert.equal(st.stepIndex, 0);
});

test("seek clamps outside [0,1]", () => {
  const p = new TimelinePlayer();
  p.seek(-1);
  assert.equal(p.t, 0);
  p.seek(2);
  assert.equal(p.t, 1);
});
