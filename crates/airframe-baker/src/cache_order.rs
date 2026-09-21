//! Triangle-order strategies for vertex-cache locality.
//!
//! The bake chain is `QEM simplify -> cache reorder -> build_meshlets(64v,
//! 126t) -> task/mesh shader`, so the order chosen here changes both vertex
//! reuse and the meshlet partitioning that follows. The production default
//! is meshoptimizer's adaptive optimizer; FIFO-16 remains a measurement
//! control. The 2006 Forsyth port was retired on 2026-09-22 after it lost
//! the A/B recorded in `docs/optimization_techniques/airframe-vertex-cache-order.md`.

use meshopt_rs::vertex::cache::{optimize_vertex_cache, optimize_vertex_cache_fifo};

/// FIFO cache size passed to [`CacheOrder::Fifo`]. meshoptimizer recommends
/// a value below the real GPU cache size to avoid thrashing.
pub const FIFO_CACHE_SIZE: u32 = 16;

/// Build-time selection of the index-buffer reorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CacheOrder {
    /// meshoptimizer adaptive score table; production default.
    Meshopt,
    /// meshoptimizer FIFO-16 control, kept for measurement comparisons.
    Fifo,
}

/// Every order the comparison harness measures, in table row order.
pub const ALL: [CacheOrder; 2] = [CacheOrder::Meshopt, CacheOrder::Fifo];

/// Environment variable the engine build script reads.
pub const ENV_VAR: &str = "EXPLORA_AIRFRAME_CACHE_ORDER";

impl CacheOrder {
    /// Order used when the environment variable is unset.
    pub const DEFAULT: Self = Self::Meshopt;

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "meshopt" => Ok(Self::Meshopt),
            "fifo" => Ok(Self::Fifo),
            "forsyth" => Err(
                "cache order \"forsyth\" was retired after the 2026-09-22 A/B; \
                 use \"meshopt\" (docs/optimization_techniques/airframe-vertex-cache-order.md)"
                    .into(),
            ),
            other => Err(format!(
                "unknown cache order {other:?}; expected meshopt or fifo"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Meshopt => "meshopt",
            Self::Fifo => "fifo",
        }
    }

    /// Selection from [`ENV_VAR`]; panics on an unrecognized value so a
    /// build-script typo cannot silently ship a different order.
    pub fn from_env() -> Self {
        match std::env::var(ENV_VAR) {
            Ok(value) => Self::parse(&value)
                .unwrap_or_else(|error| panic!("{ENV_VAR}: {error}")),
            Err(_) => Self::DEFAULT,
        }
    }

    /// Reorder a part-local index buffer. `vertex_count` is the part's
    /// vertex-buffer length, not necessarily the largest index plus one.
    pub fn reorder(self, indices: &[u32], vertex_count: usize) -> Vec<u32> {
        match self {
            Self::Meshopt => {
                let mut out = vec![0u32; indices.len()];
                optimize_vertex_cache(&mut out, indices, vertex_count);
                out
            }
            Self::Fifo => {
                let mut out = vec![0u32; indices.len()];
                optimize_vertex_cache_fifo(&mut out, indices, vertex_count, FIFO_CACHE_SIZE);
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_documented_names_only() {
        assert_eq!(CacheOrder::parse("meshopt").unwrap(), CacheOrder::Meshopt);
        assert_eq!(CacheOrder::parse("fifo").unwrap(), CacheOrder::Fifo);
        assert!(CacheOrder::parse("forsyth").is_err());
        assert!(CacheOrder::parse("sloppy").is_err());
    }

    #[test]
    fn every_order_preserves_the_triangle_set() {
        let mut indices = Vec::new();
        for z in 0..7u32 {
            for x in 0..7u32 {
                let a = z * 8 + x;
                let b = a + 1;
                let c = a + 8;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        let vertex_count = 8 * 8;
        let mut expected: Vec<[u32; 3]> = indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let mut tri = *t;
                tri.sort_unstable();
                tri
            })
            .collect();
        expected.sort_unstable();
        for order in ALL {
            let mut actual: Vec<[u32; 3]> = order
                .reorder(&indices, vertex_count)
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| {
                    let mut tri = *t;
                    tri.sort_unstable();
                    tri
                })
                .collect();
            actual.sort_unstable();
            assert_eq!(actual, expected, "{order:?}");
        }
    }

    #[test]
    fn empty_indices_reorder_to_empty() {
        for order in ALL {
            assert!(order.reorder(&[], 0).is_empty(), "{order:?}");
        }
    }
}
