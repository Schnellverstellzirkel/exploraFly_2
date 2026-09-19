//! CPU-built atmosphere lookup tables used by the Hillaire sky integrator.
//!
//! The tables are generated once at startup.  They contain only transport
//! terms; the sun irradiance remains a per-frame input, which keeps the LUTs
//! valid when exposure or the sun direction changes at runtime.

use glam::Vec3;

pub const TRANSMITTANCE_WIDTH: usize = 256;
pub const TRANSMITTANCE_HEIGHT: usize = 64;
pub const MULTISCATTERING_WIDTH: usize = 32;
pub const MULTISCATTERING_HEIGHT: usize = 32;

const PI: f32 = std::f32::consts::PI;
const BOTTOM_RADIUS: f32 = 6_360_000.0;
const TOP_RADIUS: f32 = 6_460_000.0;
const RAYLEIGH_SCALE_HEIGHT: f32 = 8_000.0;
const MIE_SCALE_HEIGHT: f32 = 1_200.0;
const OZONE_LAYER_TOP: f32 = 25_000.0;
const PLANET_RADIUS_OFFSET: f32 = 0.01;
const SAMPLE_SEGMENT_T: f32 = 0.3;

const BETA_RAYLEIGH: Vec3 = Vec3::new(5.802e-6, 13.558e-6, 33.1e-6);
const BETA_MIE_SCATTER: Vec3 = Vec3::splat(3.996e-6);
const BETA_MIE_EXTINCTION: Vec3 = Vec3::splat(4.440e-6);
const BETA_OZONE: Vec3 = Vec3::new(0.650e-6, 1.881e-6, 0.085e-6);

#[derive(Clone, Copy)]
struct Medium {
    scattering: Vec3,
    extinction: Vec3,
}

pub struct AtmosphereLuts {
    pub transmittance: Vec<[f32; 4]>,
    pub multiscattering: Vec<[f32; 4]>,
}

#[inline]
fn exp3(v: Vec3) -> Vec3 {
    Vec3::new(v.x.exp(), v.y.exp(), v.z.exp())
}

#[inline]
fn clamp3(v: Vec3, lo: f32, hi: f32) -> Vec3 {
    Vec3::new(
        v.x.clamp(lo, hi),
        v.y.clamp(lo, hi),
        v.z.clamp(lo, hi),
    )
}

#[inline]
fn ray_sphere_nearest(origin: Vec3, direction: Vec3, radius: f32) -> f32 {
    let b = origin.dot(direction);
    let c = origin.length_squared() - radius * radius;
    let discriminant = b * b - c;
    if discriminant < 0.0 {
        return -1.0;
    }
    let s = discriminant.sqrt();
    let near = -b - s;
    let far = -b + s;
    if near >= 0.0 {
        near
    } else if far >= 0.0 {
        far
    } else {
        -1.0
    }
}

#[inline]
fn distance_to_top(radius: f32, mu: f32) -> f32 {
    let discriminant = radius * radius * (mu * mu - 1.0) + TOP_RADIUS * TOP_RADIUS;
    (-radius * mu + discriminant.max(0.0).sqrt()).max(0.0)
}

#[inline]
fn ozone_density(height: f32) -> f32 {
    // Hillaire's two linear density layers form a 10--40 km tent centered at
    // 25 km, rather than a Gaussian with a non-zero sea-level tail.
    let value = if height < OZONE_LAYER_TOP {
        height / 15_000.0 - 2.0 / 3.0
    } else {
        -height / 15_000.0 + 8.0 / 3.0
    };
    value.clamp(0.0, 1.0)
}

#[inline]
fn medium(position: Vec3) -> Medium {
    let height = (position.length() - BOTTOM_RADIUS).max(0.0);
    let rayleigh_density = (-height / RAYLEIGH_SCALE_HEIGHT).exp().clamp(0.0, 1.0);
    let mie_density = (-height / MIE_SCALE_HEIGHT).exp().clamp(0.0, 1.0);
    let ozone = ozone_density(height);
    let rayleigh = BETA_RAYLEIGH * rayleigh_density;
    let mie = BETA_MIE_SCATTER * mie_density;
    let mie_extinction = BETA_MIE_EXTINCTION * mie_density;
    let ozone_extinction = BETA_OZONE * ozone;
    Medium {
        scattering: rayleigh + mie,
        extinction: rayleigh + mie_extinction + ozone_extinction,
    }
}

