// Data-driven logical system map. Routes and highlights explain relationships;
// they do not represent die layouts, live scheduling, or hardware telemetry.

import { EDGES, NODES } from "../data/graph.js";

const NS = "http://www.w3.org/2000/svg";
const WIDTH = 1600;
const HEIGHT = 1000;

const ROOT_SUMMARIES = {
  host: ["8 cores · 16 SMT contexts", "Window · simulation/render · audio threads"],
  sysram: ["30.5 GiB visible", "Nominal 32 GB LPDDR5x-6400 inferred"],
  pcie: ["PCIe 4.0 ×8", "Maximum host-to-device link"],
  dGPU: ["24 streaming multiprocessors", "3,072 CUDA cores · 8 GB GDDR6"],
  vram: ["8 GB GDDR6", "Device-local persistent assets"],
  "gpu-l2": ["32 MiB", "Shared device cache"],
  present: ["RTX render → Wayland / Mutter → Radeon 780M", "2880 × 1800 · 120 Hz internal panel"],
  igpu: ["AMD Radeon 780M", "Display controller for the internal panel"],
};

const VIEW_LABELS = { system: "Whole system", cpu: "CPU", gpu: "GPU" };

function svgEl(name, attrs = {}, parent = null) {
  const node = document.createElementNS(NS, name);
  for (const [key, value] of Object.entries(attrs)) {
    if (value !== null && value !== undefined) node.setAttribute(key, String(value));
  }
  if (parent) parent.appendChild(node);
  return node;
}

function text(parent, x, y, value, className, attrs = {}) {
  const node = svgEl("text", { x, y, class: className, ...attrs }, parent);
  node.textContent = value;
  return node;
}

function edgeStyle(edge, nodesById) {
  const from = nodesById.get(edge.from);
  const to = nodesById.get(edge.to);
  if (from?.domain === "display" || to?.domain === "display") {
    return { kind: "display", domain: "display", layer: "display" };
  }
  if (edge.kind === "sync") return { kind: "sync", domain: "sync", layer: "compute" };
  if (edge.kind === "data") return { kind: "data", domain: "memory", layer: "memory" };
  const domain = from?.domain === "gpu" || to?.domain === "gpu" ? "gpu" : "cpu";
  return { kind: "control", domain, layer: "compute" };
}

function center(node) {
  return { x: node.x + node.w * 0.5, y: node.y + node.h * 0.5 };
}

function connector(from, to) {
  const a = center(from);
  const b = center(to);
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  if (Math.abs(dx) >= Math.abs(dy)) {
    const dir = Math.sign(dx) || 1;
    const start = { x: a.x + dir * (from.w * 0.5), y: a.y };
    const end = { x: b.x - dir * (to.w * 0.5), y: b.y };
    const bend = Math.max(24, Math.abs(end.x - start.x) * 0.38);
    return {
      d: `M ${start.x} ${start.y} C ${start.x + dir * bend} ${start.y}, ${end.x - dir * bend} ${end.y}, ${end.x} ${end.y}`,
      labelX: (start.x + end.x) * 0.5,
      labelY: (start.y + end.y) * 0.5 - 13,
    };
  }
  const dir = Math.sign(dy) || 1;
  const start = { x: a.x, y: a.y + dir * (from.h * 0.5) };
  const end = { x: b.x, y: b.y - dir * (to.h * 0.5) };
  const bend = Math.max(24, Math.abs(end.y - start.y) * 0.36);
  return {
    d: `M ${start.x} ${start.y} C ${start.x} ${start.y + dir * bend}, ${end.x} ${end.y - dir * bend}, ${end.x} ${end.y}`,
    labelX: (start.x + end.x) * 0.5,
    labelY: (start.y + end.y) * 0.5 - 12,
  };
}

function wrapWords(value, maxChars, maxLines = 2) {
  const lines = [];
  let line = "";
  for (const word of value.split(/\s+/)) {
    const next = line ? `${line} ${word}` : word;
    if (next.length > maxChars && line) {
      lines.push(line);
      line = word;
    } else line = next;
  }
  if (line) lines.push(line);
  if (lines.length <= maxLines) return lines;
  const kept = lines.slice(0, maxLines);
  kept[maxLines - 1] = kept[maxLines - 1].replace(/[.,;:]?$/, "…");
  return kept;
}

