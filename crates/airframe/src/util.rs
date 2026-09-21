// Procedural glider airframe. Ported from the web prototype.
// Same planform, same parts, new storage: indexed triangles,
// one interleaved stream, baked flex weights, merged batches.
// Forward is +z here, so source z is negated and triangles flip.

/// Material category ID mapped to PBR albedo and emissive properties in the shader.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum MatId {
    /// Sail cloth weave with anisotropic texture filtering.
    Sail = 0,
    /// Teal enamel over the composite fuselage shell.
    Composite = 1,
    /// Dark structural carbon/graphite panels.
    Graphite = 2,
    /// Brass structural fittings or steel rotor/nozzle parts, selected by node.
    Titanium = 3,
    /// Matte black engine shroud, struts, and cockpit tub.
    Dark = 4,
    /// Pilot seat cushion leather.
    Seat = 5,
    /// Transparent canopy glass (drawn in alpha pass).
    Glass = 6,
    /// Glowing engine exhaust and navigation lights.
    Glow = 7,
}

impl MatId {
    /// Stable wire value written into the packed vertex stream and `PartDesc`.
    pub const fn packed_id(self) -> u8 {
        self as u8
    }
}

/// Kinematic node hierarchy governing dynamic transformations (rigid animation and aeroelasticity).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Node {
    /// Main fuselage static frame.
    Hull,
    /// Cockpit canopy assembly.
    Canopy,
    /// Port (left) wing root.
    WingL,
    /// Starboard (right) wing root.
    WingR,
    /// Trailing-edge control flap (0..5).
    Flap(u8),
    /// Spinning jet turbine rotor hub.
    Rotor,
    /// Articulating thrust vectoring nozzle petal (0..9).
    Petal(u8),
    /// V-tail canted stabilizer fin (0..1).
    Fin(u8),
}

impl Node {
    /// Slot index into the shared 23-entry node UBO and RT node ranges.
    ///
    /// Single source of truth for the runtime node table. Indexed variants
    /// panic on out-of-range ids: `Flap` must be `0..6`, `Petal` `0..10`,
    /// `Fin` `0..2`.
    pub const fn packed_id(self) -> u8 {
        match self {
            Self::Hull => 0,
            Self::Canopy => 1,
            Self::WingL => 2,
            Self::WingR => 3,
            Self::Flap(id) => {
                assert!((id as u16) < 6, "Node::Flap index must be 0..6");
                4 + id
            }
            Self::Rotor => 10,
            Self::Petal(id) => {
                assert!((id as u16) < 10, "Node::Petal index must be 0..10");
                11 + id
            }
            Self::Fin(id) => {
                assert!((id as u16) < 2, "Node::Fin index must be 0..2");
                21 + id
            }
        }
    }
}

/// How soon a semantic part may leave the visual LOD chain.
///
/// The baker packs this into `PartDesc::flags` bits 8..12 and uses it to pick
/// per-level error budgets, RT proxy density, and task-shader screen-size
/// culling. Values match `airframe_format::IMPORTANCE_*`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Importance {
    /// Planform outline that must survive longest: hull shell, sail, fins.
    Silhouette = 0,
    /// Load-bearing surfaces that keep shape while still resolvable:
    /// flaps, booms, binding, nozzle petals, canopy frame.
    Structural = 1,
    /// Small hardware that reads up close: bolts, struts, rotor blades,
    /// flap edge hardware.
    Detail = 2,
    /// Geometry hidden behind or inside other surfaces: battens, stringers,
    /// exhaust liner, cockpit tub, seat.
    Interior = 3,
    /// Emissive blobs that collapse to points first: nav glow, HUD, rotor glow.
    Emitter = 4,
}

impl Importance {
    /// Stable wire value written into packed part flags.
    pub const fn packed_id(self) -> u8 {
        self as u8
    }

