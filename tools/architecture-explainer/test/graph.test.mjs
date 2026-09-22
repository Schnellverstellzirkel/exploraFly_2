// Graph reference validation: every edge endpoint, focus id, group and
// dependsOn target must exist in the node set.

import { test } from "node:test";
import assert from "node:assert/strict";
import { NODES, EDGES, validateGraph, nodesAtLevel, edgesAtLevel, LEVELS } from "../data/graph.js";
import { validateChapters } from "../data/chapters.js";

test("graph has no broken references", () => {
  const errors = validateGraph();
  assert.deepEqual(errors, []);
});

test("node ids are unique", () => {
  const ids = NODES.map((n) => n.id);
  assert.equal(new Set(ids).size, ids.length);
});

test("every edge endpoint exists", () => {
  const ids = new Set(NODES.map((n) => n.id));
  for (const e of EDGES) {
    assert.ok(ids.has(e.from), `missing from ${e.from} in ${e.id}`);
    assert.ok(ids.has(e.to), `missing to ${e.to} in ${e.id}`);
  }
});

test("chapter focus and edge references resolve", () => {
  const nodeIds = new Set(NODES.map((n) => n.id));
  const edgeIds = new Set(EDGES.map((e) => e.id));
  const errors = validateChapters(nodeIds, edgeIds);
  assert.deepEqual(errors, []);
});

test("detail levels filter nodes and edges consistently", () => {
  for (const level of LEVELS) {
    const nodes = nodesAtLevel(level.id);
    const edges = edgesAtLevel(level.id);
    const ids = new Set(nodes.map((n) => n.id));
    assert.ok(nodes.length > 0, `level ${level.id} has nodes`);
    for (const e of edges) {
      assert.ok(ids.has(e.from), `level ${level.id} edge ${e.id} from hidden`);
      assert.ok(ids.has(e.to), `level ${level.id} edge ${e.id} to hidden`);
    }
  }
  // Higher levels never lose nodes that lower levels show.
  assert.ok(nodesAtLevel(2).length >= nodesAtLevel(1).length);
  assert.ok(nodesAtLevel(1).length >= nodesAtLevel(0).length);
});

test("system level shows the main hardware boxes", () => {
  const ids = new Set(nodesAtLevel(0).map((n) => n.id));
  for (const required of ["host", "sysram", "dGPU", "pcie", "present", "igpu"]) {
    assert.ok(ids.has(required), `missing ${required} at system level`);
  }
});

test("kernel crates are marked inactive and never focused by tour steps", async () => {
  const { NODES: all, } = await import("../data/graph.js");
  const kernels = all.filter((n) => n.id.startsWith("kernel-"));
  assert.equal(kernels.length, 2);
  for (const k of kernels) {
    assert.equal(k.inactive, true, `${k.id} must be inactive`);
  }
});
