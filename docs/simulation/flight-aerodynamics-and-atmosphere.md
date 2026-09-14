# Flight Aerodynamics, Atmospheric Physics, & Vortex Simulation

This document specifies the physics, atmospheric modeling, gas dynamics, and aerodynamic vortex simulation implemented in `crates/sim`.

---

## 1. Deterministic Physics Architecture (144 Hz)

Flight dynamics, atmospheric sampling, and particle tracking operate on a strict, deterministic fixed-frequency update loop completely decoupled from graphics rendering:

- **Fixed Timestep**: $\Delta t = \frac{1}{144}\ \text{s} \approx 6.944\ \text{ms}$ (`SIM_STEP`).
- **Render Interpolation**: The renderer evaluates frames asynchronously (e.g. at 1,429+ FPS on RTX 4060). To prevent visual jitter without introducing physics instability, graphics transforms linearly interpolate (`lerp`) between the previous state $\mathbf{S}_{t-1}$ and current state $\mathbf{S}_t$ weighted by the sub-tick accumulator remainder $\alpha \in [0, 1)$.
- **Coordinate System**: Right-handed Cartesian space with Y-up:
  - $+X$: Aircraft right (starboard).
  - $+Y$: World vertical / altitude.
  - $+Z$: Forward flight heading.

```
       +Y (Altitude)
          ▲
          │
          │     ▲ +Z (Forward heading)
          │    ╱
          │   ╱
          │  ╱
          │ ╱
          └──────────► +X (Right / Starboard)
```

---

## 2. 6-DOF Flight Kinematics & Aerodynamic Response

The aircraft state $\mathbf{P} = (x, y, z, \psi, \theta, \phi, v, \beta_{\text{boost}})$ tracks spatial position, Euler angles, airspeed, and engine spool:

### 2.1 Attitude Dynamics & Lift-Plane Steering
Flight control surfaces respond through a first-order exponential smoothing filter:
$$k = 1 - e^{-4.0 \Delta t}$$
$$\theta \leftarrow \theta + (0.6\, u_{\text{pitch}} - \theta) k$$
$$\phi \leftarrow \phi + (1.1\, u_{\text{bank}} - \phi) k$$

**Coordinated Turn Physics**:
Heading rate $\dot{\psi}$ couples passive roll slip, bank-induced elevator lift pulling, and direct rudder yaw:
$$\dot{\psi} = 0.35\, \phi + 1.6\, \theta \sin(\phi) - 0.5\, u_{\text{yaw}}$$
- **Wings-level ($\phi = 0$)**: Elevator pitch $\theta$ produces pure vertical climb/dive ($\dot{y} > 0$) with zero heading deviation ($\dot{\psi} = 0$).
- **Knife-edge bank ($\phi = \pm \frac{\pi}{2}$)**: Elevator pitch redirects the aerodynamic lift vector horizontally, pulling the nose through a high-G turn with minimal vertical climb.

### 2.2 Forward Velocity Projection
The velocity vector projects from local aircraft space into world space by applying pitch along the rolled lift-plane axis:
$$\begin{aligned}
v_{x,\text{local}} &= -\sin(\theta) \sin(\phi) \\
v_{y,\text{local}} &= \sin(\theta) \cos(\phi) \\
v_{z,\text{local}} &= \cos(\theta)
\end{aligned}$$

Projected into world heading $\psi$:
$$\begin{aligned}
\dot{x} &= v \left(v_{z,\text{local}} \sin(\psi) + v_{x,\text{local}} \cos(\psi)\right) \\
\dot{y} &= v \, v_{y,\text{local}} \\
\dot{z} &= v \left(v_{z,\text{local}} \cos(\psi) - v_{x,\text{local}} \sin(\psi)\right)
\end{aligned}$$

Operational altitude is clamped between $130\ \text{m}$ (terrain safety margin) and $22,000\ \text{m}$ (stratospheric ceiling).

