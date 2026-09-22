import { HARDWARE, SNAPSHOT } from "../data/hardware.js";

const svg = document.querySelector("#architecture");
const detailCard = document.querySelector("#detail-card");
const titleNode = document.querySelector("#detail-title");
const bodyNode = document.querySelector("#detail-body");
const liveDescription = document.querySelector("#live-description");

const cpu = HARDWARE.cpu;
const gpu = HARDWARE.gpu;

function detailsFor(target) {
  const kind = target.dataset.kind;
  const index = Number(target.dataset.index || 0);

  if (kind === "cpu-core") {
    return {
      title: "CPU core " + (index + 1) + " / " + cpu.cores,
      body: "One Zen 4 core. Two SMT contexts, 32 KiB each of L1 instruction and data cache, and 1 MiB of L2.",
    };
  }
  if (kind === "gpu-sm") {
    return {
      title: "GPU SM " + String(index + 1).padStart(2, "0") + " / " + gpu.smCount,
      body: "One Ada SM: 128 CUDA cores, four Tensor Cores and one RT Core. GPU scheduling assigns work dynamically.",
    };
  }
  if (kind === "l3") {
    return {
      title: "Shared CPU L3 · " + cpu.l3SharedMiB + " MiB",
      body: "A last-level cache shared by the eight CPU cores. Its presence does not say which requests hit it.",
    };
  }
  if (kind === "gpu-l2") {
    return {
      title: "Shared GPU L2 · " + gpu.l2MiB + " MiB",
      body: "A GPU-wide cache shared by the 24 SMs. This schematic does not measure cache hits.",
    };
  }
  if (kind === "vram") {
    return {
      title: gpu.vramGiB + " GiB of " + gpu.vramType,
      body: "Device-local memory for GPU resources such as geometry, textures and render targets.",
    };
  }
  if (kind === "pcie") {
    return {
      title: "PCIe 4.0 ×8 host link",
      body: "Connects the CPU side to the GPU. Resources and small updates cross it; the line is not continuous traffic.",
    };
  }
  return {
    title: "GPU rendering → laptop display",
    body: "RTX 4060 renders. Wayland / Mutter hands off the surface to Radeon 780M, which drives the 120 Hz panel.",
  };
}

function wrapText(node, text, maxCharacters = 43) {
  while (node.firstChild) node.removeChild(node.firstChild);
  const words = text.split(/\s+/);
  const lines = [];
  let line = "";
  for (const word of words) {
    const candidate = line ? line + " " + word : word;
    if (candidate.length > maxCharacters && line) {
      lines.push(line);
      line = word;
    } else {
      line = candidate;
    }
  }
  if (line) lines.push(line);
  lines.forEach((part, index) => {
    const span = document.createElementNS("http://www.w3.org/2000/svg", "tspan");
    span.setAttribute("x", "22");
    span.setAttribute("dy", index === 0 ? "0" : "17");
    span.textContent = part;
    node.appendChild(span);
  });
}

function showDetails(target) {
  const info = detailsFor(target);
  titleNode.textContent = info.title;
  wrapText(bodyNode, info.body);
  detailCard.setAttribute("transform", "translate(1218 147)");
  detailCard.setAttribute("visibility", "visible");

  svg.querySelectorAll(".is-selected").forEach((node) => node.classList.remove("is-selected"));
  target.classList.add("is-selected");
  liveDescription.textContent = info.title + ". " + info.body;
}

function clearDetails() {
  detailCard.setAttribute("visibility", "hidden");
  svg.querySelectorAll(".is-selected").forEach((node) => node.classList.remove("is-selected"));
  liveDescription.textContent = "";
}

svg.addEventListener("click", (event) => {
  const target = event.target.closest(".interactive");
  if (target) showDetails(target);
  else clearDetails();
});

svg.addEventListener("keydown", (event) => {
  if (event.key === "Enter" || event.key === " ") {
    const target = event.target.closest(".interactive");
    if (target) {
      event.preventDefault();
      showDetails(target);
    }
  } else if (event.key === "Escape") {
    clearDetails();
  }
});

document.querySelectorAll(".interactive").forEach((target) => {
  target.addEventListener("focus", () => showDetails(target));
});

liveDescription.textContent =
  "Snapshot inspected " + SNAPSHOT.inspectionDate + ". " +
  cpu.name + ": " + cpu.cores + " cores and " + cpu.smtContexts + " threads. " +
  gpu.name + ": " + gpu.smCount + " SMs and " + gpu.vramGiB + " GiB of video memory. Static schematic; no live hardware readings.";