function edgeKindName(kind) {
  return kind === "data" ? "Data transfer" : kind === "sync" ? "Synchronization" : "Control flow";
}

export class SystemMap {
  constructor(svg, { onSelect = null } = {}) {
    this.svg = svg;
    this.onSelect = onSelect;
    this.zoom = 1;
    this.panX = 0;
    this.panY = 0;
    this.selectedId = null;
    this.nodes = new Map();
    this.nodeData = new Map(NODES.map((node) => [node.id, node]));
    this.nodeElements = new Map();
    this.edgeElements = new Map();
    this.rootSummaries = new Map();
    this.detailLevel = 1;
    this.svg.setAttribute("role", "group");
    this.svg.setAttribute("viewBox", `0 0 ${WIDTH} ${HEIGHT}`);
    this.svg.setAttribute("preserveAspectRatio", "xMidYMid meet");
    this.svg.classList.add("map-svg");
    this._build();
    this._bindPanZoom();
    this.setView("system");
    this.setDetailLevel(this.detailLevel);
  }

  _build() {
    this.svg.innerHTML = "";
    const defs = svgEl("defs", {}, this.svg);
    const grid = svgEl("pattern", { id: "map-grid", width: 32, height: 32, patternUnits: "userSpaceOnUse" }, defs);
    svgEl("path", { d: "M 32 0 L 0 0 0 32", class: "map-grid-path" }, grid);
    const glow = svgEl("filter", { id: "focus-glow", x: "-35%", y: "-35%", width: "170%", height: "170%" }, defs);
    svgEl("feGaussianBlur", { stdDeviation: 4, result: "blur" }, glow);
    const merge = svgEl("feMerge", {}, glow);
    svgEl("feMergeNode", { in: "blur" }, merge);
    svgEl("feMergeNode", { in: "SourceGraphic" }, merge);
    const vignette = svgEl("radialGradient", { id: "map-vignette" }, defs);
    svgEl("stop", { offset: "0%", "stop-color": "#28504a", "stop-opacity": ".12" }, vignette);
    svgEl("stop", { offset: "100%", "stop-color": "#071316", "stop-opacity": ".48" }, vignette);
    this._addArrow(defs, "arrow-control", "#79cdbb");
    this._addArrow(defs, "arrow-data", "#e7bb76");
    this._addArrow(defs, "arrow-sync", "#bf9def");
    this._addArrow(defs, "arrow-display", "#8acdf8");

    svgEl("rect", { x: 0, y: 0, width: WIDTH, height: HEIGHT, class: "map-background" }, this.svg);
    svgEl("rect", { x: 0, y: 0, width: WIDTH, height: HEIGHT, fill: "url(#map-grid)", class: "map-grid" }, this.svg);
    svgEl("rect", { x: 0, y: 0, width: WIDTH, height: HEIGHT, class: "map-vignette" }, this.svg);
    this.root = svgEl("g", { class: "map-world" }, this.svg);
    this.edgeRoot = svgEl("g", { class: "map-edges", "aria-hidden": "true" }, this.root);
    this.nodeRoot = svgEl("g", { class: "map-nodes" }, this.root);
    this._drawEdges();
    const drawable = NODES.filter((node) => !node.hidden && node.w > 0 && node.h > 0);
    for (const node of drawable) this._drawNode(node, drawable);
  }

  _addArrow(parent, id, color) {
    const marker = svgEl("marker", { id, viewBox: "0 0 10 10", refX: 8, refY: 5, markerWidth: 7, markerHeight: 7, orient: "auto-start-reverse" }, parent);
    svgEl("path", { d: "M 0 1 L 9 5 L 0 9 z", fill: color }, marker);
  }

  _drawEdges() {
    for (const edge of EDGES) {
      const from = this.nodeData.get(edge.from);
      const to = this.nodeData.get(edge.to);
      if (!from || !to || from.hidden || to.hidden || from.w <= 0 || from.h <= 0 || to.w <= 0 || to.h <= 0) continue;
      const style = edgeStyle(edge, this.nodeData);
      const route = connector(from, to);
      const group = svgEl("g", {
        class: `map-edge kind-${style.kind} domain-${style.domain} layer-${style.layer}`,
        "data-edge": edge.id,
      }, this.edgeRoot);
      svgEl("path", { d: route.d, class: "edge-track" }, group);
      svgEl("path", { d: route.d, class: "edge-flow" }, group);
      const label = edge.label.length > 48 ? `${edge.label.slice(0, 45)}…` : edge.label;
      text(group, route.labelX, route.labelY, label, "edge-label", { "text-anchor": "middle" });
      this.edgeElements.set(edge.id, { group, edge, style });
    }
  }