---

## 3. International Standard Atmosphere (ISA) Model

Atmospheric temperature, barometric pressure, and air density govern aerodynamic drag, sound speed, and contrail thermodynamics from sea level to $22\ \text{km}$.

```
Altitude Band           Temperature Profile T(h)            Pressure Equation P(h)
───────────────────────────────────────────────────────────────────────────────────────────
Troposphere (0–11 km)   T0 - Γ·h (Lapse Γ = 6.5 K/km)      P0 · (T(h) / T0)^5.25588
Tropopause (11–22 km)   Isothermal (T11 = 216.65 K)         P11 · exp(-g·M·(h - 11 km) / R·T)
```

1. **Sea Level Constants**:
   - $T_0 = 288.15\ \text{K}\ (15^\circ\text{C})$
   - $P_0 = 101,325\ \text{Pa}$
   - Specific gas constant for dry air: $R = 287.05\ \text{J/(kg}\cdot\text{K)}$
   - Tropospheric lapse rate: $\Gamma = 0.0065\ \text{K/m}$
2. **Tropospheric Barometric Formula ($h < 11,000\ \text{m}$)**:
   $$T(h) = T_0 - \Gamma h$$
   $$P(h) = P_0 \left(\frac{T(h)}{T_0}\right)^{\frac{g M}{R \Gamma}} = P_0 \left(\frac{T(h)}{T_0}\right)^{5.25588}$$
3. **Stratospheric Isothermal Layer ($11,000\ \text{m} \le h \le 22,000\ \text{m}$)**:
   $$T(h) = 216.65\ \text{K}$$
   $$P(h) = P_{11} \exp\left(-\frac{g M (h - 11000)}{R T}\right)$$
4. **Air Density**:
   $$\rho(h) = \frac{P(h)}{R \cdot T(h)}$$

---

## 4. Schmidt-Appleman Contrail Thermodynamics

Contrails form when warm, moisture-laden jet exhaust mixes with cold ambient air, causing local relative humidity to exceed liquid water saturation (Schumann 1996).

### 4.1 Mixing Line Slope ($G$)
The isobaric mixing trajectory in the $(T, e)$ state diagram has slope $G$ (in $\text{Pa/K}$):
$$G = \frac{EI_{\text{H}_2\text{O}} \cdot P \cdot \frac{M_{\text{air}}}{M_{\text{H}_2\text{O}}}}{Q_{\text{fuel}} (1 - \eta_{\text{prop}})}$$
- Fuel lower heating value: $Q_{\text{fuel}} = 43.2 \times 10^6\ \text{J/kg}$ (aviation kerosene).
- Water vapor emission index: $EI_{\text{H}_2\text{O}} = 1.25\ \text{kg H}_2\text{O / kg fuel}$.
- Propulsion overall efficiency: $\eta_{\text{prop}} = 0.30$.
- Molar mass ratio $\frac{M_{\text{air}}}{M_{\text{H}_2\text{O}}} = \frac{28.9644}{18.01528} \approx 1.6078$.

### 4.2 Critical Threshold Temperature ($T_{\text{crit}}$)
Contrail formation requires ambient temperature $T_{\text{amb}} \le T_{\text{crit}}$:
$$T_{\text{crit}}(P, \text{RH}) \approx 226.0 + 8.5 \ln\left(1 + \frac{G}{1.6 \times 10^{-3}}\right) + 9.0 \cdot \text{RH}_{\text{liquid}}$$

### 4.3 Persistence Criterion
- If ambient air is ice-supersaturated ($RH_{\text{ice}} > 100\%$), condensed ice crystals grow and form persistent cirrus contrails.
- If $RH_{\text{ice}} \le 100\%$, the contrail sublimates within seconds.

---

## 5. Supersonic Exhaust Gas Dynamics & Shock Diamonds

