// Deterministic timeline engine.
// One flat sequence of steps across all chapters. Position is a continuous
// time t in [0, 1] over the whole tour. stateAt(t) is a pure function:
// seeking to t always produces the same state. Playback only advances t.

import { CHAPTERS, allSteps } from "../data/chapters.js";

const STEPS = allSteps();

// Precompute cumulative weights -> segment bounds in [0,1].
const TOTAL_WEIGHT = STEPS.reduce((sum, s) => sum + s.weight, 0);
const SEGMENTS = [];
{
  let acc = 0;
  for (const step of STEPS) {
    const start = acc / TOTAL_WEIGHT;
    acc += step.weight;
    const end = acc / TOTAL_WEIGHT;
    SEGMENTS.push({ step, start, end, index: SEGMENTS.length });
  }
}

export function stepCount() {
  return STEPS.length;
}

export function steps() {
  return STEPS.slice();
}

export function chapters() {
  return CHAPTERS;
}

export function totalWeight() {
  return TOTAL_WEIGHT;
}

// Locate the segment containing t. Clamps into range. At exact boundaries
// the later segment wins (end is exclusive of the previous step).
export function locate(t) {
  const x = Math.min(1, Math.max(0, t));
  if (SEGMENTS.length === 0) return null;
  if (x >= 1) {
    const last = SEGMENTS[SEGMENTS.length - 1];
    return { index: last.index, local: 1, segment: last, step: last.step };
  }
  // Binary search for segment where start <= x < end.
  let lo = 0;
  let hi = SEGMENTS.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (SEGMENTS[mid].start <= x) lo = mid;
    else hi = mid - 1;
  }
  const seg = SEGMENTS[lo];
  const span = seg.end - seg.start;
  const local = span > 0 ? (x - seg.start) / span : 1;
  return { index: seg.index, local, segment: seg, step: seg.step };
}

// Pure state for a position. Identical t always yields an identical object
// graph (compared field by field via shallowEqualState).
export function stateAt(t) {
  const loc = locate(t);
  if (!loc) {
    return {
      t: 0,
      stepIndex: -1,
      stepId: null,
      chapterId: null,
      chapterNumber: 0,
      chapterStepIndex: -1,
      chapterStepCount: 0,
      local: 0,
      playing: false,
      focus: [],
      edges: [],
      kind: "control",
      detailLevel: 0,
      title: "",
      body: "",
    };
  }
  const { step, index, local } = loc;
  const chapterIndex = CHAPTERS.findIndex((c) => c.id === step.chapterId);
  const chapter = CHAPTERS[chapterIndex];
  return {
    t: Math.min(1, Math.max(0, t)),
    stepIndex: index,
    stepId: step.id,
    chapterId: step.chapterId,
    chapterNumber: step.chapterNumber,
    chapterStepIndex: step.stepIndexInChapter,
    chapterStepCount: chapter.steps.length,
    local,
    playing: false,
    focus: step.focus.slice(),
    edges: (step.edges ?? []).map((e) => ({ ...e })),
    kind: step.kind,
    detailLevel: chapter.detailHint,
    title: step.title,
    body: step.body,
  };
}

// Start time of a step index.
export function timeOfStep(index) {
  if (index <= 0) return 0;
  if (index >= SEGMENTS.length) return 1;
  return SEGMENTS[index].start;
}

export function timeOfChapter(chapterId) {
  const idx = STEPS.findIndex((s) => s.chapterId === chapterId);
  if (idx < 0) return 0;
  return timeOfStep(idx);
}

export function firstStepIndexOfChapter(chapterId) {
  return STEPS.findIndex((s) => s.chapterId === chapterId);
}

export function lastStepIndexOfChapter(chapterId) {
  for (let i = STEPS.length - 1; i >= 0; i--) {
    if (STEPS[i].chapterId === chapterId) return i;
  }
  return -1;
}

// Mutable player. All transitions go through explicit methods so pause,
// resume and seek are observable and testable.
export class TimelinePlayer {
  constructor({ playing = false, speed = 1, detailLevel = null } = {}) {
    this.t = 0;
    this.playing = Boolean(playing);
    this.speed = speed;
    this._explicitDetail = detailLevel;
  }

  state() {
    const base = stateAt(this.t);
    base.playing = this.playing;
    if (this._explicitDetail !== null && this._explicitDetail !== undefined) {
      base.detailLevel = this._explicitDetail;
    }
    return base;
  }

  play() {
    if (this.t >= 1) this.t = 0;
    this.playing = true;
    return this.state();
  }

  pause() {
    this.playing = false;
    return this.state();
  }

  toggle() {
    return this.playing ? this.pause() : this.play();
  }

  reset() {
    this.t = 0;
    this.playing = false;
    return this.state();
  }

  // Deterministic seek. Same t always yields the same state fields.
  seek(t) {
    this.t = Math.min(1, Math.max(0, t));
    return this.state();
  }

  seekToStep(index) {
    const clamped = Math.min(stepCount() - 1, Math.max(0, index));
    // Sit in the middle of the step so scrubbing to a step boundary from
    // either side lands on the same visible step.
    const seg = SEGMENTS[clamped];
    this.t = (seg.start + seg.end) / 2;
    return this.state();
  }

  nextStep() {
    const cur = locate(this.t);
    const next = cur ? Math.min(stepCount() - 1, cur.index + 1) : 0;
    return this.seekToStep(next);
  }

  prevStep() {
    const cur = locate(this.t);
    const prev = cur ? Math.max(0, cur.index - 1) : 0;
    return this.seekToStep(prev);
  }

  goToChapter(chapterId) {
    const idx = firstStepIndexOfChapter(chapterId);
    if (idx >= 0) this.seekToStep(idx);
    return this.state();
  }

  setSpeed(speed) {
    this.speed = Math.min(4, Math.max(0.25, speed));
    return this.state();
  }

  setDetailLevel(level) {
    this._explicitDetail = Math.min(2, Math.max(0, level));
    return this.state();
  }

  // Advance explanatory time. dtSeconds is wall time; the result is
  // deterministic given (t, dtSeconds, speed).
  advance(dtSeconds, { durationSeconds = 60 } = {}) {
    if (!this.playing) return this.state();
    const d = Math.max(0, dtSeconds);
    this.t = Math.min(1, this.t + (d * this.speed) / durationSeconds);
    if (this.t >= 1) {
      this.t = 1;
      this.playing = false;
    }
    return this.state();
  }
}

export function shallowEqualState(a, b) {
  const keys = ["t", "stepIndex", "stepId", "chapterId", "chapterNumber",
    "chapterStepIndex", "chapterStepCount", "kind", "detailLevel", "title", "playing"];
  for (const k of keys) {
    if (a[k] !== b[k]) return false;
  }
  if (Math.abs(a.local - b.local) > 1e-12) return false;
  if (a.focus.length !== b.focus.length) return false;
  for (let i = 0; i < a.focus.length; i++) {
    if (a.focus[i] !== b.focus[i]) return false;
  }
  if (a.edges.length !== b.edges.length) return false;
  for (let i = 0; i < a.edges.length; i++) {
    if (a.edges[i].edge !== b.edges[i].edge) return false;
    if (Boolean(a.edges[i].animate) !== Boolean(b.edges[i].animate)) return false;
  }
  return true;
}