#[inline]
fn transmittance_uv(radius: f32, mu: f32) -> (f32, f32) {
    let h = (TOP_RADIUS * TOP_RADIUS - BOTTOM_RADIUS * BOTTOM_RADIUS).max(0.0).sqrt();
    let rho = (radius * radius - BOTTOM_RADIUS * BOTTOM_RADIUS).max(0.0).sqrt();
    let discriminant = radius * radius * (mu * mu - 1.0) + TOP_RADIUS * TOP_RADIUS;
    let d = (-radius * mu + discriminant.max(0.0).sqrt()).max(0.0);
    let d_min = TOP_RADIUS - radius;
    let d_max = rho + h;
    let x_mu = (d - d_min) / (d_max - d_min).max(1e-6);
    let x_r = rho / h.max(1e-6);
    (x_mu.clamp(0.0, 1.0), x_r.clamp(0.0, 1.0))
}

fn transmittance_to_top(radius: f32, mu: f32) -> Vec3 {
    let distance = distance_to_top(radius, mu);
    if distance <= 0.0 {
        return Vec3::ONE;
    }
    let dx = distance / 128.0;
    let mut optical_rayleigh = 0.0;
    let mut optical_mie = 0.0;
    let mut optical_ozone = 0.0;
    for i in 0..=128 {
        let d = i as f32 * dx;
        let ri = (d * d + 2.0 * radius * mu * d + radius * radius)
            .max(0.0)
            .sqrt();
        let height = (ri - BOTTOM_RADIUS).max(0.0);
        let weight = if i == 0 || i == 128 { 0.5 } else { 1.0 };
        optical_rayleigh += (-height / RAYLEIGH_SCALE_HEIGHT).exp().clamp(0.0, 1.0) * weight * dx;
        optical_mie += (-height / MIE_SCALE_HEIGHT).exp().clamp(0.0, 1.0) * weight * dx;
        optical_ozone += ozone_density(height) * weight * dx;
    }
    exp3(-(BETA_RAYLEIGH * optical_rayleigh
        + BETA_MIE_EXTINCTION * optical_mie
        + BETA_OZONE * optical_ozone))
}

