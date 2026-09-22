// Guided architecture tour backed by a documented snapshot. The page is
// intentionally disconnected from the running game and live hardware.

import { CHAPTERS } from "../data/chapters.js";
import { EDGES, LEVELS, nodeById } from "../data/graph.js";
import { DOMAIN_LABELS, HARDWARE, SNAPSHOT } from "../data/hardware.js";
import { SystemMap, edgeKindName } from "./diagram.js";
import { TimelinePlayer, stepCount } from "./timeline.js";

const qs = (selector, root = document) => root.querySelector(selector);
const qsa = (selector, root = document) => [...root.querySelectorAll(selector)];
const escapeHtml = (value) => String(value)
  .replaceAll("&", "&amp;")
  .replaceAll("<", "&lt;")
  .replaceAll(">", "&gt;")
  .replaceAll('"', "&quot;")
  .replaceAll("'", "&#39;");

const player = new TimelinePlayer();
const map = new SystemMap(qs("#diagram"), { onSelect: renderSelection });
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
const totalSteps = stepCount();
let detailPreference = null;
let rafId = null;
let previousFrameTime = null;

function chapterFor(state) {
  return CHAPTERS.find((chapter) => chapter.id === state.chapterId) ?? CHAPTERS[0];
}

function detailRequiredBy(state) {
  let required = 0;
  for (const id of state.focus ?? []) required = Math.max(required, nodeById(id)?.level ?? 0);
  for (const active of state.edges ?? []) {
    const edge = EDGES.find((candidate) => candidate.id === active.edge);
    if (!edge) continue;
    required = Math.max(required, edge.level, nodeById(edge.from)?.level ?? 0, nodeById(edge.to)?.level ?? 0);
  }
  return required;
}

function renderChapterNav(state) {
  const nav = qs("#chapter-nav");
  nav.innerHTML = CHAPTERS.map((chapter) => {
    const active = chapter.id === state.chapterId;
    const complete = chapter.number < state.chapterNumber;
    const percent = complete ? 100 : active
      ? ((state.chapterStepIndex + 1) / state.chapterStepCount) * 100
      : 0;
    return `
      <button type="button" class="chapter-button${active ? " is-current" : ""}${complete ? " is-complete" : ""}"
        data-chapter="${escapeHtml(chapter.id)}" aria-current="${active ? "step" : "false"}"
        aria-label="Go to chapter ${chapter.number}: ${escapeHtml(chapter.title)}">
        <span class="chapter-number">${String(chapter.number).padStart(2, "0")}</span>
        <span class="chapter-copy">
          <span class="chapter-name">${escapeHtml(chapter.title)}</span>
          <span class="chapter-meta"><span>${chapter.steps.length} stops</span><span>${complete ? "Done" : active ? "In progress" : ""}</span></span>
          <span class="chapter-progress" aria-hidden="true"><span style="width:${percent}%"></span></span>
        </span>
      </button>`;
  }).join("");
}

function renderFocus(state) {
  const entries = (state.focus ?? []).map((id) => nodeById(id)).filter(Boolean);
  qs("#focus-list").innerHTML = entries.map((node) =>
    `<button type="button" class="focus-chip" data-focus-node="${escapeHtml(node.id)}" title="Show ${escapeHtml(node.label)} in the map">${escapeHtml(node.label)}</button>`
  ).join("") || '<span class="focus-empty">This stop describes overall system behavior.</span>';
}

function renderReadableRoute(state) {
  const activeEdges = (state.edges ?? []).map(({ edge: id }) => EDGES.find((edge) => edge.id === id)).filter(Boolean);
  const focusedNodes = (state.focus ?? []).map((id) => nodeById(id)).filter(Boolean);
  const focusList = focusedNodes.length
    ? `<ul>${focusedNodes.map((node) => `<li>${escapeHtml(node.label)}</li>`).join("")}</ul>`
    : "<p>No single component is highlighted at this stop.</p>";
  const edgeList = activeEdges.length
    ? `<ul>${activeEdges.map((edge) => `<li>${escapeHtml(edgeKindName(edge.kind))}: ${escapeHtml(edge.label)}</li>`).join("")}</ul>`
    : "<p>No connection is singled out at this stop.</p>";
  qs("#map-summary").innerHTML = `<p>${escapeHtml(state.body)}</p><strong>Highlighted blocks</strong>${focusList}<strong>Active connections</strong>${edgeList}`;
}

