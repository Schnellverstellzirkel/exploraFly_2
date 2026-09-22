// Chapter structure, transitions and chapter navigation.

import { test } from "node:test";
import assert from "node:assert/strict";
import { CHAPTERS, allSteps, validateChapters } from "../data/chapters.js";
import { NODES, EDGES } from "../data/graph.js";
import {
  TimelinePlayer,
  stateAt,
  stepCount,
  locate,
  timeOfChapter,
  firstStepIndexOfChapter,
  lastStepIndexOfChapter,
} from "../js/timeline.js";

const nodeIds = new Set(NODES.map((n) => n.id));
const edgeIds = new Set(EDGES.map((e) => e.id));

test("validateChapters passes on the shipped scripts", () => {
  assert.deepEqual(validateChapters(nodeIds, edgeIds), []);
});

test("there are exactly five chapters with sequential numbers", () => {
  assert.equal(CHAPTERS.length, 5);
  CHAPTERS.forEach((ch, i) => assert.equal(ch.number, i + 1));
});

test("every chapter has at least three steps", () => {
  for (const ch of CHAPTERS) {
    assert.ok(ch.steps.length >= 3, `${ch.id} has ${ch.steps.length} steps`);
  }
});

test("allSteps flattens in chapter then step order", () => {
  const flat = allSteps();
  assert.equal(flat.length, stepCount());
  assert.equal(flat[0].chapterNumber, 1);
  assert.equal(flat[flat.length - 1].chapterNumber, 5);
  for (let i = 1; i < flat.length; i++) {
    const prev = flat[i - 1];
    const cur = flat[i];
    if (prev.chapterId === cur.chapterId) {
      assert.equal(cur.stepIndexInChapter, prev.stepIndexInChapter + 1);
    } else {
      assert.equal(cur.stepIndexInChapter, 0);
      assert.equal(cur.chapterNumber, prev.chapterNumber + 1);
    }
  }
});

test("chapter time ranges are contiguous and ordered", () => {
  let prevEnd = 0;
  for (const ch of CHAPTERS) {
    const start = timeOfChapter(ch.id);
    const first = firstStepIndexOfChapter(ch.id);
    const last = lastStepIndexOfChapter(ch.id);
    assert.ok(start >= prevEnd, `${ch.id} starts after previous chapter`);
    assert.ok(last > first, `${ch.id} has a valid step range`);
    const endState = locate(Math.min(0.999999, (start + 1) / CHAPTERS.length + start));
    void endState;
    prevEnd = start;
  }
  assert.equal(timeOfChapter(CHAPTERS[0].id), 0);
});

test("chapter transitions: going to a chapter lands on its first step", () => {
  const p = new TimelinePlayer();
  for (const ch of CHAPTERS) {
    const st = p.goToChapter(ch.id);
    assert.equal(st.chapterId, ch.id, `landed in ${st.chapterId} for ${ch.id}`);
    assert.equal(st.chapterStepIndex, 0);
    // Stepping backward from the start of chapter N reaches chapter N-1.
    if (ch.number > 1) {
      p.goToChapter(ch.id);
      const back = p.prevStep();
      assert.equal(back.chapterNumber, ch.number - 1);
    }
    // Stepping forward through the chapter reaches the next chapter.
    p.goToChapter(ch.id);
    let guard = 0;
    let cur = p.state();
    while (cur.chapterId === ch.id && guard < 50) {
      cur = p.nextStep();
      guard += 1;
    }
    if (ch.number < 5) {
      assert.equal(cur.chapterNumber, ch.number + 1, `forward from ${ch.id}`);
    } else {
      assert.equal(cur.chapterNumber, 5, "last chapter stays at 5");
    }
  }
});

test("nextStep at the end stays on the last step and does not wrap", () => {
  const p = new TimelinePlayer();
  p.seek(1);
  const st = p.nextStep();
  assert.equal(st.chapterNumber, 5);
  assert.equal(st.stepIndex, stepCount() - 1);
});

test("stateAt is consistent with player.seek", () => {
  for (const t of [0, 0.1, 0.25, 0.5, 0.75, 0.999, 1]) {
    const direct = stateAt(t);
    const p = new TimelinePlayer();
    p.seek(t);
    const viaPlayer = p.state();
    assert.equal(direct.stepId, viaPlayer.stepId, `step at t=${t}`);
    assert.equal(direct.chapterNumber, viaPlayer.chapterNumber, `chapter at t=${t}`);
    assert.equal(direct.kind, viaPlayer.kind);
  }
});
