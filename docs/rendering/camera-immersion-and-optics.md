# Camera Immersion, Perception Physics, and Real-Footage Optical Pipeline

This note records the camera kinematics, flight dynamics, Vulkan uniform integration, and optical shader pipeline implemented for `exploraFly_2`. It includes one target-GPU throughput capture below; that run failed the 1,000-present/s target, and target-GPU visual acceptance remains separate.

---

## 1. Problem Statement & Research Foundation

In high-speed flight simulation and open-world exploration, rendering high-fidelity geometry and atmospheric scattering alone is insufficient for immersion. Pure pinhole cameras rigidly affixed to the aircraft airframe feel synthetic, static, and disconnected from the physical world.

Human perception of high-speed airborne footage (such as military HUDs, chase plane telemetry, cockpit action cams, and FPV drones) relies on two interconnected systems:
1. **Dynamic Kinematics & Inertial Mass**: Cameras have mass, mounting elasticity, and aerodynamic drag. Accelerating, high-G turns, buffeting, and transonic shockwaves cause lag, displacement, and airframe micro-vibration.
2. **Physical Optical Imperfections**: Lenses are curved optical elements with chromatic dispersion, mechanical vignetting ($cos^4\theta$), finite sensor dynamic range requiring temporal exposure adaptation, and CMOS photodiode quantum shot noise (sensor grain).

### Key Research & Literature Citations
- **Critically Damped Harmonic Oscillators**: Ryan Juckett's analytic *Damped Springs* formulation [1] provides the scalar fixed-target update used by the camera. It does not prove that the local rotation-vector approximation is globally independent or free from overshoot under arbitrarily fast target changes.
- **External Camera Design Patterns**: The current MSFS SDK and Cinemachine manuals expose velocity-based external framing, independent damping, lookahead, and camera-radius obstacle avoidance as configurable techniques [2, 9–11]. These are implementation examples, not evidence of vestibular safety or validated tuning for this game.
- **Optical Lens Distortion Models**: Brown-Conrady polynomial radial distortion ($r_d = r_u(1 + k_1 r_u^2 + k_2 r_u^4)$) matching wide-angle airborne action lenses [3].
- **Transverse Chromatic Aberration & Spectral Dispersion**: Wavelength-dependent radial focal shift in wide optical elements (SIGGRAPH computational optics & post-processing notes) [4].
- **Luminance-Dependent Sensor Noise**: Poisson-Gaussian CMOS photodiode shot noise models for organic digital sensor response [5].

---

## 2. Flight Dynamics & Camera Kinematics (`crates/sim/src/camera.rs`)

### 2.1 Air-Relative Reference and Aerobatic Roll
The chase camera blends the aircraft nose direction with normalized air-relative velocity (`pose.velocity - wind_velocity`). Speed authority ramps from 10 to 25 m/s. The velocity contribution also depends on the angular divergence between body-forward and airflow, reaching at most 72% when divergence is large. Near vertical body attitudes, velocity influence is faded out so loops and vertical recoveries continue to use the aircraft orientation.

This is a partial blend: it borrows the transition idea from the MSFS external-camera settings but does not copy their full velocity-follow rule. Body-forward still controls roll extraction. Fractional horizon stabilization uses the existing bank curve, then smoothly yields to body orientation in extreme vertical flight. Quaternions remain the final orientation representation, preserving orthogonal camera axes and avoiding Euler singularities.

### 2.2 Independent Angular Follow
Angular follow no longer applies one exponential slerp factor to the entire attitude. The shortest local quaternion error is represented as a rotation vector; its local X, Y, and Z components are advanced with separate critically damped springs at 38, 28, and 22 s$^{-1}$ (pitch, yaw, and roll coordinates). The incremental rotation is recomposed as a normalized quaternion, preserving the existing horizon and vertical-flight logic.

The scalar spring update is analytic for a fixed target. The three rotation-vector coordinates are still a local approximation on $SO(3)$: rotations do not commute, so these are independent response rates rather than globally independent Euler axes. Existing loop, inversion, reversal, and continuity tests remain the guard against unstable behavior.

### 2.3 Translational Inertia, Lookahead, and Lens Response
Air-relative acceleration drives a critically damped offset in camera-local right, up, and forward coordinates. The offset target is bounded to $\pm0.22$ m laterally, $\pm0.18$ m vertically, and $\pm0.12$ m longitudinally. This adds modest mounting inertia without speed-based boom extension.

