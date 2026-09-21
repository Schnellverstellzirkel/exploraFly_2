// Exhaust plume + vortex trail simulation. Fixed pools, no per-frame alloc.
// Runs at SIM_STEP (144 Hz) next to Pose::step. Render lerps prev/curr.
// Physics refs: Schumann 1996 Schmidt-Appleman, Prandtl 1904 cell length
// reviewed by Powell 2010, Lamb-Oseen vortex pair plus Crow 1970 wave,
// Henyey-Greenstein 1941 plus Cornette-Shanks plus NVIDIA 2023 HG+Draine
// Mie approx, Gladstone-Dale refraction, Nubis 2015/2017/2023 noise recipe.

use glam::Vec3;

// ISA lapse to 22 km. Matches Pose clamp 130..22000 m in flight.rs.
pub const LAPSE: f32 = 0.0065;
pub const T0: f32 = 288.15;
pub const P0: f32 = 101325.0;
// Fuel defaults from Wolf et al 2024 / Schumann 1996.
pub const FUEL_Q: f32 = 43.2e6;
pub const FUEL_EI_H2O: f32 = 1.25;
pub const PROP_ETA: f32 = 0.3;

pub const EMITTER_NOZZLE: usize = 0;
pub const EMITTER_TIP_L: usize = 1;
pub const EMITTER_TIP_R: usize = 2;
pub const EMITTER_FLAP_L: usize = 3;
pub const EMITTER_FLAP_R: usize = 4;
pub const EMITTER_COUNT: usize = 5;
// 8192 segments per emitter holds minutes at distance-based emission.
pub const POOL_N: usize = 8192;
pub const CROW_WAVELENGTH_FACTOR: f32 = 8.6;

/// Nubis remap. Preserves core density where multiply would collapse it.
// base in [0,1], detail in [0,1], coverage in [0,1].
pub fn nubis_remap(base: f32, detail: f32, coverage: f32, from: f32, to: f32) -> f32 {
    // From Schneider 2017 slides: remap(base, detail, 1, 0, 1) erodes edges.
    // General form: x = base * (1-coverage) + detail * coverage, then affine to [from,to].
    let x = base * (1.0 - coverage) + detail * coverage;
    (from + (to - from) * x).clamp(from.min(to), from.max(to))
}