function renderState(state = player.state()) {
  const chapter = chapterFor(state);
  const needed = detailRequiredBy(state);
  const desired = detailPreference ?? state.detailLevel;
  const detail = detailPreference === null ? Math.max(desired, needed) : detailPreference;

  qs("#chapter-mark").textContent = String(chapter.number).padStart(2, "0");
  qs("#chapter-kicker").textContent = `CHAPTER ${chapter.number}`;
  qs("#chapter-title").textContent = chapter.title;
  qs("#chapter-summary").textContent = chapter.summary;
  qs("#step-count").textContent = `${state.chapterStepIndex + 1} / ${state.chapterStepCount}`;
  qs("#step-title").textContent = state.title;
  qs("#step-body").textContent = state.body;

  const kind = state.kind ?? "control";
  const kindLabel = kind === "data" ? "DATA TRANSFER" : kind === "sync" ? "SYNCHRONIZATION" : "CONTROL FLOW";
  const kindElement = qs("#step-kind");
  kindElement.textContent = kindLabel;
  kindElement.className = `step-kind kind-${escapeHtml(kind)}`;
  qs("#detail-note").textContent = detail > desired
    ? "Expanded to show the highlighted blocks"
    : detail < needed
      ? "Increase detail to reveal the highlighted blocks"
    : LEVELS.find((level) => level.id === detail)?.label ?? "";

  const position = state.stepIndex + 1;
  const percent = Math.round(state.t * 100);
  qs("#timeline-position").textContent = `${String(position).padStart(2, "0")} / ${String(totalSteps).padStart(2, "0")}`;
  qs("#timeline-percent").textContent = `${percent}%`;
  const scrubber = qs("#timeline-scrubber");
  scrubber.value = String(Math.round(state.t * Number(scrubber.max)));
  scrubber.style.setProperty("--progress", `${percent}%`);
  qs("#play-label").textContent = state.playing ? "Pause tour" : "Play tour";
  qs("#play-toggle").classList.toggle("is-playing", state.playing);
  qs("#play-toggle").setAttribute("aria-label", state.playing ? "Pause architecture tour" : "Play architecture tour");
  qs("#play-toggle .play-icon").textContent = state.playing ? "Ⅱ" : "▶";
  qs("#detail-level").value = String(detail);

  renderChapterNav(state);
  renderFocus(state);
  renderReadableRoute(state);
  map.setDetailLevel(detail);
  map.setStep(state);
}

function renderSelection(id) {
  const panel = qs("#selection-panel");
  const selection = qs("#selection");
  if (!id) {
    panel.hidden = true;
    selection.innerHTML = "";
    return;
  }
  const node = nodeById(id);
  if (!node) return;
  panel.hidden = false;
  const resources = (node.resources ?? []).map((resource) => nodeById(resource)?.label ?? resource);
  const dependencies = (node.dependsOn ?? []).map((dep) => nodeById(dep)?.label ?? dep);
  const refs = node.codeRefs ?? [];
  selection.innerHTML = `
    <p class="selection-domain domain-${escapeHtml(node.domain ?? "inactive")}">${escapeHtml(DOMAIN_LABELS[node.domain] ?? node.domain ?? "Component")}</p>
    <h3 class="selection-title">${escapeHtml(node.label)}</h3>
    <p class="selection-detail">${escapeHtml(node.detail ?? "No additional detail is documented for this component.")}</p>
    ${node.unused ? '<p class="selection-flag">Present in the hardware; unused by this renderer.</p>' : ""}
    <details class="node-more">
      <summary>Connections and source evidence</summary>
      <dl>
        <div><dt>Resources</dt><dd>${escapeHtml(resources.join(", ") || "None listed")}</dd></div>
        <div><dt>Depends on</dt><dd>${escapeHtml(dependencies.join(", ") || "None listed")}</dd></div>
        <div><dt>Code / evidence</dt><dd>${refs.map((ref) => `<code>${escapeHtml(ref)}</code>`).join("<br>") || "None listed"}</dd></div>
      </dl>
    </details>`;
}

function renderHardwarePanel() {
  const h = HARDWARE;
  const rows = [
    ["Processor", `${h.cpu.name} (${h.cpu.arch})`],
    ["Cores / threads", `${h.cpu.cores} / ${h.cpu.smtContexts}`],
    ["CPU cache", `${h.cpu.l1iPerCoreKiB} KiB L1i + ${h.cpu.l1dPerCoreKiB} KiB L1d + ${h.cpu.l2PerCoreKiB} KiB L2 per core; ${h.cpu.l3SharedMiB} MiB shared L3`],
    ["System RAM", `${h.ram.osVisibleGiB} GiB visible; ${h.ram.nominalConfig}${h.ram.nominalInferred ? " (configuration inferred)" : ""}`],
    ["Graphics", `${h.gpu.name} · ${h.gpu.smCount} SM · ${h.gpu.l2MiB} MiB L2`],
    ["GPU memory", `${h.gpu.vramGiB} GB ${h.gpu.vramType} · ${h.gpu.memoryBusBits}-bit`],
    ["Host link", h.gpu.pcie],
    ["Display", `${h.display.rendererGpu} → ${h.display.compositor} → ${h.display.displayController}`],
    ["Panel", h.display.panel],
    ["Driver / session", `${SNAPSHOT.driver} · ${SNAPSHOT.session}`],
  ];
  qs("#hardware-facts").innerHTML = rows.map(([key, value]) =>
    `<tr><th scope="row">${escapeHtml(key)}</th><td>${escapeHtml(value)}</td></tr>`
  ).join("");
  qs("#snapshot-line").textContent = `${SNAPSHOT.machine} · inspected ${SNAPSHOT.inspectionDate} · documented snapshot`;
  qs("#snapshot-copy").textContent = `${SNAPSHOT.os}; NVIDIA driver ${SNAPSHOT.driver}. Source revision ${SNAPSHOT.sourceRevision}. The page reports documented facts and no live measurements.`;
}

