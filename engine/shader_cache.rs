//! Cache the effective compiler input, including injected shader headers.
use std::path::Path;

pub fn is_current(output: &Path, input: &str) -> bool {
    output.is_file()
        && std::fs::read_to_string(output.with_extension("source"))
            .is_ok_and(|previous| previous == input)
}

pub fn record(output: &Path, input: &str) -> std::io::Result<()> {
    std::fs::write(output.with_extension("source"), input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_reuses_existing_output_with_identical_effective_source() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("explora-shader-{}-{unique}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let output = dir.join("ground.frag.spv");
        let source = "compiler recipe\n#define SHADOW_RAYS 2\nvoid main() {}";
        assert!(!is_current(&output, source));
        std::fs::write(&output, [3, 2, 35, 7]).unwrap();
        assert!(!is_current(&output, source), "legacy timestamp-only artifacts must rebuild");
        record(&output, source).unwrap();
        assert!(is_current(&output, source));
        assert!(!is_current(&output, &source.replace("RAYS 2", "RAYS 8")));
        assert!(!is_current(&output, &source.replace("compiler recipe", "new compiler options")));
        std::fs::remove_file(&output).unwrap();
        assert!(!is_current(&output, source));
        std::fs::remove_file(output.with_extension("source")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
