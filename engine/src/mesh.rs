pub struct MeshVertex {
    pub pos: [f32; 3],
    pub color: [f32; 3],
}

// Small glider silhouette. Fuselage dart plus two wing quads.
// Faces wound for counter-clockwise front in a Y-up world.
pub const GLIDER: [MeshVertex; 18] = [
    // Nose dart, top.
    MeshVertex { pos: [0.0, 0.6, 6.0], color: [0.92, 0.93, 0.95] },
    MeshVertex { pos: [-0.7, 0.0, -2.0], color: [0.75, 0.78, 0.82] },
    MeshVertex { pos: [0.7, 0.0, -2.0], color: [0.75, 0.78, 0.82] },
    // Nose dart, bottom.
    MeshVertex { pos: [0.0, -0.4, 5.0], color: [0.55, 0.6, 0.65] },
    MeshVertex { pos: [0.7, 0.0, -2.0], color: [0.5, 0.55, 0.6] },
    MeshVertex { pos: [-0.7, 0.0, -2.0], color: [0.5, 0.55, 0.6] },
    // Left wing, top.
    MeshVertex { pos: [-0.5, 0.1, 1.5], color: [0.85, 0.88, 0.9] },
    MeshVertex { pos: [-11.0, 0.1, -1.0], color: [0.7, 0.74, 0.78] },
    MeshVertex { pos: [-0.5, 0.1, -2.0], color: [0.8, 0.83, 0.86] },
    // Left wing, bottom.
    MeshVertex { pos: [-0.5, -0.05, -2.0], color: [0.5, 0.54, 0.58] },
    MeshVertex { pos: [-11.0, -0.05, -1.0], color: [0.45, 0.49, 0.53] },
    MeshVertex { pos: [-0.5, -0.05, 1.5], color: [0.52, 0.56, 0.6] },
    // Right wing, top.
    MeshVertex { pos: [0.5, 0.1, -2.0], color: [0.8, 0.83, 0.86] },
    MeshVertex { pos: [11.0, 0.1, -1.0], color: [0.7, 0.74, 0.78] },
    MeshVertex { pos: [0.5, 0.1, 1.5], color: [0.85, 0.88, 0.9] },
    // Right wing, bottom.
    MeshVertex { pos: [0.5, -0.05, 1.5], color: [0.52, 0.56, 0.6] },
    MeshVertex { pos: [11.0, -0.05, -1.0], color: [0.45, 0.49, 0.53] },
    MeshVertex { pos: [0.5, -0.05, -2.0], color: [0.5, 0.54, 0.58] },
];