fn transmittance_lut_sample(data: &[[f32; 4]], uv: (f32, f32)) -> Vec3 {
    let x = uv.0.clamp(0.0, 1.0) * (TRANSMITTANCE_WIDTH - 1) as f32;
    let y = uv.1.clamp(0.0, 1.0) * (TRANSMITTANCE_HEIGHT - 1) as f32;
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(TRANSMITTANCE_WIDTH - 1);
    let y1 = (y0 + 1).min(TRANSMITTANCE_HEIGHT - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let at = |ix: usize, iy: usize| Vec3::new(
        data[iy * TRANSMITTANCE_WIDTH + ix][0],
        data[iy * TRANSMITTANCE_WIDTH + ix][1],
        data[iy * TRANSMITTANCE_WIDTH + ix][2],
    );
    let a = at(x0, y0).lerp(at(x1, y0), tx);
    let b = at(x0, y1).lerp(at(x1, y1), tx);
    a.lerp(b, ty)
}

fn build_transmittance() -> Vec<[f32; 4]> {
    let mut data = Vec::with_capacity(TRANSMITTANCE_WIDTH * TRANSMITTANCE_HEIGHT);
    let h = (TOP_RADIUS * TOP_RADIUS - BOTTOM_RADIUS * BOTTOM_RADIUS).max(0.0).sqrt();
    for y in 0..TRANSMITTANCE_HEIGHT {
        let x_r = y as f32 / (TRANSMITTANCE_HEIGHT - 1) as f32;
        let rho = h * x_r;
        let radius = (rho * rho + BOTTOM_RADIUS * BOTTOM_RADIUS).sqrt();
        let d_min = TOP_RADIUS - radius;
        let d_max = rho + h;
        for x in 0..TRANSMITTANCE_WIDTH {
            let x_mu = x as f32 / (TRANSMITTANCE_WIDTH - 1) as f32;
            let d = d_min + x_mu * (d_max - d_min);
            let mu = if d == 0.0 {
                1.0
            } else {
                (h * h - rho * rho - d * d) / (2.0 * radius * d)
            };
            let t = transmittance_to_top(radius, mu.clamp(-1.0, 1.0));
            data.push([t.x, t.y, t.z, 1.0]);
        }
    }
    data
}

fn integrate_multiple_scattering(
    position: Vec3,
    direction: Vec3,
    sun_direction: Vec3,
    transmittance: &[[f32; 4]],
) -> (Vec3, Vec3) {
    let bottom = ray_sphere_nearest(position, direction, BOTTOM_RADIUS);
    let top = ray_sphere_nearest(position, direction, TOP_RADIUS);
    let max_distance = if bottom < 0.0 {
        top
    } else if top > 0.0 {
        bottom.min(top)
    } else {
        0.0
    };
    if max_distance <= 0.0 {
        return (Vec3::ZERO, Vec3::ZERO);
    }

    let mut luminance = Vec3::ZERO;
    let mut first_order = Vec3::ZERO;
    let mut throughput = Vec3::ONE;
    let mut previous_t = 0.0;
    for sample in 0..20 {
        let new_t = max_distance * (sample as f32 + SAMPLE_SEGMENT_T) / 20.0;
        let dt = new_t - previous_t;
        previous_t = new_t;
        let p = position + direction * new_t;
        let m = medium(p);
        let sample_transmittance = exp3(-(m.extinction * dt));
        let up = p / p.length().max(1e-6);
        let sun_mu = sun_direction.dot(up).clamp(-1.0, 1.0);
        let sun_transmittance = transmittance_lut_sample(transmittance, transmittance_uv(p.length(), sun_mu));
        let earth_shadow = if ray_sphere_nearest(p, sun_direction, BOTTOM_RADIUS) >= 0.0 {
            0.0
        } else {
            1.0
        };
        let phase = 1.0 / (4.0 * PI);
        let source = sun_transmittance * m.scattering * (phase * earth_shadow);
        let extinction = m.extinction.max(Vec3::splat(1e-4));
        luminance += throughput * ((source - source * sample_transmittance) / extinction);
        first_order += throughput * ((m.scattering - m.scattering * sample_transmittance) / extinction);
        throughput *= sample_transmittance;
    }
    (luminance, first_order)
}

fn build_multiscattering(transmittance: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let mut data = Vec::with_capacity(MULTISCATTERING_WIDTH * MULTISCATTERING_HEIGHT);
    for y in 0..MULTISCATTERING_HEIGHT {
        let uv_y = y as f32 / (MULTISCATTERING_HEIGHT - 1) as f32;
        let height = BOTTOM_RADIUS
            + (uv_y + PLANET_RADIUS_OFFSET)
                .clamp(0.0, 1.0)
                * (TOP_RADIUS - BOTTOM_RADIUS - PLANET_RADIUS_OFFSET);
        for x in 0..MULTISCATTERING_WIDTH {
            let uv_x = x as f32 / (MULTISCATTERING_WIDTH - 1) as f32;
            let sun_mu = uv_x * 2.0 - 1.0;
            let sun_direction = Vec3::new(
                0.0,
                (1.0 - sun_mu * sun_mu).max(0.0).sqrt(),
                sun_mu,
            );
            let position = Vec3::new(0.0, 0.0, height);
            let mut luminance = Vec3::ZERO;
            let mut first_order = Vec3::ZERO;
            for lane in 0..64 {
                let grid_y = lane / 8;
                let grid_x = lane - grid_y * 8;
                let rand_a = (grid_y as f32 + 0.5) / 8.0;
                let rand_b = (grid_x as f32 + 0.5) / 8.0;
                let theta = 2.0 * PI * rand_a;
                let cos_phi = 1.0 - 2.0 * rand_b;
                let sin_phi = (1.0 - cos_phi * cos_phi).max(0.0).sqrt();
                let direction = Vec3::new(theta.cos() * sin_phi, theta.sin() * sin_phi, cos_phi);
                let (l, f) = integrate_multiple_scattering(position, direction, sun_direction, transmittance);
                luminance += l;
                first_order += f;
            }
            let inv_sphere_samples = 1.0 / 64.0;
            luminance *= inv_sphere_samples;
            first_order = clamp3(first_order * inv_sphere_samples, 0.0, 0.999);
            let geometric_series = Vec3::ONE / (Vec3::ONE - first_order);
            let result = luminance * geometric_series;
            data.push([result.x, result.y, result.z, 1.0]);
        }
    }
    data
}

pub fn generate() -> AtmosphereLuts {
    let transmittance = build_transmittance();
    let multiscattering = build_multiscattering(&transmittance);
    AtmosphereLuts {
        transmittance,
        multiscattering,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transmittance_is_monotone_and_bounded() {
        let data = build_transmittance();
        for texel in &data {
            assert!(texel[..3].iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        }
        let top_zenith = data[(TRANSMITTANCE_HEIGHT - 1) * TRANSMITTANCE_WIDTH][0];
        let ground_zenith = data[0][0];
        assert!(top_zenith > ground_zenith);
    }
}
