//! Startup presets keep image-quality choices consistent across resize and pipelines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    Performance,
    #[default]
    Balanced,
    Cinematic,
}

pub struct Settings {
    pub scale: f32,
    pub sky: [u32; 2],
    pub ground: [u32; 2],
    pub clouds: [u32; 2],
    pub plume: [u32; 2],
    pub composite: [u32; 2],
    pub ibl_samples: u32,
}

impl Quality {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "performance" => Ok(Self::Performance),
            "balanced" => Ok(Self::Balanced),
            "cinematic" => Ok(Self::Cinematic),
            _ => Err("EXPLORA_QUALITY must be performance, balanced, or cinematic"),
        }
    }

    pub fn from_env() -> Self {
        std::env::var("EXPLORA_QUALITY")
            .map(|value| Self::parse(&value).expect("invalid render quality"))
            .unwrap_or_default()
    }

    pub fn settings(self) -> Settings {
        match self {
            Self::Performance => Settings {
                scale: 0.8,
                sky: [4, 4],
                ground: [4, 2],
                clouds: [4, 4],
                plume: [2, 2],
                composite: [2, 2],
                ibl_samples: 4,
            },
            Self::Balanced => Settings {
                scale: 1.0,
                // Preserve the finite solar disc at native shading resolution.
                sky: [1, 1],
                ground: [1, 1],
                clouds: [2, 2],
                plume: [1, 1],
                composite: [1, 1],
                ibl_samples: 8,
            },
            Self::Cinematic => Settings {
                scale: 1.0,
                sky: [1, 1],
                ground: [1, 1],
                clouds: [1, 1],
                plume: [1, 1],
                composite: [1, 1],
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
        assert_eq!(Quality::parse("balanced"), Ok(Quality::default()));
        assert_eq!(Quality::parse("cinematic"), Ok(Quality::Cinematic));
        assert!(Quality::parse("cinemtaic").is_err());
    }

    #[test]
    fn full_resolution_presets_preserve_odd_window_sizes() {
        for quality in [Quality::Balanced, Quality::Cinematic] {
            assert_eq!(quality.scene_size(1919, 1079), [1919, 1079]);
            assert_eq!(quality.settings().composite, [1, 1]);
            assert_eq!(quality.settings().ground, [1, 1]);
        }
        assert_eq!(Quality::Performance.scene_size(1920, 1080), [1536, 864]);
        assert_eq!(Quality::Performance.scene_size(0, 0), [1, 1]);
    }
}
