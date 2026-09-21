use std::path::PathBuf;

fn main() {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("airframe.bin"));
    let (asset, stats) = airframe_baker::bake_with_stats();
    std::fs::write(&output, asset.encode()).expect("write baked airframe");
    println!(
        "airframe baker: {} triangles, {} vertices, {:.1} KiB stream, {} RT nodes -> {}",
        stats.triangles,
        stats.vertices,
        stats.vertices as f32 * airframe_format::VERTEX_BYTES as f32 / 1024.0,
        stats.rt_nodes,
        output.display(),
    );
}
