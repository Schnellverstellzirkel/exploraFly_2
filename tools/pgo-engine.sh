#!/usr/bin/env bash
# Train and apply LLVM PGO for the native engine on the target workstation.
#
# This is intentionally opt-in. PGO data is workload-specific and must not be
# hidden in .cargo/config.toml or treated as a portable build input.
set -Eeuo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
pgo_root=${EXPLORA_PGO_ROOT:-"$repo_root/target/pgo"}
if [[ "$pgo_root" != /* ]]; then
    pgo_root="$repo_root/$pgo_root"
fi
instrumented_target="$pgo_root/instrumented"
use_target="$pgo_root/use"
profile_data="$pgo_root/data"
merged_profile="$pgo_root/merged.profdata"
presents=${EXPLORA_PGO_PRESENTS:-3000}
host_target=x86_64-unknown-linux-gnu

usage() {
    cat <<'EOF'
Usage: tools/pgo-engine.sh

Builds an instrumented dist binary, trains it on three representative scene
workloads, merges the generated .profraw files, and builds a separate PGO-use
dist binary. Set EXPLORA_PGO_PRESENTS to change the presents per workload and
EXPLORA_PGO_ROOT to change the output directory.

The final binary is printed at:
  target/pgo/use/x86_64-unknown-linux-gnu/dist/explora
EOF
}

if (($# == 1)) && [[ "$1" == "-h" || "$1" == "--help" ]]; then
    usage
    exit 0
fi
if (($# != 0)); then
    usage
    exit 2
fi

if ! [[ "$presents" =~ ^[1-9][0-9]*$ ]]; then
    echo "EXPLORA_PGO_PRESENTS must be a positive integer" >&2
    exit 2
fi

rustc_llvm_major=$(rustc -vV | sed -n 's/^LLVM version: \([0-9][0-9]*\)\..*/\1/p')
if [[ -z "$rustc_llvm_major" ]]; then
    echo "could not determine rustc's LLVM major version" >&2
    exit 1
fi

find_profdata() {
    if [[ -n "${LLVM_PROFDATA:-}" ]]; then
        printf '%s\n' "$LLVM_PROFDATA"
        return
    fi
    local rustc_tools
    rustc_tools="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-profdata"
    if [[ -x "$rustc_tools" ]]; then
        printf '%s\n' "$rustc_tools"
        return
    fi
    local candidate
    for candidate in \
        "llvm-profdata" \
        "llvm-profdata-$rustc_llvm_major" \
        "/usr/lib/llvm-$rustc_llvm_major/bin/llvm-profdata"; do
        if command -v "$candidate" >/dev/null 2>&1; then
            command -v "$candidate"
            return
        elif [[ -x "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return
        fi
    done
}

llvm_profdata=$(find_profdata || true)
if [[ -z "$llvm_profdata" ]]; then
    cat >&2 <<EOF
No llvm-profdata matching rustc LLVM $rustc_llvm_major was found.
Install the LLVM tools for the active Rust toolchain (for example,
\`rustup component add llvm-tools-preview\`) or set LLVM_PROFDATA to a
matching binary.
EOF
    exit 1
fi

profdata_version=$(
    "$llvm_profdata" --version 2>/dev/null \
        | sed -n 's/.*LLVM version \([0-9][0-9]*\)\..*/\1/p'
)
if [[ -n "$profdata_version" && "$profdata_version" != "$rustc_llvm_major" ]]; then
    echo "llvm-profdata $llvm_profdata is LLVM $profdata_version, but rustc is LLVM $rustc_llvm_major" >&2
    exit 1
fi

mkdir -p "$profile_data"
find "$profile_data" -type f -name '*.profraw' -delete
rm -f "$merged_profile"

build_instrumented() {
    echo "[pgo] building instrumented dist binary"
    local extra_flags="-Cprofile-generate=$profile_data"
    CARGO_TARGET_DIR="$instrumented_target" \
        RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$extra_flags" \
        cargo build --profile dist -p explora-engine --target "$host_target"
}

run_workload() {
    local name=$1
    shift
    echo "[pgo] training workload: $name ($presents presents)"
    # LLVM's default `default_%m.profraw` name is stable for this binary, so
    # separate processes would overwrite one another when several workloads
    # are trained sequentially. `%p` keeps every process profile; the merge
    # step below then sees the complete representative workload set.
    env LLVM_PROFILE_FILE="$profile_data/%p-%m.profraw" \
        "$@" "$instrumented_target/$host_target/dist/explora" --benchmark "$presents"
}

build_instrumented

# These workloads deliberately exercise different hot paths. Keep the same
# resolution/preset/driver/power state when comparing a PGO binary with an
# unprofiled binary; the profile itself is not a correctness or FPS result.
# Dist builds ignore DEBUG_ONLY quality/pacing/RT overrides and always run the
# playable cinematic preset, so PGO trains the shipping path.
run_workload cruise \
    EXPLORA_WIND=0
run_workload bank-boost \
    EXPLORA_WIND=1
run_workload terrain-heavy \
    EXPLORA_WIND=3

shopt -s nullglob
raw_profiles=("$profile_data"/*.profraw)
shopt -u nullglob
if ((${#raw_profiles[@]} == 0)); then
    echo "training produced no .profraw files" >&2
    exit 1
fi

echo "[pgo] merging ${#raw_profiles[@]} raw profile(s) with $llvm_profdata"
"$llvm_profdata" merge -output="$merged_profile" "${raw_profiles[@]}"

echo "[pgo] building PGO-use dist binary"
use_flags="-Cprofile-use=$merged_profile"
# Cargo fingerprints the profile-use path, not the contents of the merged
# profile. Reusing this target after a new merge can leave dependency bitcode
# carrying an older LLVM profile summary, which fat-LTO correctly rejects.
if [[ -d "$use_target" ]]; then
    cargo clean --target-dir "$use_target"
fi
CARGO_TARGET_DIR="$use_target" \
    RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$use_flags" \
    cargo build --profile dist -p explora-engine --target "$host_target"

echo "[pgo] complete: $use_target/$host_target/dist/explora"
echo "[pgo] compare it with cargo framebench under identical workload and power conditions"
