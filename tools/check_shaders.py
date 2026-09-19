"""Validate every offline shader produced by a debug Cargo engine build."""
from pathlib import Path
import subprocess
import sys


def main() -> int:
    shaders = sorted(Path("target/debug/build").glob("explora-engine-*/out/*.spv"))
    if not shaders:
        print("No compiled engine shaders found; run cargo test --workspace first.", file=sys.stderr)
        return 1
    for shader in shaders:
        subprocess.run(["spirv-val", "--target-env", "vulkan1.3", str(shader)], check=True)
    print(f"Validated {len(shaders)} SPIR-V modules for Vulkan 1.3.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