The look target advances by `clamp_length(air_velocity * 0.035 s, 3 m)`. This is intentionally short and capped because Cinemachine's current Position Composer documentation warns that lookahead can amplify noisy target motion and cause jitter. The vertical FOV starts at $72^\circ$ and rises smoothly by at most $2^\circ$ between 120 and 850 m/s; this is a restrained lens response, not zooming the chase rig far away.

### 2.4 Swept-Volume Boom Clearance
The render loop passes its cached world collision-height query to the camera. The camera sweeps its nominal $14.5$ m back / $3.6$ m up boom in 1 m increments, checking the center and eight points around a 0.55 m radius. On the first terrain, roof, tree-crown, or scatter-height obstruction, the boom contracts immediately to before the detected obstruction, subject to the 0.12 minimum length fraction; an obstruction very close to the aircraft anchor can therefore still intersect the camera. It returns toward full length with a 3 s$^{-1}$ exponential release.

This is a bounded height-field approximation of a camera-sphere sweep, not a triangle-level capsule cast: `WorldNeighborhood::collision_height_at` provides a conservative vertical envelope for terrain and known objects. It prevents many boom-through-ridge/building cases but can over-contract near broad height envelopes and does not guarantee collision against arbitrary future meshes. The minimum boom fraction is 0.12 to avoid placing the camera directly at the aircraft anchor.

### 2.5 Continuous PD Aerodynamic Pitch Control Law (`crates/sim/src/flight.rs`)
Root-cause analysis revealed that visual "jitter" on the tail fin during pitch and bank maneuvers was caused by a discrete sign-branching step discontinuity in the legacy aerodynamic pitch controller:
- *Legacy Flaw*: Branching on `if error > 0.0` caused the pitch damping term to step discontinuously by up to $18.7\text{ rad/s}^2$ as angle-of-attack crossed trim. At $144\text{ Hz}$, this triggered a discrete limit cycle chattering between $+13.0$ and $-13.0\text{ rad/s}^2$, shaking the empennage at $z = -4.5\text{ m}$ in a physically impossible square wave.
- *Continuous PD Resolution*: Replaced with a smooth 2nd-order damped harmonic oscillator ($\zeta \approx 0.85$):
  $$a_{\text{pitch}} = \text{clamp}\left((\alpha_{\text{target}} - \alpha) \cdot 22.0 - (\omega_{\text{pitch}} - \omega_{\text{target}}) \cdot 8.0, -12.0, 12.0\right) \cdot \mu_{\text{authority}}$$
  Continuous in both position and rate ($C^1$ in $\omega$, $C^2$ in orientation), completely eliminating square-wave limit cycles and stabilizing the empennage.

### 2.6 Sub-Step Pose Interpolation for Variable Refresh Rates (`engine/src/main.rs`)
A second critical contributor to tail jitter was the simulation-to-render timestep disparity (the classic Glenn Fiedler *"Fix Your Timestep!"* problem):
- *The Problem*: The aerodynamic physics simulation steps in discrete slices of $\Delta t_{\text{sim}} = 1/144\text{ s} \approx 6.94\text{ ms}$. At display refresh rates such as $60\text{ Hz}$, $165\text{ Hz}$, $240\text{ Hz}$, or uncapped presentation (>500 FPS), the number of physics steps executed per render frame varies unpredictably (alternating between 0, 1, or 2 steps). Without interpolation, on a zero-step frame the aircraft is completely frozen, and on the next frame it leaps forward by a full $6.94\text{ ms}$ slice. Because the tail is located $4.5\text{ m}$ behind the rotation center, a pitch rotation of $2.1\text{ rad/s}$ translates to a sudden $6.7\text{ cm}$ visual displacement on screen every alternating frame, appearing as high-frequency square-wave jitter!
- *The Solution*: Implemented canonical sub-step render interpolation:
  $$\alpha = \text{clamp}\left(\frac{\text{accumulator}}{\Delta t_{\text{sim}}}, 0.0, 1.0\right)$$
  $$\vec{r}_{\text{render}} = \vec{r}_{\text{prev}} + (\vec{r}_{\text{curr}} - \vec{r}_{\text{prev}}) \cdot \alpha$$
  $$\mathbf{Q}_{\text{render}} = \text{slerp}(\mathbf{Q}_{\text{prev}}, \mathbf{Q}_{\text{curr}}, \alpha)$$
  This guarantees that the aircraft mesh and chase camera advance continuously and monotonically on every single display frame, with zero micro-stutter or discrete leaps.