    /// Decode a packed importance nibble. Returns `None` outside `0..5`.
    pub const fn from_packed(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Silhouette),
            1 => Some(Self::Structural),
            2 => Some(Self::Detail),
            3 => Some(Self::Interior),
            4 => Some(Self::Emitter),
            _ => None,
        }
    }

    /// Central per-class rendering policy for the whole offline pipeline:
    /// source tessellation, LOD error growth, RT proxy density, task-shader
    /// drop threshold, impostor crossover, and animation retention.
    pub const fn policy(self) -> LodPolicy {
        match self {
            Self::Silhouette => LodPolicy {
                tess_abs_error: 0.001,
                lod_error_scale: 0.5,
                rt_density: 0.10,
                drop_px: 0.0,
                impostor_px: 0.000_2,
                preserve_animation: true,
            },
            Self::Structural => LodPolicy {
                tess_abs_error: 0.005,
                lod_error_scale: 1.0,
                rt_density: 0.08,
                drop_px: 0.0,
                impostor_px: 0.000_5,
                preserve_animation: true,
            },
            Self::Detail => LodPolicy {
                tess_abs_error: 0.012,
                lod_error_scale: 2.0,
                rt_density: 0.05,
                drop_px: 0.002,
                impostor_px: 0.0,
                preserve_animation: false,
            },
            Self::Interior => LodPolicy {
                tess_abs_error: 0.025,
                lod_error_scale: 4.0,
                rt_density: 0.04,
                drop_px: 0.004,
                impostor_px: 0.0,
                preserve_animation: false,
            },
            Self::Emitter => LodPolicy {
                tess_abs_error: 0.020,
                lod_error_scale: 2.5,
                rt_density: 0.0,
                drop_px: 0.001,
                impostor_px: 0.0,
                preserve_animation: false,
            },
        }
    }
}

/// Compile-time rendering policy for one [`Importance`] class.
///
/// Offline stages read these fields instead of keeping separate per-stage
/// tables that can drift apart. `drop_px` and `impostor_px` are projected
/// bounding-sphere radii in NDC half-height units, the same units
/// `plane.task` uses for its screen-size cull.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LodPolicy {
    /// Absolute object-space chordal error budget in meters for offline
    /// primitive tessellation. Larger coarser geometry.
    pub tess_abs_error: f32,
    /// Multiplier on the baker's structural base LOD error-budget row.
    /// Larger coarser far levels.
    pub lod_error_scale: f32,
    /// Fraction of the part's full-resolution triangles kept in the RT
    /// shadow proxy. `0.0` contributes no RT geometry.
    pub rt_density: f32,
    /// Screen radius below which the task shader drops the part entirely.
    /// `0.0` never drops.
    pub drop_px: f32,
    /// Screen radius below which an impostor may replace the mesh when
    /// [`PartFlags::ALLOW_IMPOSTOR`] is set. `0.0` never switches.
    pub impostor_px: f32,
    /// Keep the part on the animated node chain farther into the distance.
    pub preserve_animation: bool,
}

/// Rendering-behaviour bits carried on each [`RawPart`].
///
/// Material category is not behaviour: glass alpha, RT participation,
/// impostor eligibility, and silhouette priority are explicit flags the
/// baker reads instead of inferring from `MatId`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(transparent)]
pub struct PartFlags(u16);

impl PartFlags {
    /// No special behaviour.
    pub const NONE: Self = Self(0);
    /// Part contributes geometry to the ray-traced shadow proxy.
    pub const CAST_RT: Self = Self(1 << 0);
    /// Part may switch to a far-field impostor when small enough on screen.
    pub const ALLOW_IMPOSTOR: Self = Self(1 << 1);
    /// Part stays on the animated node chain farther into the distance.
    pub const PRESERVE_ANIMATION: Self = Self(1 << 2);
    /// Surface is drawn double-sided (thin cloth or shell).
    pub const TWO_SIDED: Self = Self(1 << 3);
    /// Surface uses the alpha pass and the legacy glass draw path.
    pub const ALPHA: Self = Self(1 << 4);
    /// Planform outline that must survive the longest in the LOD chain.
    pub const SILHOUETTE_CRITICAL: Self = Self(1 << 5);

