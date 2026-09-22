# Architecture guide

`tools/architecture-explainer/` is a standalone guide to how exploraFly moves
from build-time assets and flight input through CPU work, Vulkan submission,
GPU resources, and the display path. It is a static project and machine
snapshot, not a connection to the running game. It does not report utilization,
temperatures, clocks, occupancy, frame timings, or presentation feedback.

## Run it

ES modules require HTTP; they do not load from a `file://` URL. From the
repository root, run:

```sh
python3 -m http.server 8765 --directory tools/architecture-explainer
```

Open `http://127.0.0.1:8765/`. The page has no production dependencies, CDN,
or backend.

## Take the tour

The left rail contains five chapters and 33 explanatory stops. Select a chapter,
then use **Previous**, **Next**, or **Play tour**. The timeline can be scrubbed;
playback speed is adjustable. Playback orders the explanation. Its timing does
not measure execution time.

The map highlights the blocks and connections named by the current stop. Use
**Whole system**, **CPU**, or **GPU** to frame the schematic, and choose
**System**, **Subsystems**, or **Resources** to change its detail. Detail
expands automatically when a step needs blocks hidden at the chosen level;
manually choosing a lower level keeps that overview and notes that some
highlighted blocks are hidden. Select a map block or a highlighted-block chip to
inspect its role, resources, dependencies, and code references.

Drag the map to pan; scroll or use the zoom buttons to zoom. **Restart** returns
to the first stop and restores the whole-system view. Keyboard controls are
**Space** to play or pause, **← / →** to move between stops, **Home / End** to
jump to the beginning or end, **1 / 2 / 3** to choose map detail, **+ / −** to
zoom, and **0** to restore the whole-system view. SVG components can be focused
and selected with Enter or Space. The page honors `prefers-reduced-motion`.

## Read the map carefully

The map coordinates are a logical layout. CPU-core and SM-level details are
inventories or representative resources; they do not describe a die floorplan,
OS thread pinning, GPU scheduling, cache traffic, or a shader-to-SM assignment.
Animated connection marks illustrate documented relationships and are not live
activity.

The 144 Hz fixed-step simulation, the project target of 1,000 complete
presentation submissions per second (1 ms), and the 120 Hz panel refresh are
separate quantities. The target is not claimed as achieved. A submitted present
is not necessarily a distinct image displayed by the monitor, and this guide
measures none of these rates.

Hardware claims and their provenance live in `data/hardware.js`; the graph
nodes and code references are in `data/graph.js`; the five narration scripts
are in `data/chapters.js`. Source records include URLs or local evidence,
publication or inspection dates where available, access dates, and limits on
what each source establishes.

## Check changes

From this directory, run `npm test`. The Node test suite validates chapter and
graph references, timeline behavior, documented hardware arithmetic, and the
page/controller element contract. Browser screenshots are a visual check; they
are not game-renderer or GPU-performance evidence.
