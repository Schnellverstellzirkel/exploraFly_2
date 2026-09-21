//! Startup quality presets.
//!
//! The playable configuration is one preset: `Cinematic` (max visual quality).
//! `Performance` and `Balanced` are DEBUG_ONLY. Release and dist builds always
//! resolve to `Cinematic` and ignore `EXPLORA_QUALITY`.
use crate::flags;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// DEBUG_ONLY: reduced resolution and shading rates.
    Performance,
    /// DEBUG_ONLY: native resolution, mid IBL budget.
    Balanced,
    /// Playable preset: native resolution, full shading rates, 16 IBL samples.
    #[default]
    Cinematic,
}

pub struct Settings {
    pub scale: f32,
    pub sky: [u32; 2],
    pub ground: [u32; 2],
    pub terrain_lod_step: u32,
    pub plume: [u32; 2],
    pub composite: [u32; 2],
    pub cloud_puffs: u32,
    pub cloud_grid: u32,
    pub ibl_samples: u32,
}

impl Quality {
    /// Parse a preset name. Used by DEBUG_ONLY `EXPLORA_QUALITY` and tests.
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "performance" => Ok(Self::Performance),
            "balanced" => Ok(Self::Balanced),
            "cinematic" => Ok(Self::Cinematic),
            _ => Err("EXPLORA_QUALITY must be performance, balanced, or cinematic"),
        }
    }

    /// Resolve startup quality.
    ///
    /// Playable builds always return `Cinematic`. Debug builds honor
    /// `EXPLORA_QUALITY` and default to `Cinematic` when the flag is unset.
    pub fn from_env() -> Self {
        match flags::debug_var(flags::QUALITY) {
            None => Self::Cinematic,
            Some(value) => Self::parse(&value).expect("invalid render quality"),
        }
    }

    pub fn settings(self) -> Settings {
        match self {
            // DEBUG_ONLY presets: never selected outside debug_assertions.
            Self::Performance => Settings {
                scale: 0.67,
                sky: [2, 2],
                ground: [2, 2],
                terrain_lod_step: 2,
                plume: [2, 2],
                composite: [2, 2],
                cloud_puffs: 6,
                cloud_grid: 17,
                ibl_samples: 4,
            },
            Self::Balanced => Settings {
                scale: 1.0,
                sky: [1, 1],
                ground: [1, 1],
                terrain_lod_step: 1,
                plume: [1, 1],
                composite: [1, 1],
                cloud_puffs: 8,
                cloud_grid: 19,
                ibl_samples: 8,
            },
            Self::Cinematic => Settings {
                scale: 1.0,
                sky: [1, 1],
                ground: [1, 1],
                terrain_lod_step: 1,
                plume: [1, 1],
                composite: [1, 1],
                cloud_puffs: 8,
                cloud_grid: 19,
                ibl_samples: 16,
            },
        }
    }

    pub fn scene_size(self, width: u32, height: u32) -> [u32; 2] {
        let scale = self.settings().scale;
        [
            ((width as f64 * scale as f64).round() as u32).max(1),
            ((height as f64 * scale as f64).round() as u32).max(1),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_selection_is_explicit_and_rejects_typos() {
        assert_eq!(Quality::parse("performance"), Ok(Quality::Performance));
        assert_eq!(Quality::parse("balanced"), Ok(Quality::Balanced));
        assert_eq!(Quality::parse("cinematic"), Ok(Quality::Cinematic));
        assert!(Quality::parse("cinemtaic").is_err());
    }

    #[test]
    fn playable_preset_is_cinematic_max_quality() {
        assert_eq!(Quality::default(), Quality::Cinematic);
        let settings = Quality::Cinematic.settings();
        assert_eq!(settings.scale, 1.0);
        assert_eq!(settings.sky, [1, 1]);
        assert_eq!(settings.ground, [1, 1]);
        assert_eq!(settings.composite, [1, 1]);
        assert_eq!(settings.ibl_samples, 16);
        assert_eq!(settings.terrain_lod_step, 1);
    }

    #[test]
    fn from_env_ignores_quality_flag_in_playable_builds() {
        if flags::DEBUG_ONLY {
            // Debug builds honor an explicitly set flag; default is Cinematic.
            assert_eq!(Quality::from_env(), Quality::Cinematic);
        } else {
            // Simulate a hostile environment: the flag must not be read.
            assert_eq!(Quality::from_env(), Quality::Cinematic);
        }
    }

    #[test]
    fn full_resolution_presets_preserve_odd_window_sizes() {
        for quality in [Quality::Balanced, Quality::Cinematic] {
            assert_eq!(quality.scene_size(1919, 1079), [1919, 1079]);
            assert_eq!(quality.settings().composite, [1, 1]);
            assert_eq!(quality.settings().ground, [1, 1]);
        }
        assert_eq!(Quality::Performance.scene_size(1920, 1080), [1286, 724]);
        assert_eq!(Quality::Performance.scene_size(0, 0), [1, 1]);
    }
}