Under afterburner operation, underexpanded supersonic exhaust forms standing diamond shock cells (Prandtl 1904, Powell 2010).

```
 Nozzle Lip              Shock Diamond Cells (Wavelength λ)
 ┌──────┐           /\         /\         /\
 │Engine│══════════/  \═══════/  \═══════/  \════════ (Exhaust Flow)
 └──────┘          \  /       \  /       \  /
                    \/         \/         \/
                   |<─   λ   ─>|
```

1. **Fully Expanded Jet Mach Number ($M_j$)**:
   Derived from nozzle pressure ratio $\text{NPR} = \frac{P_{\text{nozzle}}}{P_{\text{amb}}}$ for an isentropic ideal gas ($\gamma = 1.4$):
   $$M_j = \sqrt{\frac{2}{\gamma - 1} \left(\text{NPR}^{\frac{\gamma - 1}{\gamma}} - 1\right)} = \sqrt{5.0 \left(\text{NPR}^{0.2857} - 1\right)}$$

2. **Prandtl Shock Cell Wavelength ($\lambda$)**:
   $$\lambda = 1.306 \cdot d_{\text{nozzle}} \cdot \sqrt{M_j^2 - 1}$$
   where $d_{\text{nozzle}}$ is the effective nozzle exit diameter.

3. **Volumetric Density & Blackbody Temperature Modulation**:
   Within the volumetric raymarcher, standing shock cells modulate the core temperature and density:
   $$\text{band} = 0.5 + 0.5 \cos\left(\frac{2\pi z}{\lambda}\right)$$
   $$\text{cell} = \text{band}^3 \exp(-0.28 z) \cdot \beta_{\text{boost}}$$
   $$T(z) = \text{lerp}(900\ \text{K}, 800\ \text{K} + 1300\ \text{K} \cdot \beta_{\text{boost}}, e^{-0.22 z}) + 700\ \text{K} \cdot \text{cell}$$

---

## 6. Trailing Lamb-Oseen Vortices & Crow Instability

Wingtip vortices generate persistent trailing aerodynamic wakes that undergo mutual induction instability (Crow 1970).

### 6.1 Lamb-Oseen Velocity Profile
The tangential swirl velocity $v_\theta(r)$ around each vortex core with circulation $\Gamma_0$ and core radius $r_c$:
$$v_\theta(r) = \frac{\Gamma_0}{2\pi r} \left(1 - \exp\left(-\frac{r^2}{r_c^2}\right)\right)$$

### 6.2 Crow Sinusoidal Instability
Mutual velocity induction between the left and right vortex pair separated by distance $b_0 = \frac{\pi}{4} b$ amplifies symmetric perturbations at a characteristic wavelength:
$$\lambda_{\text{Crow}} \approx 8.6 \cdot b_0$$
As the vortices age, the sinusoidal displacement grows exponentially until the vortices reconnect into linked vortex rings, accelerating turbulent dissipation.

---

## 7. Procedural Volumetric Noise Synthesis (`noise.rs`)

The volume noise generator implements the Nubis multi-scale volumetric noise model:

1. **Base 3D Noise (Perlin-Worley 3D, $32^3$ RGBA)**:
   - **R Channel**: Low-frequency Perlin value noise inverted and combined with cellular Worley noise.
   - **G, B, A Channels**: Progressive high-frequency Worley octaves ($2\times, 4\times, 8\times$ frequency) used to erode cloud and exhaust boundaries.
2. **Detail 3D Noise (Worley 3D, $16^3$ RGB)**:
   - High-frequency billowy Worley cellular textures for fine-scale boundary turbulence.
3. **Domain Warp 2D Texture ($32^2$ RG)**:
   - 2D curl noise vector field for shearing and advecting volumetric rays in the exhaust stream.
4. **Deterministic Hash Lattice**:
   Evaluated using integer lattice hashing without OS entropy, guaranteeing bit-exact reproducible noise fields at engine boot time.