pub fn saturate(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// ISA temperature at altitude m.
pub fn isa_temperature(alt_m: f32) -> f32 {
    // Troposphere lapse to 11 km, isothermal above. Enough for visuals.
    if alt_m < 11000.0 {
        T0 - LAPSE * alt_m
    } else {
        T0 - LAPSE * 11000.0
    }
}

/// ISA pressure with barometric formula.
pub fn isa_pressure(alt_m: f32) -> f32 {
    let t = isa_temperature(alt_m);
    if alt_m < 11000.0 {
        P0 * (t / T0).powf(5.25588)
    } else {
        let p11 = P0 * ((T0 - LAPSE * 11000.0) / T0).powf(5.25588);
        let h = alt_m - 11000.0;
        p11 * (-9.80665 * 0.0289644 * h / (8.31447 * t)).exp()
    }
}

pub fn isa_density(alt_m: f32) -> f32 {
    isa_pressure(alt_m) / (287.05 * isa_temperature(alt_m))
}

/// Schmidt-Appleman mixing slope G after Schumann 1996.
// G = EI_H2O * p * M_air / (Q * (1-eta) * M_H2O), scaled to Pa/K.
pub fn mixing_slope_g(pressure_pa: f32) -> f32 {
    // M_air/M_H2O = 28.9644/18.01528 = 1.6078.
    FUEL_EI_H2O * pressure_pa * 1.6078 / (FUEL_Q * (1.0 - PROP_ETA))
}

/// Threshold temperature for contrail formation (approx tangent construction).
/// Returns critical ambient temperature in K. Below this, exhaust mixture
/// reaches liquid saturation. Uses Schumann 1996 tangent plus Gierens 2021
/// high-T extension simplified to a closed form fit valid 15000..45000 Pa.
pub fn contrail_t_crit(pressure_pa: f32, rh_liquid_01: f32) -> f32 {
    // Fit anchors: at 250 hPa, rh=0 -> Tcrit ~ 233 K; rh=0.6 -> ~238 K.
    // Slope dT/dln(p) ~ 12 K per doubling in this band.
    let g = mixing_slope_g(pressure_pa);
    // Reference from Rap et al 2010 tables linearized around 230 K.
    let t0 = 226.0 + 8.5 * (g / 1.6e-3).ln_1p();
    // Humidity raises threshold: moist air needs less cooling.
    t0 + 9.0 * rh_liquid_01
}

/// Persistence needs ice supersaturation: r_ice > 100%.
pub fn contrail_persistent(rh_ice_01: f32) -> bool {
    rh_ice_01 > 1.0
}

/// Prandtl 1904 shock cell spacing: lambda = 1.306 * d * sqrt(Mj^2 - 1).
/// d is nozzle exit diameter m, Mj is fully expanded jet Mach.
pub fn shock_cell_spacing(diameter_m: f32, mj: f32) -> f32 {
    if mj <= 1.0 {
        return 0.0;
    }
    1.306 * diameter_m * (mj * mj - 1.0).sqrt()
}

/// Fully expanded Mach from pressure ratio (isentropic, gamma=1.4).
pub fn jet_mach_from_npr(npr: f32) -> f32 {
    if npr <= 1.0 {
        return 0.0;
    }
    // M^2 = (2/(g-1)) * (NPR^((g-1)/g) - 1).
    let m2 = 5.0 * (npr.powf(0.285714) - 1.0);
    m2.max(0.0).sqrt()
}

/// Nozzle pressure ratio from spool and altitude.
pub fn nozzle_pressure_ratio(spool_01: f32, ambient_pa: f32) -> f32 {
    // Idle exit ~1.2x ambient, mil ~2.2x, full burner ~4.5x. Tuned to show
    // diamonds only at high spool, matching X-59 night burner photos.
    let exit_pa = ambient_pa * (1.15 + spool_01 * spool_01 * 3.4);
    exit_pa / ambient_pa.max(1.0)
}

/// Lamb-Oseen tangential velocity for one vortex.
pub fn lamb_oseen_vtheta(gamma: f32, r: f32, rc: f32) -> f32 {
    if r < 1e-4 {
        return 0.0;
    }
    gamma / (std::f32::consts::TAU * r) * (1.0 - (-r * r / (rc * rc).max(1e-6)).exp())
}

/// Circulation from lift: Gamma = L / (rho * V * b_eff).
pub fn circulation(lift_n: f32, rho: f32, speed_ms: f32, span_m: f32) -> f32 {
    lift_n / (rho * speed_ms.max(1.0) * span_m.max(0.5))
}

#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    pub life: f32,
    pub radius: f32,
    pub density: f32,
    /// Emission density. Age erosion scales from this via nubis_remap.
    pub density0: f32,
    pub ice: f32,
    pub flow_uv: [f32; 2],
    pub seed: f32,
}

impl Default for Segment {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            age: 0.0,
            life: 1.0,
            radius: 0.2,
            density: 0.0,
            density0: 0.0,
            ice: 0.0,
            flow_uv: [0.0, 0.0],
            seed: 0.0,
        }
    }
}

pub struct TrailPool {
    pub segs: Vec<Segment>,
    pub head: usize,
    pub live: usize,
    pub last_emit_pos: Vec3,
    pub has_last: bool,
}

impl Default for TrailPool {
    fn default() -> Self {
        Self::new()
    }
}

impl TrailPool {
    pub fn new() -> Self {
        Self {
            segs: vec![Segment::default(); POOL_N],
            head: 0,
            live: 0,
            last_emit_pos: Vec3::ZERO,
            has_last: false,
        }
    }

    /// Rebase all points when floating origin jumps.
    pub fn rebase(&mut self, delta: Vec3) {
        if delta == Vec3::ZERO {
            return;
        }
        for s in self.segs.iter_mut() {
            if s.density > 0.0 {
                s.pos -= delta;
            }
        }
        self.last_emit_pos -= delta;
    }

    fn push(&mut self, s: Segment) {
        self.segs[self.head] = s;
        self.head = (self.head + 1) % POOL_N;
        self.live = (self.live + 1).min(POOL_N);
    }

