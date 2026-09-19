//! Directional albedo energy preservation Look-Up Table (LUT).
//!
//! Evaluates directional hemispherical reflectance $E(\mu_o, \alpha)$ for
//! height-correlated Smith GGX microfacet BRDFs. Microfacet specular models
//! lose energy at grazing angles due to multiple scattering between facets;
//! this precomputed $32 \times 32$ table compensates for single-scattering
//! losses without requiring costly real-time multiple-scattering integrals.

use glam::Vec3;

/// Grid resolution along each parameter axis ($\mu_o$ and $\alpha$).
pub const ENERGY_LUT_SIZE: usize = 32;

/// Number of Hammersley quasi-Monte Carlo samples evaluated per LUT cell.
pub const ENERGY_LUT_SAMPLES: u32 = 4096;

/// Fresnel-free directional albedo for height-correlated Smith GGX.
/// Uses deterministic Hammersley NDF quadrature, once at initialization.
/// The same visibility is used in shaders/plane.frag; anisotropic compensation uses
/// the geometric mean alpha as an approximation.
pub fn energy_lut() -> Vec<f32> {
    const N: usize = ENERGY_LUT_SIZE;
    const SAMPLES: u32 = ENERGY_LUT_SAMPLES;
    let mut lut = vec![1.0f32; N * N];
    for j in 0..N {
        let alpha = (j as f32 + 0.5) / N as f32;

        for i in 0..N {
            let mu_o = (i as f32 + 0.5) / N as f32;
            let sin_o = (1.0 - mu_o * mu_o).max(0.0).sqrt();
            let o = Vec3::new(sin_o, 0.0, mu_o);
            let mut acc = 0.0f32;
            for sample in 0..SAMPLES {
                let xi1 = (sample as f32 + 0.5) / SAMPLES as f32;
                let xi2 = sample.reverse_bits() as f32 * (1.0 / 4294967296.0);
                let cos_h = ((1.0 - xi1) / (1.0 + (alpha * alpha - 1.0) * xi1)).sqrt();
                let sin_h = (1.0 - cos_h * cos_h).max(0.0).sqrt();
                let phi = std::f32::consts::TAU * xi2;
                let h = Vec3::new(sin_h * phi.cos(), sin_h * phi.sin(), cos_h);
                let oh = o.dot(h);
                let inc = 2.0 * oh * h - o;
                if inc.z <= 0.0 {
                    continue; // reflected light direction below the surface
                }
                let root_o = (alpha * alpha * (1.0 - mu_o * mu_o) + mu_o * mu_o).sqrt();
                let root_i = (alpha * alpha * (1.0 - inc.z * inc.z) + inc.z * inc.z).sqrt();
                let g2 = 2.0 * mu_o * inc.z / (inc.z * root_o + mu_o * root_i);
                acc += g2 * oh / (mu_o * cos_h);
            }
            lut[j * N + i] = acc / SAMPLES as f32;
        }
    }
    lut
}

/// Average albedo Eavg = integrate Ess over mu with weight 2 mu, for tests.
#[cfg(test)]
pub fn energy_avg(lut: &[f32], j: usize) -> f32 {
    const N: usize = ENERGY_LUT_SIZE;
    let mut acc = 0.0;
    for i in 0..N {
        let mu = (i as f32 + 0.5) / N as f32;
        acc += 2.0 * mu * lut[j * N + i];
    }
    acc / N as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_energy_lut_anchors() {
        let lut = energy_lut();
        const N: usize = ENERGY_LUT_SIZE;
        assert!(lut[N - 1] > 0.97, "mirror must conserve energy");
        // Exact correlated Smith at alpha=1 and normal incidence:
        // directional albedo = 1 - ln(2), approximately 0.30685.
        assert!((lut[N * N - 1] - (1.0 - 2.0f32.ln())).abs() < 0.025);
        // At grazing, correlated masking tends to unit directional albedo.
        assert!(lut[(N - 1) * N] > 0.90);
        assert!((0.40..0.48).contains(&energy_avg(&lut, N - 1)));
        for value in lut {
            assert!(value.is_finite() && value > 0.0 && value <= 1.05);
        }
    }
}
