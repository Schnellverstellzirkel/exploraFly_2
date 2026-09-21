use world::{scatter_collision_at, scatter_slot, surface_height_at, TERRAIN_CELL_METRES};

#[test]
fn the_open_lake_has_no_invisible_trees() {
    // These slots and their jitter lie well inside the submerged lake basin.
    // The rendered biome gates reject them, regardless of slot hash.
    for cz in 194..206 {
        for cx in 16..26 {
            assert!(
                scatter_slot(cx, cz).is_none(),
                "underwater slot ({cx}, {cz}) must not produce a collision tree"
            );
        }
    }
}

#[test]
fn crown_clearance_covers_jittered_neighbour_slots() {
    let mut checked = 0;
    // The shared forest-cover gate intentionally leaves many meadow slots
    // empty. Sample a wider signed window so this remains a geometry/clearance
    // check rather than depending on a handful of old dense-biome slots.
    for cz in (-160..=160).step_by(20) {
        for cx in (-200..=200).step_by(20) {
            let Some(item) = scatter_slot(cx, cz) else {
                continue;
            };
            let reach = item.radius + 12.0 - 0.5;
            for (dx, dz) in [(reach, 0.0), (-reach, 0.0), (0.0, reach), (0.0, -reach)] {
                let x = (item.x + dx) as f64;
                let z = (item.z + dz) as f64;
                assert!(
                    scatter_collision_at(x, z) >= item.top - 0.001,
                    "slot ({cx}, {cz}) lost its crown clearance at ({x}, {z})"
                );
            }
            checked += 1;
        }
    }
    assert!(checked >= 20);
}

#[test]
fn scatter_roots_stay_on_the_visible_terrain_triangles() {
    let mut checked = 0;
    // Include both signs, both triangles of a cell, and terrain period seams.
    let mut samples: Vec<i64> = (-2800..=2800).step_by(140).collect();
    samples.extend_from_slice(&[-1000, -70, -1, 1, 44, 71, 900, 2700]);
    for &cz in &samples {
        for &cx in &samples {
            let Some(item) = scatter_slot(cx, cz) else {
                continue;
            };
            let cell = TERRAIN_CELL_METRES as f64;
            let x = item.x as f64 / cell;
            let z = item.z as f64 / cell;
            let (u, v) = ((x - x.floor()) as f32, (z - z.floor()) as f32);
            let corner = |dx: f64, dz: f64| {
                surface_height_at((x.floor() + dx) * cell, (z.floor() + dz) * cell)
            };
            // Barycentric height of ground.vert's actual indexed triangle.
            let visible = if u + v <= 1.0 {
                corner(0.0, 0.0) * (1.0 - u - v) + corner(1.0, 0.0) * u + corner(0.0, 1.0) * v
            } else {
                corner(1.0, 1.0) * (u + v - 1.0)
                    + corner(1.0, 0.0) * (1.0 - v)
                    + corner(0.0, 1.0) * (1.0 - u)
            };
            assert!(
                (item.ground - visible).abs() < 0.001,
                "slot ({cx}, {cz}): collision root {} vs visible mesh {visible}",
                item.ground
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 20,
        "must exercise surviving scatter across the world"
    );
}

#[test]
fn scatter_hash_wrap_matches_the_shader_on_both_sides_of_zero() {
    // ground.vert masks slot hash inputs to 16 bits. This slot translation
    // also spans 24 full terrain periods, so biome gates and species repeat.
    let mut checked = 0;
    for cz in (-320..=320).step_by(20) {
        for cx in (-320..=320).step_by(20) {
            let near = scatter_slot(cx, cz);
            let repeated = scatter_slot(cx + 65536, cz + 65536);
            assert_eq!(
                near.is_some(),
                repeated.is_some(),
                "slot ({cx}, {cz}) must keep the shader's periodic presence"
            );
            if let (Some(near), Some(repeated)) = (near, repeated) {
                assert_eq!(
                    near.radius, repeated.radius,
                    "slot ({cx}, {cz}) must keep the shader's species and size"
                );
                // Far f32 positions quantize to 0.125 m; allow the resulting
                // sub-metre slope difference in the interpolated base.
                assert!((near.ground - repeated.ground).abs() < 1.0);
                assert!(
                    ((near.top - near.ground) - (repeated.top - repeated.ground)).abs() < 0.001
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 10);
}