    /// Advect + age. No alloc. Called at 144 Hz.
    pub fn step(&mut self, dt: f32, wind: Vec3, downwash: f32) {
        for s in self.segs.iter_mut() {
            if s.density <= 0.0 {
                continue;
            }
            s.age += dt;
            if s.age >= s.life {
                s.density = 0.0;
                continue;
            }
            // A deposited wake starts at its emitted opacity and approaches
            // zero continuously. Noise breakup belongs in the shader: mixing
            // a seed into lifetime used to drop density immediately at birth
            // and leave an opaque residual that popped away at expiration.
            let t = (s.age / s.life).clamp(0.0, 1.0);
            s.density = s.density0 * (1.0 - t) * (1.0 - t) * (1.0 + 2.0 * t);
            // A narrow young wake spreads gradually as it mixes with the air.
            s.radius += (0.06 + 0.10 * s.ice) * dt;
            // Buoyant rise for warm exhaust, sink for pair downwash.
            s.vel.y += (0.25 * s.ice - downwash * 0.15) * dt;
            s.vel = s.vel.lerp(wind, 1.0 - (-0.4 * dt).exp());
            s.pos += s.vel * dt;
            // Flow-map UV drifts slowly so noise sticks to fluid.
            // Emission time stays fixed: shader detail follows the deposited wake.
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EmitterState {
    pub pos: Vec3,
    pub prev_pos: Vec3,
    pub dir: Vec3,
    pub strength: f32,
}

impl Default for EmitterState {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            prev_pos: Vec3::ZERO,
            dir: Vec3::Z,
            strength: 0.0,
        }
    }
}

pub struct PlumeParams {
    pub length_m: f32,
    pub radius_m: f32,
    pub exit_vel: f32,
    pub npr: f32,
    pub mj: f32,
    pub cell_lambda: f32,
    pub core_temp_k: f32,
    pub flicker: f32,
}

impl Default for PlumeParams {
    fn default() -> Self {
        Self {
            length_m: 3.0,
            radius_m: 0.42,
            exit_vel: 120.0,
            npr: 1.2,
            mj: 0.0,
            cell_lambda: 0.0,
            core_temp_k: 900.0,
            flicker: 0.0,
        }
    }
}

pub struct Effects {
    pub pools: Vec<TrailPool>,
    pub emitters: [EmitterState; EMITTER_COUNT],
    pub prev_emitters: [EmitterState; EMITTER_COUNT],
    pub plume: PlumeParams,
    pub origin: Vec3,
    pub time: f32,
    pub fx_enabled: bool,
    // Shaping inputs preserved for render lerp.
    pub spool: f32,
    pub speed_ms: f32,
    pub altitude_m: f32,
    pub load_g: f32,
    pub rh_ice: f32,
    /// Last tip circulation, for Lamb-Oseen swirl at emission.
    pub last_gamma: f32,
}

impl Default for Effects {
    fn default() -> Self {
        Self::new()
    }
}

impl Effects {
    pub fn new() -> Self {
        let mut pools = Vec::with_capacity(EMITTER_COUNT);
        for _ in 0..EMITTER_COUNT {
            pools.push(TrailPool::new());
        }
        Self {
            pools,
            emitters: [EmitterState::default(); EMITTER_COUNT],
            prev_emitters: [EmitterState::default(); EMITTER_COUNT],
            plume: PlumeParams::default(),
            origin: Vec3::ZERO,
            time: 0.0,
            // Engine applies the DEBUG_ONLY EXPLORA_FX policy after construction.
            fx_enabled: true,
            spool: 0.0,
            speed_ms: 70.0,
            altitude_m: 1500.0,
            load_g: 1.0,
            rh_ice: 0.4,
            last_gamma: 0.0,
        }
    }

    /// Track the render origin; trail simulation remains in absolute world space.
    pub fn set_origin(&mut self, origin: Vec3) {
        // Emitters and segments are absolute world positions. Rendering subtracts
        // its current origin; rebasing only the history would tear the trail.
        self.origin = origin;
    }

    /// Advance sim one fixed step. Emitter pos/dir are world coords.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        dt: f32,
        emitter_pos: &[Vec3; EMITTER_COUNT],
        emitter_dir: &[Vec3; EMITTER_COUNT],
        spool_01: f32,
        speed_ms: f32,
        altitude_m: f32,
        load_g: f32,
    ) {
        self.step_with_wind(
            dt,
            emitter_pos,
            emitter_dir,
            spool_01,
            speed_ms,
            altitude_m,
            load_g,
            Vec3::new(1.5, 0.0, 0.5),
        );
    }

