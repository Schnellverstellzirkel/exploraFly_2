// Resource-size arithmetic mirrored from the engine constants.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CONST,
  uniformLayout,
  frameBufferLayout,
  vegetationOutputBytes,
  smTotals,
  gddr6BandwidthGBs,
  factCards,
} from "../data/facts.js";

test("uniform block layout totals 1,824 bytes", () => {
  const u = uniformLayout();
  assert.equal(u.matrices, 25); // viewProj + invViewProj + 23 nodes
  assert.equal(u.matrixBytes, 25 * 64);
  assert.equal(u.tailVec4, 14);
  assert.equal(u.tailBytes, 224);
  assert.equal(u.total, 1824);
  assert.equal(CONST.UBO_BYTES, 1824);
  assert.equal(u.total, CONST.UBO_BYTES);
  assert.equal(u.total % 16, 0);
});

test("FRAME_BYTES composition matches engine offsets", () => {
  const f = frameBufferLayout();
  // Offsets from engine/src/plane/mod.rs:16-26.
  assert.equal(f.uboOffset, 0);
  assert.equal(f.terrainOffset, CONST.UBO_BYTES);
  assert.equal(f.terrainBytes, 1024 * 20);
  assert.equal(f.vegetationOffset, 1824 + 20480);
  assert.equal(f.vegetationBytes, 4096 * 16);
  assert.equal(f.canopyOffset, 22304 + 65536);
  assert.equal(f.canopyBytes, 16384 * 16);
  assert.equal(f.total, 1824 + 20480 + 65536 + 262144);
  assert.equal(f.total, 349984);
  // Additivity: each region sits exactly after the previous one.
  assert.equal(f.terrainOffset + f.terrainBytes, f.vegetationOffset);
  assert.equal(f.vegetationOffset + f.vegetationBytes, f.canopyOffset);
  assert.equal(f.canopyOffset + f.canopyBytes, f.total);
});

test("vegetation GPU-cull output size", () => {
  // DrawIndirectCommand header (16 B) + 262144 visible entries x 16 bytes.
  assert.equal(vegetationOutputBytes(), 16 + 262144 * 16);
  assert.equal(vegetationOutputBytes(), 4194320);
  assert.equal(CONST.VEGETATION_VISIBLE_CAPACITY, 262144);
});

test("SM-derived totals for 24 SMs", () => {
  const t = smTotals(24);
  assert.equal(t.cudaCores, 3072);
  assert.equal(t.rtCores, 24);
  assert.equal(t.tensorCores, 96);
  assert.equal(t.maxResidentWarps, 1152);
});

test("theoretical GDDR6 bandwidth is labeled arithmetic", () => {
  const band = gddr6BandwidthGBs(8.001, 128);
  assert.ok(Math.abs(band - 256.032) < 1e-9);
});

test("fact cards: UBO and FRAME_BYTES cards state the right numbers", () => {
  const cards = factCards();
  const byId = Object.fromEntries(cards.map((c) => [c.id, c]));
  assert.ok(byId["fact-ubo"]);
  assert.match(byId["fact-ubo"].value, /1,824/);
  assert.ok(byId["fact-frame-bytes"]);
  assert.match(byId["fact-frame-bytes"].value, /349,984/);
  assert.equal(byId["fact-frame-bytes"].kind, "calculation");
  assert.ok(byId["fact-frame-bytes"].calculation.includes("= 349984"));

  // Every card carries provenance with an access date.
  for (const c of cards) {
    assert.ok(c.provenance.length > 0, `${c.id} has provenance`);
    for (const p of c.provenance) {
      assert.ok(p.accessed, `${c.id} provenance has access date`);
      assert.ok(p.ref, `${c.id} provenance has a reference`);
      assert.ok("limits" in p, `${c.id} provenance has limits field`);
    }
  }
});

test("simulation frequency constant", () => {
  assert.equal(CONST.SIM_HZ, 144);
  assert.equal(CONST.GPU_STAMPS_PER_FRAME, 11);
  assert.equal(CONST.RT_INSTANCE_BYTES, 64);
  assert.equal(CONST.DEDICATE_ABOVE_BYTES, 16 * 1024 * 1024);
});