### 2.7 Continuous $C^2$ Airframe Structural Rumble (`crates/sim/src/camera.rs`)
To deliver authentic high-speed flight immersion without artificial strobing or square-wave artifacts:
1. **Low-Frequency Structural Modes**: Real fighter aircraft buffeting occurs at fundamental structural resonant frequencies (wing bending at $3.6\text{ Hz}$, empennage torsion at $7.2\text{ Hz}$, atmospheric swell at $1.6\text{ Hz}$). At 60–144 FPS, these low frequencies are well below the Nyquist limit ($f_N = 30-72\text{ Hz}$), preventing discrete frame-alternating aliasing.
2. **Ken Perlin Quintic Hermite Gradient Noise ($C^2$ Continuity)**:
   $$s(t) = 6t^5 - 15t^4 + 10t^3 \quad (s'(0)=s'(1)=0, s''(0)=s''(1)=0)$$
   Guarantees continuous first and second derivatives (continuous velocity and acceleration), preventing impulse jerk spikes.
3. **Squirrel Eiserloh Trauma Model**:
   - Trauma $T \in [0.0, 1.0]$ tracks aerodynamic stress: G-load factor ($|G - 1| > 0.5$), transonic shock buffet ($M \in [0.85, 1.22]$), high-AoA flow separation ($|\alpha| > 0.20\text{ rad}$), dynamic pressure ($v > 350\text{ m/s}$), and afterburner thrust.
   - Non-linear response: $\text{shake} = T^2$.
   - Asymmetric envelope: fast attack ($8.0\text{ s}^{-1}$) upon entering high G, gradual decay ($2.5\text{ s}^{-1}$) settling back to quiet cruise.
4. **Sightline-Aligned Roll Rumble (Zero Plane Parallax)**:
   In War Thunder's chase camera, airframe buffeting is aligned with the camera's sightline ($\hat{z}_{\text{cam}}$). Rotating the camera in pitch or yaw relative to the aircraft would displace the tail fin (which sits 10m away from the lens) vertically or horizontally on screen. By applying the structural buffet rotation strictly around the sightline vector ($\mathbf{Q}_{\text{roll}}$), the distant horizon and clouds tilt dynamically, while the aircraft reticle, empennage, and tail remain rock-solid in screen coordinates.

---

## 3. Hardware Pipeline & Zero-Cost Vulkan Integration

### 3.1 Host-Coherent Mapped Uniform Memory (`engine/src/plane.rs`)
To preserve the 1,000 submitted-presentation/s design target without re-recording Vulkan command buffers:
- Pre-recorded command buffers bind `set_layout` (Group 0: UBO) and `composite_set_layout` (Group 1: HDR linear scene target).
- Per-frame camera state is packed into the host-coherent mapped UBO memory (`dst.add(32 + NODE_COUNT * 16)`).
- UBO allocation expanded to $1776\text{ bytes}$ ($444\text{ floats}$), perfectly maintaining $16\text{-byte}$ alignment for Vulkan std140 buffers.

### 3.2 Tail Layout Definition
| UBO Offset | Name | Component Data |
|---|---|---|
| `tail[36..40]` | `cameraParams` | `(fov_y, aspect_ratio, speed_mps, wing_load_G)` |
| `tail[40..44]` | `cameraParams2` | `(shake_intensity, photometric_exposure, mach_number, 0.0)` |

---

## 4. Real-Footage Optical Post-Processing (`engine/shaders/composite.frag`)

The final composite pass executes on the linear HDR target (before ACES tonemapping and sRGB swapchain presentation):

```
Scene Passes (Opaque + Sky + Plume + Trail + Glass) [RGBA16F Linear HDR]
                                  │
                                  ▼
                1. Brown-Conrady Curvilinear Lens Distortion
                                  │
                                  ▼
                2. Transverse Radial Chromatic Aberration
                                  │
                                  ▼
                3. High-Speed Peripheral Optical Flow Streaking
                                  │
                                  ▼
                4. Dynamic Photometric Auto-Exposure Adaptation
                                  │
                                  ▼
                5. Physical cos⁴(θ) Vignetting
                                  │
                                  ▼
                6. High-Radiance Halation / Optical Glare
                                  │
                                  ▼
                7. Photodiode Poisson-Gaussian CMOS Sensor Grain
                                  │
                                  ▼
                8. ACES Filmic Tonemapping -> sRGB Swapchain
```

