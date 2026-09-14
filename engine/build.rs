// Offline GLSL to SPIR-V compile with shaderc (glslang). Runs at cargo
// build time, not in the game loop. Runtime only does include_bytes! of the
// emitted modules, so naga/WGSL stay out of the binary entirely.
// Regenerates when any shader source changes via rerun-if-changed.

use std::path::PathBuf;

fn env_header(samples: u32) -> String {
    let mut points = format!(
        "#define ENV_SAMPLES {}u\nconst vec3 ENV_POINTS[{}] = vec3[{}](\n",
        samples, samples, samples
    );
    for i in 0..samples {
        let azimuth = (i.reverse_bits() as f64 / 4294967296.0) * std::f64::consts::TAU;
        let sep = if i + 1 == samples { "" } else { "," };
        points.push_str(&format!(
            "    vec3({:.9}, {:.9}, {:.9}){}\n",
            azimuth.cos(),
            azimuth.sin(),
            (i as f64 + 0.5) / samples as f64,
            sep
        ));
    }
    points.push_str(");\n");
    points
}

fn compile(
    compiler: &shaderc::Compiler,
    options: &shaderc::CompileOptions,
    src: &str,
    name: &str,
    kind: shaderc::ShaderKind,
    src_path: Option<&std::path::Path>,
    out: &PathBuf,
) {
    if let Some(sp) = src_path {
        if out.exists() {
            if let (Ok(sm), Ok(om)) = (std::fs::metadata(sp), std::fs::metadata(out)) {
                if let (Ok(st), Ok(ot)) = (sm.modified(), om.modified()) {
                    if ot >= st {
                        return;
                    }
                }
            }
        }
    }
    let binary = compiler
        .compile_into_spirv(src, kind, name, "main", Some(options))
        .unwrap_or_else(|e| panic!("shaderc failed for {name}: {e}"));
    assert!((binary.len() & 3) == 0, "unaligned SPIR-V for {name}");
    std::fs::write(out, binary.as_binary_u8())
        .unwrap_or_else(|_| panic!("write failed for {}", out.display()));
    println!("shaderc: {name} -> {} ({} bytes)", out.display(), binary.len());
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let shader_dir = manifest.join("shaders");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    let sources = [
        "shaders/plane.vert",
        "shaders/plane.frag",
        "shaders/sky.vert",
        "shaders/sky.frag",
        "shaders/depth.frag",
        "shaders/plume.vert",
        "shaders/plume.frag",
        "shaders/trail.vert",
        "shaders/trail.frag",
        "shaders/composite.vert",
        "shaders/composite.frag",
    ];
    for s in &sources {
        println!("cargo:rerun-if-changed={s}");
    }

    #[derive(Clone)]
    struct Job {
        name: String,
        src_file: String,
        header: String,
        kind: shaderc::ShaderKind,
    }

    let mut jobs = Vec::new();
    jobs.push(Job {
        name: "plane.vert".into(),
        src_file: "plane.vert".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Vertex,
    });
    for samples in [8u32, 16, 32, 128] {
        jobs.push(Job {
            name: format!("plane-{samples}.frag"),
            src_file: "plane.frag".into(),
            header: env_header(samples),
            kind: shaderc::ShaderKind::Fragment,
        });
    }
    jobs.push(Job {
        name: "sky.vert".into(),
        src_file: "sky.vert".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Vertex,
    });
    jobs.push(Job {
        name: "sky.frag".into(),
        src_file: "sky.frag".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Fragment,
    });
    jobs.push(Job {
        name: "depth.frag".into(),
        src_file: "depth.frag".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Fragment,
    });
    jobs.push(Job {
        name: "plume.vert".into(),
        src_file: "plume.vert".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Vertex,
    });
    jobs.push(Job {
        name: "plume.frag".into(),
        src_file: "plume.frag".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Fragment,
    });
    jobs.push(Job {
        name: "trail.vert".into(),
        src_file: "trail.vert".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Vertex,
    });
    jobs.push(Job {
        name: "trail.frag".into(),
        src_file: "trail.frag".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Fragment,
    });
    jobs.push(Job {
        name: "composite.vert".into(),
        src_file: "composite.vert".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Vertex,
    });
    jobs.push(Job {
        name: "composite.frag".into(),
        src_file: "composite.frag".into(),
        header: String::new(),
        kind: shaderc::ShaderKind::Fragment,
    });

    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(jobs.len());

    let chunks: Vec<_> = jobs
        .chunks((jobs.len() + num_threads - 1) / num_threads)
        .map(|c| c.to_vec())
        .collect();

    std::thread::scope(|s| {
        for chunk in chunks {
            let shader_dir = &shader_dir;
            let out_dir = &out_dir;
            s.spawn(move || {
                let compiler = shaderc::Compiler::new().expect("shaderc compiler");
                let mut options = shaderc::CompileOptions::new().expect("shaderc options");
                options.set_target_env(
                    shaderc::TargetEnv::Vulkan,
                    shaderc::EnvVersion::Vulkan1_3 as u32,
                );
                options.set_optimization_level(shaderc::OptimizationLevel::Performance);

                for job in chunk {
                    let path = shader_dir.join(&job.src_file);
                    let text = std::fs::read_to_string(&path)
                        .unwrap_or_else(|_| panic!("missing shader {}", job.src_file));
                    let src = if job.header.is_empty() {
                        text
                    } else if let Some(pos) = text.find('\n') {
                        format!("{}\n{}{}", &text[..pos], job.header, &text[pos..])
                    } else {
                        format!("{}{}", job.header, text)
                    };
                    let out = out_dir.join(format!("{}.spv", job.name));
                    compile(
                        &compiler,
                        &options,
                        &src,
                        &job.name,
                        job.kind,
                        Some(&path),
                        &out,
                    );
                }
            });
        }
    });
}