    /// True when every bit in `other` is set in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Bitwise union.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Bitwise difference.
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Raw bit payload for tests and diagnostics.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Default behaviour for a procedural part before any per-site override.
    pub const fn for_part(mat: MatId, importance: Importance) -> Self {
        let mut bits = 0u16;
        let is_glass = matches!(mat, MatId::Glass);
        let is_emitter = matches!(importance, Importance::Emitter);
        if !is_glass && !is_emitter {
            bits |= Self::CAST_RT.0;
        }
        if is_glass {
            bits |= Self::ALPHA.0 | Self::TWO_SIDED.0;
        }
        if matches!(mat, MatId::Sail) {
            bits |= Self::TWO_SIDED.0;
        }
        match importance {
            Importance::Silhouette => {
                bits |= Self::ALLOW_IMPOSTOR.0
                    | Self::PRESERVE_ANIMATION.0
                    | Self::SILHOUETTE_CRITICAL.0;
            }
            Importance::Structural => {
                bits |= Self::ALLOW_IMPOSTOR.0 | Self::PRESERVE_ANIMATION.0;
            }
            Importance::Detail | Importance::Interior | Importance::Emitter => {}
        }
        Self(bits)
    }
}

impl std::ops::BitOr for PartFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for PartFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

impl std::ops::BitAnd for PartFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::Not for PartFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

/// Unindexed raw vertex generated by procedural CAD modeling functions.
///
/// High-quality unpacked floats on purpose: this exists only during baking.
/// Quantization and meshlet-local packing happen in `airframe-baker` and
/// `airframe-format`, at the GPU boundary.
pub struct RawVert {
    /// Position coordinates [x, y, z] in meters.
    pub pos: [f32; 3],
    /// Texture mapping coordinates [u, v].
    pub uv: [f32; 2],
    /// Aeroelastic flex influence weight along the wing span.
    pub flex: f32,
}

/// Discrete procedural airframe component before batching into merged GPU buffers.
pub struct RawPart {
    /// Kinematic node parent for animated motion.
    pub node: Node,
    /// Surface material identifier.
    pub mat: MatId,
    /// Semantic LOD/visibility class for the baker.
    pub importance: Importance,
    /// Explicit rendering behaviour: RT, impostor, alpha, silhouette.
    pub flags: PartFlags,
    /// Vertex buffer payload.
    pub verts: Vec<RawVert>,
    /// Triangle index buffer.
    pub idx: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_ids_cover_the_runtime_tables() {
        assert_eq!(MatId::Sail.packed_id(), 0);
        assert_eq!(MatId::Glass.packed_id(), 6);
        assert_eq!(MatId::Glow.packed_id(), 7);
        assert_eq!(Node::Hull.packed_id(), 0);
        assert_eq!(Node::Canopy.packed_id(), 1);
        assert_eq!(Node::WingL.packed_id(), 2);
        assert_eq!(Node::WingR.packed_id(), 3);
        for id in 0..6u8 {
            assert_eq!(Node::Flap(id).packed_id(), 4 + id);
        }
        assert_eq!(Node::Rotor.packed_id(), 10);
        for id in 0..10u8 {
            assert_eq!(Node::Petal(id).packed_id(), 11 + id);
        }
        assert_eq!(Node::Fin(0).packed_id(), 21);
        assert_eq!(Node::Fin(1).packed_id(), 22);
        assert_eq!(Importance::Silhouette.packed_id(), 0);
        assert_eq!(Importance::Emitter.packed_id(), 4);
        for id in 0..=4u8 {
            assert!(Importance::from_packed(id).is_some());
        }
        assert!(Importance::from_packed(5).is_none());
    }

    #[test]
    #[should_panic(expected = "Node::Flap")]
    fn flap_index_above_five_panics() {
        let _ = Node::Flap(6).packed_id();
    }

    #[test]
    #[should_panic(expected = "Node::Petal")]
    fn petal_index_above_nine_panics() {
        let _ = Node::Petal(10).packed_id();
    }