    /// Advance effects with the same world-space air velocity used by flight.
    /// Existing vapor gradually entrains into this flow as the wake ages.
    #[allow(clippy::too_many_arguments)]
    pub fn step_with_wind(
        &mut self,
        dt: f32,
        emitter_pos: &[Vec3; EMITTER_COUNT],
        emitter_dir: &[Vec3; EMITTER_COUNT],
        spool_01: f32,
        speed_ms: f32,
        altitude_m: f32,
        load_g: f32,
        wind_velocity: Vec3,
    ) {
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }
        let wind_velocity = if wind_velocity.is_finite() {
            wind_velocity
        } else {
            Vec3::ZERO
        };
        self.time += dt;
        self.spool = spool_01;
        self.speed_ms = speed_ms;
        self.altitude_m = altitude_m;
        self.load_g = load_g;
        self.prev_emitters = self.emitters;
        for i in 0..EMITTER_COUNT {
            self.emitters[i].prev_pos = self.emitters[i].pos;
            self.emitters[i].pos = emitter_pos[i];
            self.emitters[i].dir = emitter_dir[i];
        }
        // Plume params from spool + altitude. Always visible when spool > 0.
        let ambient_p = isa_pressure(altitude_m);
        let npr = nozzle_pressure_ratio(spool_01, ambient_p);
        let mj = jet_mach_from_npr(npr);
        let lambda = shock_cell_spacing(0.86, mj);
        self.plume.npr = npr;
        self.plume.mj = mj;
        self.plume.cell_lambda = lambda;
        self.plume.length_m = 2.5 + spool_01 * spool_01 * 11.0;
        self.plume.radius_m = 0.40 + spool_01 * 0.18 + (1.0 - ambient_p / P0) * 0.25;
        self.plume.exit_vel = 120.0 + spool_01 * 480.0;
        self.plume.core_temp_k = 800.0 + spool_01 * 1300.0;
        self.plume.flicker =
            (self.time * 57.0).sin() * 0.5 + (self.time * 91.0).sin() * 0.3 + (self.time * 23.0).sin() * 0.2;

        // Humidity proxy: moist near 8-12 km, dry above/below. Physical shaping
        // only; visibility floor keeps vapor readable per user rule.
        let h = altitude_m;
        let rh_ice = if h < 3000.0 {
            0.25
        } else if h < 8000.0 {
            0.55 + 0.2 * ((h - 3000.0) / 5000.0)
        } else if h < 13000.0 {
            0.95
        } else {
            0.6
        };
        self.rh_ice = rh_ice;

        // Vortex strength per emitter.
        let rho = isa_density(altitude_m);
        // Lift ~ load * weight proxy. Weight proxy constant keeps units stable.
        let lift = load_g.max(0.0) * 9000.0;
        let gamma = circulation(lift, rho, speed_ms, 19.2);
        self.last_gamma = gamma;
        let tip_strength = saturate(gamma / 28.0) * saturate(speed_ms / 55.0);
        // Flaps weaker than tips.
        let mut strengths = [0.0f32; EMITTER_COUNT];
        strengths[EMITTER_NOZZLE] = saturate(spool_01 * 1.2);
        strengths[EMITTER_TIP_L] = tip_strength;
        strengths[EMITTER_TIP_R] = tip_strength;
        strengths[EMITTER_FLAP_L] = tip_strength * 0.45;
        strengths[EMITTER_FLAP_R] = tip_strength * 0.45;
        for (emitter, strength) in self.emitters.iter_mut().zip(strengths) {
            emitter.strength = strength;
        }

