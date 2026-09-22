import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const html = await readFile(new URL("../index.html", import.meta.url), "utf8");
const main = await readFile(new URL("../js/main.js", import.meta.url), "utf8");
const diagram = await readFile(new URL("../js/diagram.js", import.meta.url), "utf8");

test("the page includes every fixed element used by its controllers", () => {
  const selectors = [...main.matchAll(/qs\("#([A-Za-z][A-Za-z0-9_-]*)/g), ...diagram.matchAll(/querySelector\("#([A-Za-z][A-Za-z0-9_-]*)/g)];
  const ids = [...new Set(selectors.map(([, id]) => id))];
  for (const id of ids) assert.ok(html.includes(`id="${id}"`), `missing #${id}`);
});

test("the guide loads its local stylesheet and module controller", () => {
  assert.match(html, /href="style\.css"/);
  assert.match(html, /src="js\/main\.js"/);
});