    #[test]
    #[should_panic(expected = "Node::Fin")]
    fn fin_index_above_one_panics() {
        let _ = Node::Fin(2).packed_id();
    }

    #[test]
    fn policies_match_the_documented_pipeline_thresholds() {
        let silhouette = Importance::Silhouette.policy();
        assert_eq!(silhouette.tess_abs_error, 0.001);
        assert_eq!(silhouette.lod_error_scale, 0.5);
        assert_eq!(silhouette.rt_density, 0.10);
        assert_eq!(silhouette.drop_px, 0.0);
        assert!(silhouette.preserve_animation);

        let structural = Importance::Structural.policy();
        assert_eq!(structural.lod_error_scale, 1.0);
        assert_eq!(structural.rt_density, 0.08);
        assert_eq!(structural.drop_px, 0.0);

        // plane.task MIN_SCREEN_RADIUS_* thresholds.
        assert_eq!(Importance::Detail.policy().drop_px, 0.002);
        assert_eq!(Importance::Interior.policy().drop_px, 0.004);
        assert_eq!(Importance::Emitter.policy().drop_px, 0.001);

        // Emissive blobs cast no shadow and never take an impostor path.
        let emitter = Importance::Emitter.policy();
        assert_eq!(emitter.rt_density, 0.0);
        assert_eq!(emitter.impostor_px, 0.0);
        assert!(!emitter.preserve_animation);

        // Only silhouette and structural parts may cross over to impostors.
        assert!(silhouette.impostor_px > 0.0);
        assert!(structural.impostor_px > 0.0);
        assert_eq!(Importance::Detail.policy().impostor_px, 0.0);
        assert_eq!(Importance::Interior.policy().impostor_px, 0.0);
    }

    #[test]
    fn default_flags_separate_material_from_behaviour() {
        let glass = PartFlags::for_part(MatId::Glass, Importance::Silhouette);
        assert!(glass.contains(PartFlags::ALPHA));
        assert!(!glass.contains(PartFlags::CAST_RT));
        assert!(glass.contains(PartFlags::ALLOW_IMPOSTOR));
        assert!(glass.contains(PartFlags::PRESERVE_ANIMATION));

        let sail = PartFlags::for_part(MatId::Sail, Importance::Silhouette);
        assert!(sail.contains(PartFlags::TWO_SIDED));
        assert!(sail.contains(PartFlags::CAST_RT));
        assert!(sail.contains(PartFlags::SILHOUETTE_CRITICAL));

        let hull = PartFlags::for_part(MatId::Composite, Importance::Silhouette);
        assert!(hull.contains(PartFlags::CAST_RT));
        assert!(!hull.contains(PartFlags::TWO_SIDED));

        let glow = PartFlags::for_part(MatId::Glow, Importance::Emitter);
        assert!(!glow.contains(PartFlags::CAST_RT));
        assert!(!glow.contains(PartFlags::ALLOW_IMPOSTOR));

        let batten = PartFlags::for_part(MatId::Graphite, Importance::Interior);
        assert!(batten.contains(PartFlags::CAST_RT));
        assert!(!batten.contains(PartFlags::ALLOW_IMPOSTOR));
        assert!(!batten.contains(PartFlags::PRESERVE_ANIMATION));
        assert!(!batten.contains(PartFlags::SILHOUETTE_CRITICAL));
    }

    #[test]
    fn flag_set_algebra_behaves() {
        let flags = PartFlags::CAST_RT | PartFlags::ALPHA;
        assert!(flags.contains(PartFlags::CAST_RT));
        assert!(flags.contains(PartFlags::ALPHA));
        assert!(!flags.contains(PartFlags::ALLOW_IMPOSTOR));
        assert_eq!(flags.bits(), (1 << 0) | (1 << 4));
        let without = flags.difference(PartFlags::CAST_RT);
        assert!(!without.contains(PartFlags::CAST_RT));
        assert!(without.contains(PartFlags::ALPHA));
        assert!(!PartFlags::NONE.contains(PartFlags::CAST_RT));
    }
}