### 4.1 Brown-Conrady Curvilinear Lens Distortion
Simulates wide-angle airborne action camera optics:
$$\vec{u}_{\text{centered}} = \vec{u} - 0.5$$
$$r^2 = (u_x \cdot \text{aspect})^2 + u_y^2$$
$$D(r) = 1.0 + 0.042 r^2 + 0.016 r^4$$
An overscan compensation factor ($S = 0.970$) preserves pixel coverage across viewport corners while curving horizontal horizon lines realistically at the periphery.

### 4.2 Transverse Chromatic Aberration
Models wavelength-dependent refraction where red light focuses outward and blue inward:
$$\vec{\delta}_{\text{ca}} = \vec{u}_{\text{centered}} \cdot (0.0030 + 0.0020 \cdot \text{shake}) r^2$$
$$R = \text{texture}(\dots, \vec{u}_{\text{lens}} + \vec{\delta}_{\text{ca}}).r$$
$$G = \text{texture}(\dots, \vec{u}_{\text{lens}}).g$$
$$B = \text{texture}(\dots, \vec{u}_{\text{lens}} - \vec{\delta}_{\text{ca}}).b$$
The center crosshair and cockpit remain perfectly sharp ($r^2 \to 0$), while frame edges exhibit subtle, authentic color fringing.

### 4.3 High-Speed Velocity Streaking
At speeds exceeding $100\text{ m/s}$ (Mach $0.3+$ to Mach $3+$), peripheral optical flow exhibits radial motion integration streaks sampled outward from the vanishing point, magnifying the sensation of speed.

### 4.4 Dynamic Photometric Auto-Exposure
Tracks solar alignment ($\cos\theta_{\text{sun}} = \hat{d}_{\text{look}} \cdot \hat{d}_{\text{sun}}$). When facing the solar disk, the exposure smoothly stops down to preserve sun corona and atmospheric halo detail. When banking away toward darkened terrain, exposure opens up to reveal shadow contrast.

### 4.5 Photodiode Poisson-Gaussian Sensor Grain
Replaces CG digital flatness with organic camera sensor texture:
$$N = \text{hash}(\text{gl\_FragCoord.xy}, \text{time})$$
$$W_{\text{luma}} = 4.0 \cdot Y^{0.55} \cdot (1.0 - Y)$$
Concentrated in midtones and shadows, eliminating 8-bit color quantization banding on the atmospheric sky gradient without washing out specular highlights.

---

## 5. Verification & Performance

- **Unit Tests**:
  - `sim::flight::tests::pitch_control_is_smooth_without_chattering_square_wave`
  - `sim::camera::tests::chase_camera_axis_springs_converge_smoothly_without_divergence`
  - `sim::camera::tests::nominal_boom_distance_stays_stable_without_obstructions`
  - `sim::camera::tests::camera_to_anchor_distance_is_invariant_under_active_rumble`
  - `sim::camera::tests::trauma_rises_under_high_g_and_transonic_buffet_and_decays_smoothly`
  - `sim::camera::tests::quintic_noise_has_continuous_first_and_second_derivatives`
  - `sim::camera::tests::camera_follows_pitch_and_survives_vertical_and_inverted_flight`
  - `sim::camera::tests::high_speed_reference_blends_toward_airflow_without_losing_vertical_body_follow`
  - `sim::camera::tests::translational_response_is_bounded_and_opposes_lateral_acceleration`
  - `sim::camera::tests::swept_camera_boom_contracts_at_a_ridge_and_returns_slowly`
  - `sim::camera::tests::world_up_projects_toward_top_of_vulkan_image`
  - `engine::plane::tests::test_ubo_tail_and_bytes_alignment`