  _drawNode(node, allNodes) {
    const hasChildren = allNodes.some((candidate) => candidate.group === node.id);
    const isContainer = node.level === 0 || hasChildren;
    const domain = node.inactive ? "inactive" : node.domain;
    const layer = node.domain === "memory" ? "memory" : node.domain === "display" ? "display" : "compute";
    const group = svgEl("g", {
      class: `map-node domain-${domain} layer-${layer}${isContainer ? " node-container" : ""}`,
      "data-node": node.id,
      tabindex: 0,
      role: "button",
      "aria-label": `${node.label}. ${node.detail ?? ""}`.trim(),
      "aria-pressed": "false",
    }, this.nodeRoot);
    svgEl("rect", { x: node.x, y: node.y, width: node.w, height: node.h, rx: isContainer ? 12 : 7, class: "node-rect" }, group);
    svgEl("path", { d: `M ${node.x + 16} ${node.y + 8} h 34`, class: "node-accent" }, group);

    if (isContainer) {
      text(group, node.x + 16, node.y + 28, node.label, "node-label");
      const summary = ROOT_SUMMARIES[node.id];
      if (node.level === 0 && summary) {
        const summaryGroup = svgEl("g", { class: "root-summary" }, group);
        const summaryY = node.h < 130 ? 59 : 70;
        if (summary[0]) text(summaryGroup, node.x + 17, node.y + summaryY, summary[0], "node-value");
        if (summary[1]) text(summaryGroup, node.x + 17, node.y + summaryY + 23, summary[1], "node-subtitle");
        if (node.id === "present") {
          svgEl("rect", { x: node.x + 17, y: node.y + node.h - 18, width: node.w - 34, height: 3, rx: 2, class: "presentation-track" }, summaryGroup);
        }
        this.rootSummaries.set(node.id, summaryGroup);
      } else if (node.level > 0 && node.h > 110 && !hasChildren) {
        const chipLabel = node.domain === "gpu" ? "GPU RESOURCE" : node.domain === "cpu" ? "CPU WORK" : "PERSISTENT / PER-FRAME";
        svgEl("rect", { x: node.x + 15, y: node.y + node.h - 28, width: 118, height: 17, rx: 5, class: "node-chip" }, group);
        text(group, node.x + 23, node.y + node.h - 16, chipLabel, "node-chip-text");
      }
    } else {
      const fontSize = node.h <= 45 ? 12 : node.w < 130 ? 11 : 13;
      const maxChars = Math.max(10, Math.floor((node.w - 25) / (fontSize * 0.59)));
      const lines = wrapWords(node.label, maxChars, 2);
      const lineHeight = fontSize + 3;
      const firstY = node.y + node.h * 0.53 - ((lines.length - 1) * lineHeight) * 0.5 + 4;
      lines.forEach((line, index) => text(group, node.x + 12, firstY + index * lineHeight, line, "node-label-small"));
      if (node.unused) svgEl("circle", { cx: node.x + node.w - 13, cy: node.y + 13, r: 3.5, class: "node-dot" }, group);
    }

    const title = svgEl("title", {}, group);
    title.textContent = `${node.label} — ${node.detail ?? ""}`;
    group.addEventListener("click", (event) => {
      event.stopPropagation();
      this.select(node.id);
    });
    group.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        event.stopPropagation();
        this.select(node.id);
      }
    });
    this.nodeElements.set(node.id, group);
    this.nodes.set(node.id, node);
  }

  setView(view) {
    const key = VIEW_LABELS[view] ? view : "system";
    this.svg.dataset.view = key;
    const label = document.querySelector("#view-name");
    if (label) label.textContent = VIEW_LABELS[key];
    document.querySelectorAll("[data-view]").forEach((button) => {
      const active = button.dataset.view === key;
      button.classList.toggle("is-active", active);
      button.setAttribute("aria-pressed", String(active));
    });
    if (key === "system") this.resetView();
    else {
      const center = key === "cpu" ? { x: 280, y: 300 } : { x: 1060, y: 300 };
      this.zoom = innerWidth <= 700 ? 1.55 : 1.12;
      this.panX = -(center.x - WIDTH / 2) * this.zoom;
      this.panY = -(center.y - HEIGHT / 2) * this.zoom;
      this.applyView();
    }
  }

  setDetailLevel(level) {
    this.detailLevel = Math.max(0, Math.min(2, Number(level) || 0));
    for (const [id, node] of this.nodes) {
      this.nodeElements.get(id)?.classList.toggle("is-hidden", node.level > this.detailLevel);
    }
    for (const { group, edge } of this.edgeElements.values()) {
      const visible = edge.level <= this.detailLevel
        && this.nodes.get(edge.from)?.level <= this.detailLevel
        && this.nodes.get(edge.to)?.level <= this.detailLevel;
      group.classList.toggle("is-hidden", !visible);
    }
    for (const summary of this.rootSummaries.values()) {
      summary.classList.toggle("is-visible", this.detailLevel === 0);
    }
  }

  setLayer(name, visible) {
    this.svg.classList.toggle(`hide-${name}`, !visible);
  }

  setMotion(enabled) {
    this.svg.classList.toggle("motion-off", !enabled);
  }

  setStep(state) {
    const focus = new Set(state.focus ?? []);
    const visibleFocus = [...focus].some((id) => this.nodes.get(id)?.level <= this.detailLevel);
    const activeEdges = new Set((state.edges ?? []).map(({ edge }) => edge));
    for (const [id, node] of this.nodes) {
      const group = this.nodeElements.get(id);
      const matches = focus.has(id);
      group?.classList.toggle("is-focused", matches);
      group?.classList.toggle("is-muted", visibleFocus && !matches && node.level <= this.detailLevel);
    }
    for (const [id, item] of this.edgeElements) item.group.classList.toggle("is-active", activeEdges.has(id));
  }

  select(id) {
    if (!this.nodes.has(id)) return;
    this.selectedId = id;
    for (const [nodeId, group] of this.nodeElements) {
      const selected = nodeId === id;
      group.classList.toggle("is-selected", selected);
      group.setAttribute("aria-pressed", String(selected));
    }
    this.onSelect?.(id);
  }

  clearSelection() {
    this.selectedId = null;
    for (const group of this.nodeElements.values()) {
      group.classList.remove("is-selected");
      group.setAttribute("aria-pressed", "false");
    }
    this.onSelect?.(null);
  }

  _bindPanZoom() {
    this.svg.addEventListener("wheel", (event) => {
      event.preventDefault();
      this.setZoom(this.zoom * (event.deltaY < 0 ? 1.12 : 0.89));
    }, { passive: false });
    let drag = null;
    this.svg.addEventListener("pointerdown", (event) => {
      if (event.target.closest?.(".map-node")) return;
      drag = { x: event.clientX, y: event.clientY, panX: this.panX, panY: this.panY };
      this.svg.classList.add("is-dragging");
      this.svg.setPointerCapture(event.pointerId);
    });
    this.svg.addEventListener("pointermove", (event) => {
      if (!drag) return;
      const rect = this.svg.getBoundingClientRect();
      this.panX = drag.panX + (event.clientX - drag.x) * WIDTH / Math.max(rect.width, 1);
      this.panY = drag.panY + (event.clientY - drag.y) * HEIGHT / Math.max(rect.height, 1);
      this.applyView();
    });
    const stopDrag = () => {
      drag = null;
      this.svg.classList.remove("is-dragging");
    };
    this.svg.addEventListener("pointerup", stopDrag);
    this.svg.addEventListener("pointercancel", stopDrag);
  }

  setZoom(zoom) {
    this.zoom = Math.max(0.7, Math.min(2.4, zoom));
    this.applyView();
  }

  resetView() {
    this.zoom = 1;
    this.panX = 0;
    this.panY = 0;
    this.applyView();
  }

  applyView() {
    this.root?.setAttribute("transform", `translate(${this.panX} ${this.panY}) translate(${WIDTH / 2} ${HEIGHT / 2}) scale(${this.zoom}) translate(${-WIDTH / 2} ${-HEIGHT / 2})`);
    const button = document.querySelector("#btn-zoom-reset");
    if (button) button.textContent = `${Math.round(this.zoom * 100)}%`;
  }
}

export { edgeKindName };