        if !self.fx_enabled {
            return;
        }
        for i in 0..EMITTER_COUNT {
            self.pools[i].step(dt, wind_velocity, if i == 0 { 0.0 } else { gamma * 0.02 });
        }
        // Emit sub-meter samples along the swept path, including at boost speed.
        for i in 0..EMITTER_COUNT {
            let st = self.emitters[i].strength;
            if st < 0.02 {
                continue;
            }
            let has_last = self.pools[i].has_last;
            if !has_last {
                self.pools[i].last_emit_pos = emitter_pos[i];
                self.pools[i].has_last = true;
                continue;
            }
            let dist = (emitter_pos[i] - self.pools[i].last_emit_pos).length();
            // Higher strength emits more often. Nozzle emits fastest.
            let spacing = if i == 0 { 0.6 } else { 0.45 };
            let start = self.pools[i].last_emit_pos;
            let count = (dist / spacing).floor() as usize;
            for sample in 1..=count.min(64) {
                let emit_pos = start.lerp(emitter_pos[i], sample as f32 * spacing / dist);
                let is_nozzle = i == 0;
                let tcrit = contrail_t_crit(ambient_p, 0.3);
                let tamb = isa_temperature(altitude_m);
                let forms = tamb < tcrit;
                // Forced visibility floor: density never zero when strength high,
                // but physics scales width and life.
                let phys = if forms { 1.0 } else { 0.35 };
                let seed = i as f32 * 0.173;
                let rh = self.rh_ice;
                // Coherent drift along the wake, not independent kicks per vertex.
                let phase = self.time * 0.6 + i as f32;
                let drift = Vec3::new(phase.sin(), 0.2 * (phase * 0.7).cos(), 0.0) * 0.08;
                let back_vel = if is_nozzle { emitter_dir[i] * 8.0 } else { Vec3::ZERO };
                let life = if is_nozzle {
                    // Exhaust dissipates in seconds unless cold + moist aloft.
                    2.5 + phys * 6.0 * saturate((9000.0 - (altitude_m - 9000.0).abs()) / 9000.0)
                } else if contrail_persistent(rh) || altitude_m > 8000.0 {
                    150.0
                } else {
                    // Maneuver vapor: seconds.
                    2.0 + st * 2.5
                };
                let seg = Segment {
                    pos: emit_pos,
                    vel: back_vel + drift,
                    age: (dist - sample as f32 * spacing) / speed_ms.max(1.0),
                    life,
                    radius: if is_nozzle {
                        0.18
                    } else {
                        0.055
                    },
                    density: st * phys.max(0.25),
                    density0: st * phys.max(0.25),
                    ice: if is_nozzle {
                        saturate((tcrit - tamb) / 25.0)
                    } else {
                        saturate(st * (0.4 + 0.6 * rh))
                    },
                    flow_uv: [self.time - (dist - sample as f32 * spacing) / speed_ms.max(1.0), 0.0],
                    seed,
                };
                self.pools[i].push(seg);
                self.pools[i].last_emit_pos = emit_pos;
            }
        }
    }

    pub fn live_count(&self, e: usize) -> usize {
        self.pools[e].live.min(POOL_N)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vapor_starts_at_emitted_density_and_fades_before_expiration() {
        let mut pool = TrailPool::new();
        pool.push(Segment {
            density: 0.8,
            density0: 0.8,
            life: 4.0,
            seed: 0.7,
            ..Segment::default()
        });
        pool.step(0.001, Vec3::ZERO, 0.0);
        assert!(pool.segs[0].density > 0.799, "fresh wake must not lose half its opacity");
        let mut previous = pool.segs[0].density;
        for _ in 0..398 {
            pool.step(0.01, Vec3::ZERO, 0.0);
            let next = pool.segs[0].density;
            assert!(next >= 0.0 && next <= previous);
            previous = next;
        }
        assert!(previous < 0.001, "wake must already be transparent before expiry");
        pool.step(0.03, Vec3::ZERO, 0.0);
        assert_eq!(pool.segs[0].density, 0.0);
    }

    fn effects_with_stationary_trail() -> Effects {
        let mut fx = Effects::new();
        fx.fx_enabled = true;
        fx.pools[EMITTER_NOZZLE].push(Segment {
            density: 1.0,
            density0: 1.0,
            life: 10.0,
            ..Segment::default()
        });
        fx
    }

    #[test]
    fn shared_wind_advects_existing_trails_in_world_space() {
        let mut calm = effects_with_stationary_trail();
        let mut windy = effects_with_stationary_trail();
        let positions = [Vec3::ZERO; EMITTER_COUNT];
        let directions = [Vec3::NEG_Z; EMITTER_COUNT];
        for _ in 0..144 {
            calm.step_with_wind(
                1.0 / 144.0, &positions, &directions, 0.0, 70.0, 1500.0, 1.0, Vec3::ZERO,
            );
            windy.step_with_wind(
                1.0 / 144.0, &positions, &directions, 0.0, 70.0, 1500.0, 1.0, Vec3::X * 10.0,
            );
        }
        let calm_segment = calm.pools[EMITTER_NOZZLE].segs[0];
        let windy_segment = windy.pools[EMITTER_NOZZLE].segs[0];
        assert_eq!(calm_segment.pos, Vec3::ZERO);
        assert!(windy_segment.pos.x > 1.0 && windy_segment.pos.x < 3.0);
        assert!(windy_segment.vel.x > 3.0 && windy_segment.vel.x < 4.0);
        assert_eq!(windy_segment.pos.y, 0.0);
        assert_eq!(windy_segment.pos.z, 0.0);
        assert_eq!(windy_segment.age, calm_segment.age);
        assert_eq!(windy_segment.density, calm_segment.density);
    }

    #[test]
    fn default_effects_step_retains_its_existing_breeze() {
        let mut legacy = effects_with_stationary_trail();
        let mut explicit = effects_with_stationary_trail();
        let positions = [Vec3::ZERO; EMITTER_COUNT];
        let directions = [Vec3::NEG_Z; EMITTER_COUNT];
        legacy.step(0.1, &positions, &directions, 0.0, 70.0, 1500.0, 1.0);
        explicit.step_with_wind(
            0.1, &positions, &directions, 0.0, 70.0, 1500.0, 1.0, Vec3::new(1.5, 0.0, 0.5),
        );
        assert_eq!(legacy.pools[0].segs[0].pos, explicit.pools[0].segs[0].pos);
        assert_eq!(legacy.pools[0].segs[0].vel, explicit.pools[0].segs[0].vel);
        assert_eq!(legacy.time, explicit.time);
    }

    #[test]
    fn invalid_timestep_or_wind_cannot_corrupt_trails() {
        let mut fx = effects_with_stationary_trail();
        let positions = [Vec3::ZERO; EMITTER_COUNT];
        let directions = [Vec3::NEG_Z; EMITTER_COUNT];
        for dt in [0.0, -0.1, f32::NAN, f32::INFINITY] {
            fx.step_with_wind(
                dt, &positions, &directions, 0.0, 70.0, 1500.0, 1.0, Vec3::X * 10.0,
            );
        }
        assert_eq!(fx.time, 0.0);
        assert_eq!(fx.pools[0].segs[0].age, 0.0);
        for wind in [Vec3::splat(f32::NAN), Vec3::splat(f32::INFINITY)] {
            fx.step_with_wind(
                1.0 / 144.0, &positions, &directions, 0.0, 70.0, 1500.0, 1.0, wind,
            );
            assert_eq!(fx.pools[0].segs[0].pos, Vec3::ZERO);
            assert_eq!(fx.pools[0].segs[0].vel, Vec3::ZERO);
        }
    }

    // CPU mirrors of the GLSL phase functions in plume/trail shaders.
    // Kept here (not in prod code) so tests pin the same math the GPU runs.
    fn phase_hg(mu: f32, g: f32) -> f32 {
        let gg = g * g;
        let denom = 1.0 + gg - 2.0 * g * mu;
        (1.0 - gg) / (12.566371 * denom.powf(1.5).max(1e-6))
    }

    fn phase_cs(mu: f32, g: f32) -> f32 {
        let gg = g * g;
        let p1 = 1.5 * (1.0 - gg) / (2.0 + gg);
        let p2 = (1.0 + mu * mu) / (1.0 + gg - 2.0 * g * mu).powf(1.5).max(1e-6);
        p1 * p2 / 12.566371
    }

    fn phase_mie_approx(mu: f32, g_hg: f32, g_draine: f32, w: f32) -> f32 {
        let draine = (1.0 + 0.5 * mu * mu)
            / (1.0 + g_draine * g_draine - 2.0 * g_draine * mu)
                .powf(1.5)
                .max(1e-6);
        let norm = (1.0 - g_draine * g_draine) / 12.566371;
        w * draine * norm + (1.0 - w) * phase_hg(mu, g_hg)
    }

    fn phase_double_hg(mu: f32) -> f32 {
        0.85 * phase_hg(mu, 0.75) + 0.15 * phase_hg(mu, -0.25)
    }

    /// Gladstone-Dale index offset: n-1 = K * rho. K ~ 0.23e-3 m3/kg visible.
    /// Reference for the deferred heat-shimmer pass ( Gladstone-Dale 1863 ).
    fn gladstone_dale_n(rho: f32) -> f32 {
        1.0 + 0.23e-3 * rho
    }

    #[test]
    fn isa_anchors_hold() {
        assert!((isa_temperature(0.0) - 288.15).abs() < 0.01);
        assert!((isa_pressure(0.0) - 101325.0).abs() < 1.0);
        assert!(isa_pressure(10000.0) < 30000.0);
    }

    #[test]
    fn schmidt_appleman_orders_hold() {
        let p = 25000.0;
        let g = mixing_slope_g(p);
        assert!(g > 0.5e-3 && g < 4.0e-3, "G={g}");
        let cold = contrail_t_crit(p, 0.0);
        let moist = contrail_t_crit(p, 0.6);
        assert!(moist > cold);
        assert!(cold > 220.0 && cold < 245.0);
    }

    #[test]
    fn prandtl_spacing_grows_with_mach() {
        let l1 = shock_cell_spacing(0.86, 1.4);
        let l2 = shock_cell_spacing(0.86, 1.9);
        assert!(l2 > l1 && l1 > 0.0);
        assert_eq!(shock_cell_spacing(0.86, 0.9), 0.0);
    }

    #[test]
    fn phases_are_normalized_positive() {
        for mu in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            assert!(phase_hg(mu, 0.6) > 0.0);
            assert!(phase_cs(mu, 0.6) > 0.0);
            assert!(phase_mie_approx(mu, 0.6, 0.8, 0.5) > 0.0);
            assert!(phase_double_hg(mu) > 0.0);
        }
        // Forward peak dominates.
        assert!(phase_hg(1.0, 0.6) > phase_hg(0.0, 0.6) * 3.0);
        // Sea-level air bends visible light by ~0.28e-3.
        assert!((gladstone_dale_n(1.225) - 1.0002817).abs() < 1e-6);
    }

    #[test]
    fn lamb_oseen_finite_at_core() {
        assert_eq!(lamb_oseen_vtheta(20.0, 0.0, 0.2), 0.0);
        let v = lamb_oseen_vtheta(20.0, 0.5, 0.2);
        assert!(v.is_finite() && v > 0.0);
    }

    #[test]
    fn fast_flight_emits_evenly_spaced_airborne_vapor() {
        let mut fx = Effects::new();
        let mut positions = [Vec3::new(0.0, 1500.0, 0.0); EMITTER_COUNT];
        let dirs = [Vec3::NEG_Z; EMITTER_COUNT];
        fx.step(1.0 / 144.0, &positions, &dirs, 1.0, 1030.0, 1500.0, 3.5);
        for p in &mut positions { p.z += 7.0; }
        fx.step(1.0 / 144.0, &positions, &dirs, 1.0, 1030.0, 1500.0, 3.5);
        let pool = &fx.pools[EMITTER_TIP_L];
        assert_eq!(pool.live, 15);
        for pair in pool.segs[..pool.live].windows(2) {
            assert!((pair[1].pos.distance(pair[0].pos) - 0.45).abs() < 0.001);
            assert!(pair[1].vel.length() < 5.0, "vapor must not chase the plane");
            assert!(pair[1].vel.distance(pair[0].vel) < 0.001, "adjacent samples must drift coherently");
            assert!(pair[0].age > pair[1].age, "substep ages must follow the emission path");
        }
    }

    #[test]
    fn pool_emit_and_rebase() {
        let mut fx = Effects::new();
        let p = [
            Vec3::new(0.0, 1500.0, 0.0),
            Vec3::new(-9.6, 1500.0, 0.0),
            Vec3::new(9.6, 1500.0, 0.0),
            Vec3::new(-5.0, 1500.0, 0.0),
            Vec3::new(5.0, 1500.0, 0.0),
        ];
        let d = [Vec3::NEG_Z; 5];
        for k in 0..300 {
            let mut pp = p;
            for e in pp.iter_mut() {
                e.z += k as f32 * 0.8;
            }
            fx.step(1.0 / 144.0, &pp, &d, 1.0, 300.0, 10000.0, 1.5);
        }
        assert!(fx.pools[0].live > 10);
        let before = fx.pools[1].segs[0].pos;
        fx.set_origin(Vec3::new(20000.0, 1500.0, 0.0));
        assert_eq!(fx.pools[1].segs[0].pos, before, "render-origin shifts must not move world-space trails");
        fx.pools[1].rebase(Vec3::new(100.0, 0.0, 0.0));
        assert!((fx.pools[1].segs[0].pos - (before - Vec3::new(100.0, 0.0, 0.0))).length() < 0.01);
    }

    #[test]
    fn nubis_remap_preserves_core() {
        // Core stays dense where multiply would collapse.
        let r = nubis_remap(0.9, 0.1, 0.5, 0.0, 1.0);
        assert!(r > 0.4, "r={r}");
        assert_eq!(nubis_remap(0.0, 0.0, 0.0, 0.0, 1.0), 0.0);
    }
}