- **Native target-GPU capture (2026-09-23)**: One 10,000-present `dist` run used the NVIDIA GeForce RTX 4060 Laptop GPU, driver 580.173.02, on the 2880×1800/120 Hz workstation display. The maximized window and scene were 2880×1646; quality was Cinematic, one complete scene per present, 1× MSAA, ray-traced shadows and mesh shaders enabled, MAILBOX present mode. The recorded workload was the default spawn in calm wind. Although `EXPLORA_BOOST=1` and `EXPLORA_BANK=1` were set, the playable `dist` build reported `force_boost=false` and `force_bank=false`, so those debug-only flight overrides were not active.
  - Submitted-present intervals: mean 6047.332 μs (165.362/s), p50 5972.173 μs, p95 7248.532 μs, p99 7432.565 μs, max 11393.434 μs; 9993/10000 exceeded 1 ms. The submission target failed.
  - GPU frame timestamps: mean 6025.005 μs, p50 5953.664 μs, p95 7129.888 μs, p99 7297.568 μs, max 9573.440 μs. The logged CPU `sim+camera` stage averaged 330.4 μs; this combines simulation and camera work and does not isolate the sweep cost.
  - The Vulkan display-timing extension was unavailable, so there were 0/10000 distinct displayed-frame feedback samples. The reported rate counts queue-present submissions and does not establish monitor display cadence. The sweep's per-frame CPU cost still needs an isolated comparison, and camera visual acceptance has not been performed.
- **Acceptance status**: The camera has CPU simulation coverage and a complete native throughput sample, but that sample fails the project's 1 ms submission target. Target-GPU visual acceptance remains unverified.

---

## 6. References

1. Juckett, R. [*Damped Springs*](https://www.ryanjuckett.com/damped-springs/). Published 2012-07-20 (accessed 2026-09-23). Applicability: analytic scalar damped-spring integration. Limit: derivation assumes a fixed target and does not establish global stability for the camera's coupled rotation-vector update.
2. Microsoft Flight Simulator 2024 SDK, [*cameras.cfg*](https://docs.flightsimulator.com/msfs2024/retail/content-configuration/cfg-files/cameras.cfg/). The page does not state a publication/update date; accessed 2026-09-23. Applicability: its external-camera settings specify heading-follow below 10 m/s and velocity-vector follow above 25 m/s. Limit: product configuration reference, not a measured comparison or comfort study; this camera uses a partial, divergence-weighted blend.
3. Brown, D. C. *Decentering distortion of lenses*. Photogrammetric Engineering (1966).
4. Heide, F. et al. *End-to-end Optimization of Optics and Image Processing for Achromatic Extended Depth of Field and Super-resolution Imaging*. ACM Transactions on Graphics (SIGGRAPH 2013).
5. Foi, A. et al. *Practical Poissonian-Gaussian Noise Parameter Estimation and Simulation for Raw Sensor Data*. IEEE Transactions on Image Processing (2008).
6. Eiserloh, S. *Math for Game Programmers: Juicing Your Cameras With Math*. Game Developers Conference (GDC 2016).
7. Perlin, K. *Improving Noise*. ACM Transactions on Graphics (SIGGRAPH 2002).
8. Mabey, D. G. *Physical Phenomena Associated with Unsteady Transonic Flow and Its Structural Interaction*. AGARD Report / Prog. Aerospace Sci. (1989).
9. Unity, Cinemachine 3.1 manual, [*Orbital Follow*](https://docs.unity3d.com/Packages/com.unity.cinemachine@3.1/manual/CinemachineOrbitalFollow.html). The Unity package catalog listed Cinemachine 3.1.5 for Unity 6.0 at access; the manual page has no separate update date. Accessed 2026-09-23. Applicability: documents independent position damping and the tradeoff between per-axis angular damping and quaternion damping. Limit: Unity component behavior is a design reference, not a dependency or tuning authority for this Rust camera.
10. Unity, Cinemachine 3.1 manual, [*Deoccluder*](https://docs.unity3d.com/Packages/com.unity.cinemachine@3.1/manual/CinemachineDeoccluder.html). The Unity package catalog listed Cinemachine 3.1.5 for Unity 6.0 at access; the manual page has no separate update date. Accessed 2026-09-23. Applicability: documents camera radius and distinct obstruction/return damping. Limit: its raycasts require colliders; this project uses a conservative height-field sweep instead.
11. Unity, Cinemachine 3.1 manual, [*Position Composer*](https://docs.unity3d.com/Packages/com.unity.cinemachine@3.1/manual/CinemachinePositionComposer.html). The Unity package catalog listed Cinemachine 3.1.5 for Unity 6.0 at access; the manual page has no separate update date. Accessed 2026-09-23. Applicability: documents velocity lookahead and per-axis positional damping. Limit: warns that noisy target motion can amplify into camera jitter; the project caps its lookahead at 3 m and 35 ms.
