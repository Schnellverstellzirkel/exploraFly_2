//! Emit a deterministic camera diagnostics CSV for the standard flight sequence.
//! Run with: cargo run -p sim --example camera_trace > camera-trace.csv

use glam::{Quat, Vec3};
use sim::camera::ChaseCamera;
use sim::flight::{Controls, Pose, SIM_STEP};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Cruise,
    HardBank,
    BarrelRoll,
    LoopReversal,
    StallHighAoa,
    RidgePass,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Cruise => "cruise",
            Self::HardBank => "hard_bank",
            Self::BarrelRoll => "barrel_roll",
            Self::LoopReversal => "loop_pitch_reversal",
            Self::StallHighAoa => "stall_high_aoa",
            Self::RidgePass => "low_altitude_ridge_pass",
        }
    }
}

const PHASES: [(Phase, u32); 6] = [
    (Phase::Cruise, 216),
    (Phase::HardBank, 288),
    (Phase::BarrelRoll, 360),
    (Phase::LoopReversal, 432),
    (Phase::StallHighAoa, 288),
    (Phase::RidgePass, 360),
];

fn phase_at(frame: u32) -> (Phase, u32) {
    let mut start = 0;
    for (phase, frames) in PHASES {
        if frame < start + frames {
            return (phase, frame - start);
        }
        start += frames;
    }
    (Phase::RidgePass, 0)
}

fn controls_for(phase: Phase, local_time: f32) -> Controls {
    match phase {
        Phase::Cruise | Phase::RidgePass => Controls::neutral(),
        Phase::HardBank => Controls {
            bank: 1.0,
            ..Controls::neutral()
        },
        Phase::BarrelRoll => Controls {
            pitch: 0.18 * (local_time * 2.0).sin(),
            bank: 1.0,
            ..Controls::neutral()
        },
        Phase::LoopReversal => Controls {
            pitch: if local_time < 1.5 { 1.0 } else { -1.0 },
            yaw: 0.15,
            ..Controls::neutral()
        },
        Phase::StallHighAoa => Controls {
            pitch: if local_time < 1.0 { 1.0 } else { -1.0 },
            yaw: 0.2,
            ..Controls::neutral()
        },
    }
}

fn main() {
    println!("time_s,phase,speed_mps,altitude_m,aoa_rad,sideslip_rad,airflow_blend,pitch_error_rad,yaw_error_rad,roll_error_rad,offset_right_m,offset_up_m,offset_forward_m,boom_fraction,boom_back_m,slide_x_m,slide_y_m,slide_z_m,framing_compromised,collision_unresolved,collision_budget_exhausted,airframe_visibility,height_queries,fov_y_deg");

    let total_frames = PHASES.iter().map(|(_, frames)| frames).sum();
    let mut pose = Pose::start();
    let mut camera = ChaseCamera::new();
    let mut previous_phase = None;

    for frame_index in 0..total_frames {
        let (phase, local_frame) = phase_at(frame_index);
        let local_time = local_frame as f32 * SIM_STEP;
        if previous_phase != Some(phase) {
            match phase {
                Phase::StallHighAoa => {
                    pose.speed = 70.0;
                    pose.velocity =
                        pose.orientation * Vec3::Z * 48.0 - pose.orientation * Vec3::Y * 38.0;
                }
                Phase::RidgePass => {
                    pose.x = 0.0;
                    pose.y = 70.0;
                    pose.z = 0.0;
                    pose.orientation = Quat::IDENTITY;
                    pose.speed = 90.0;
                    pose.velocity = Vec3::Z * pose.speed;
                }
                _ => {}
            }
            previous_phase = Some(phase);
        }

        let controls = controls_for(phase, local_time);
        let previous_air_velocity = pose.velocity;
        if matches!(phase, Phase::HardBank | Phase::BarrelRoll) {
            let forward = pose.orientation * Vec3::Z;
            let right = pose.orientation * Vec3::X;
            let side_speed = 24.0 * (local_time * 1.3).sin();
            let target_velocity = forward * pose.speed + right * side_speed;
            pose.velocity = pose.velocity.lerp(target_velocity, 0.04);
        }
        pose.step(&controls, SIM_STEP);
        let air_acceleration =
            ((pose.velocity - previous_air_velocity) / SIM_STEP).clamp_length_max(120.0);

        if phase == Phase::RidgePass {
            pose.y = 70.0;
        }
        let anchor_x = pose.x as f64;
        let anchor_y = pose.y;
        let anchor_z = pose.z as f64;
        let ridge_pass = phase == Phase::RidgePass;
        let collision_height = |x: f64, z: f64| {
            let on_ridge = ridge_pass
                && (x - anchor_x).abs() < 8.0
                && (anchor_z - 11.0..=anchor_z - 5.0).contains(&z);
            if on_ridge {
                anchor_y + 8.0
            } else {
                anchor_y - 50.0
            }
        };
        let frame = camera.step_with_wind_collision_and_acceleration(
            &pose,
            &controls,
            SIM_STEP,
            16.0 / 9.0,
            Vec3::new(pose.x, pose.y, pose.z),
            Vec3::ZERO,
            air_acceleration,
            collision_height,
        );

        if frame_index % 14 == 0 || local_frame == 0 {
            println!(
                "{:.5},{},{:.3},{:.3},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.3},{:.3},{:.3},{:.3},{},{},{},{:.3},{},{:.3}",
                frame_index as f32 * SIM_STEP,
                phase.label(),
                pose.speed,
                pose.y,
                pose.aoa,
                pose.sideslip,
                frame.airflow_blend,
                frame.angular_error.x,
                frame.angular_error.y,
                frame.angular_error.z,
                frame.position_offset.x,
                frame.position_offset.y,
                frame.position_offset.z,
                frame.boom_fraction,
                frame.boom_back_m,
                frame.obstacle_slide.x,
                frame.obstacle_slide.y,
                frame.obstacle_slide.z,
                frame.framing_compromised,
                frame.collision_unresolved,
                frame.collision_budget_exhausted,
                frame.airframe_visibility,
                frame.collision_queries,
                frame.fov_y.to_degrees(),
            );
        }
    }
}