function stopPlayback() {
  player.pause();
  if (rafId !== null) cancelAnimationFrame(rafId);
  rafId = null;
  previousFrameTime = null;
  renderState();
}

function animationFrame(now) {
  if (!player.playing || document.hidden) {
    rafId = null;
    previousFrameTime = null;
    return;
  }
  if (previousFrameTime !== null) player.advance((now - previousFrameTime) / 1000, { durationSeconds: 75 });
  previousFrameTime = now;
  renderState();
  if (player.playing) rafId = requestAnimationFrame(animationFrame);
  else {
    rafId = null;
    previousFrameTime = null;
  }
}

function startPlayback() {
  player.play();
  renderState();
  if (rafId === null && !document.hidden) rafId = requestAnimationFrame(animationFrame);
}

function togglePlayback() {
  if (player.playing) stopPlayback();
  else startPlayback();
}

function pauseAnd(callback) {
  player.pause();
  if (rafId !== null) cancelAnimationFrame(rafId);
  rafId = null;
  previousFrameTime = null;
  callback();
  renderState();
}

function bindControls() {
  qs("#chapter-nav").addEventListener("click", (event) => {
    const button = event.target.closest("[data-chapter]");
    if (button) pauseAnd(() => player.goToChapter(button.dataset.chapter));
  });
  qs("#focus-list").addEventListener("click", (event) => {
    const button = event.target.closest("[data-focus-node]");
    if (button) map.select(button.dataset.focusNode);
  });
  qsa("[data-view]").forEach((button) => button.addEventListener("click", () => map.setView(button.dataset.view)));
  qs("#detail-level").addEventListener("change", (event) => {
    detailPreference = Number(event.target.value);
    renderState();
  });
  qs("#btn-zoom-in").addEventListener("click", () => map.setZoom(map.zoom * 1.2));
  qs("#btn-zoom-out").addEventListener("click", () => map.setZoom(map.zoom / 1.2));
  qs("#btn-zoom-reset").addEventListener("click", () => map.setView("system"));
  qs("#previous-step").addEventListener("click", () => pauseAnd(() => player.prevStep()));
  qs("#next-step").addEventListener("click", () => pauseAnd(() => player.nextStep()));
  qs("#play-toggle").addEventListener("click", togglePlayback);
  qs("#reset-tour").addEventListener("click", () => {
    detailPreference = null;
    pauseAnd(() => player.reset());
    map.setView("system");
    map.clearSelection();
  });
  qs("#play-speed").addEventListener("change", (event) => player.setSpeed(Number(event.target.value)));
  qs("#timeline-scrubber").addEventListener("input", (event) => {
    pauseAnd(() => player.seek(Number(event.target.value) / Number(event.target.max)));
  });
  qs("#clear-selection").addEventListener("click", () => map.clearSelection());

  document.addEventListener("keydown", (event) => {
    const tag = (event.target.tagName ?? "").toLowerCase();
    if (["input", "select", "textarea", "button", "a"].includes(tag) || event.target.isContentEditable) return;
    if (event.code === "Space") {
      event.preventDefault();
      togglePlayback();
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      pauseAnd(() => player.prevStep());
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      pauseAnd(() => player.nextStep());
    } else if (event.key === "Home") {
      detailPreference = null;
      pauseAnd(() => player.reset());
    } else if (event.key === "End") {
      pauseAnd(() => player.seek(1));
    } else if (["1", "2", "3"].includes(event.key)) {
      detailPreference = Number(event.key) - 1;
      renderState();
    } else if (event.key === "+" || event.key === "=") map.setZoom(map.zoom * 1.2);
    else if (event.key === "-") map.setZoom(map.zoom / 1.2);
    else if (event.key === "0") map.setView("system");
  });

  reducedMotion.addEventListener("change", (event) => map.setMotion(!event.matches));
  map.setMotion(!reducedMotion.matches);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      if (rafId !== null) cancelAnimationFrame(rafId);
      rafId = null;
      previousFrameTime = null;
    } else if (player.playing && rafId === null) rafId = requestAnimationFrame(animationFrame);
  });
}

function init() {
  renderHardwarePanel();
  bindControls();
  renderState();
}

init();
