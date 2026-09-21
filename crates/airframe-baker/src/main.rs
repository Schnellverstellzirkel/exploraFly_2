use std::path::PathBuf;

use airframe_baker::{analyze, bake_with_stats, round_trip, validate_target, BakeTarget, LOD_LEVELS};

fn usage() -> &'static str {
    "usage: airframe-baker [OUTPUT] [--analyze] [--dump-stats] [--validate] \
     [--target mesh-shader|legacy]"
}

struct Cli {
    output: Option<PathBuf>,
    analyze: bool,
    dump_stats: bool,
    validate: bool,
    target: Option<BakeTarget>,
}

fn parse_args() -> Result<Cli, String> {
    let mut cli = Cli {
        output: None,
        analyze: false,
        dump_stats: false,
        validate: false,
        target: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--analyze" => cli.analyze = true,
            "--dump-stats" => cli.dump_stats = true,
            "--validate" => cli.validate = true,
            "--target" => {
                let value = args.next().ok_or("--target needs mesh-shader or legacy")?;
                cli.target = Some(BakeTarget::parse(&value)?);
            }
            "--help" | "-h" => return Err(String::new()),
            other if other.starts_with('-') => {
                return Err(format!("unknown flag {other:?}\n{}", usage()));
            }
            other => {
                if cli.output.is_some() {
                    return Err(format!("unexpected argument {other:?}\n{}", usage()));
                }
                cli.output = Some(PathBuf::from(other));
            }
        }
    }
    Ok(cli)
}

fn dump_stats(stats: &airframe_baker::BakeStats) {
    println!(
        "triangles={} vertices={} opaque_indices={} glass_indices={} rt_indices={} rt_nodes={}",
        stats.triangles,
        stats.vertices,
        stats.opaque_indices,
        stats.glass_indices,
        stats.rt_indices,
        stats.rt_nodes
    );
    println!(
        "parts={} lods={} meshlets={} meshlet_vertices={}",
        stats.parts, stats.lods, stats.meshlets, stats.meshlet_vertices
    );
    for level in 0..LOD_LEVELS {
        println!(
            "lod{level}: triangles={} meshlets={}",
            stats.lod_triangles[level], stats.lod_meshlets[level]
        );
    }
}

fn print_analyze(asset: &airframe_format::BakedAirframe) {
    println!(
        "{:>5} {:>4} {:>4} {:>8}  lod triangles / meshlets / error_m",
        "part", "node", "mat", "radius"
    );
    for record in analyze(asset) {
        let mut levels = String::new();
        for level in 0..record.lod_triangles.len() {
            if level > 0 {
                levels.push_str(", ");
            }
            levels.push_str(&format!(
                "{}t/{}m/{:.4}m",
                record.lod_triangles[level], record.lod_meshlets[level], record.lod_errors[level]
            ));
        }
        println!(
            "{:>5} {:>4} {:>4} {:>8.3}  {}",
            record.part, record.node, record.material, record.bounds[3], levels
        );
    }
}

fn main() {
    let cli = match parse_args() {
        Ok(cli) => cli,
        Err(error) if error.is_empty() => {
            println!("{}", usage());
            return;
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    let (asset, stats) = bake_with_stats();
    let mut failed = false;

    if let Err(error) = round_trip(&asset) {
        eprintln!("round trip failed: {error}");
        failed = true;
    }
    if let Some(target) = cli.target {
        if let Err(error) = validate_target(&asset, target) {
            eprintln!("target {:?} validation failed: {error}", target);
            failed = true;
        }
    }
    if cli.validate && asset.validate().is_err() {
        eprintln!("validate failed");
        failed = true;
    }
    if cli.analyze {
        print_analyze(&asset);
    }
    if cli.dump_stats {
        dump_stats(&stats);
    }

    let validate_only = cli.validate && cli.output.is_none() && !cli.analyze && !cli.dump_stats;
    if !validate_only {
        let output = cli
            .output
            .unwrap_or_else(|| PathBuf::from("airframe.bin"));
        std::fs::write(&output, asset.encode()).expect("write baked airframe");
        if !cli.analyze && !cli.dump_stats {
            println!(
                "airframe baker: {} triangles, {} vertices, {} parts, {} meshlets, {:.1} KiB, {} RT nodes -> {}",
                stats.triangles,
                stats.vertices,
                stats.parts,
                stats.meshlets,
                asset.encode().len() as f32 / 1024.0,
                stats.rt_nodes,
                output.display(),
            );
        }
    } else {
        println!(
            "airframe baker: validate ok ({} triangles, {} meshlets)",
            stats.triangles, stats.meshlets
        );
    }

    if failed {
        std::process::exit(1);
    }
}
