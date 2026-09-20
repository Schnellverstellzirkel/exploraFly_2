use glam::Vec3;
use sim::flight::{Controls, Pose, SIM_STEP};
use sim::wind::Wind;

#[test]
fn turns_over_the_spawn_valley_do_not_hit_invisible_obstacles() {
    // Repeated short left/right inputs reproduce the reported midair jumps.
    // Exercise the real flight and world queries together: the sim-only
    // turning tests cannot detect a bad terrain/tree clearance floor.
    for bank in [-1.0, 1.0] {
        let mut pose = Pose::start();
        pose.x = world::SPAWN_X;
        pose.y = world::SPAWN_ALTITUDE;
        pose.z = world::SPAWN_Z;
        let wind = Wind::new(1.0);
        pose.velocity += wind.velocity(Vec3::new(pose.x, pose.y, pose.z), 0.0);

        for step in 0..144 * 20 {
            let controls = Controls {
                bank: if step % 576 < 100 { bank } else { 0.0 },
                ..Controls::neutral()
            };
            let air = wind.velocity(Vec3::new(pose.x, pose.y, pose.z), step as f32 * SIM_STEP);
            pose.step_with_wind(&controls, SIM_STEP, air);
            let ground = world::surface_height_at(pose.x as f64, pose.z as f64);
            assert!(
                pose.y > ground + 250.0,
                "test route must stay well above the canopy"
            );

            let floor =
                world::collision_height_at(pose.x as f64, pose.z as f64) + world::CLEARANCE_METRES;
            assert!(
                pose.y >= floor,
                "bank {bank}, step {step}: invisible floor {floor} would lift the plane \
                 from {} at ({}, {}), terrain {ground}",
                pose.y,
                pose.x,
                pose.z
            );
        }
    }
}
