# Architecture explorer

The architecture explorer is a single, full-screen 2D illustration of exploraFly's CPU, GPU, memory, and display path. It has no chapter rail or surrounding control panel.

## Run it

From the repository root:

```sh
python3 -m http.server 8765 --directory tools/architecture-explainer
```

Open `http://127.0.0.1:8765/`. The page has no production dependencies, CDN, or backend.

## Explore the diagram

The first screen shows all eight Ryzen 7 7840HS CPU cores, the shared CPU L3 cache, the PCIe 4.0 x8 host link, all 24 RTX 4060 Laptop GPU streaming multiprocessors, shared GPU L2, 8 GiB GDDR6, and the hybrid presentation route to the laptop display. Click a core, SM, cache, memory block, link, or screen to reveal a short explanation in the illustration. Keyboard users can focus components and press Enter or Space; Escape closes the explanation. The page honors `prefers-reduced-motion`.

The colored marks show an illustrative path from CPU to GPU to screen. They are not live traffic or telemetry. Core and SM cells are inventories, not die layouts or fixed thread/work assignments. Hardware capacities and paths come from the documented snapshot in `data/hardware.js`; each source record includes provenance and limits.

The 144 Hz fixed-step simulation, 1,000-per-second presentation submission target, and 120 Hz panel refresh are separate measures. The target is not claimed as achieved. A submitted present is not necessarily a distinct frame displayed by the monitor, and this page measures none of these rates.
