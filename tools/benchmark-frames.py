#!/usr/bin/env python3
"""Capture reproducible real-scene 1 ms frame budgets from the native Vulkan app."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/explora"))
    parser.add_argument("--presents", type=int, default=10000)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--quality", choices=("performance", "balanced", "cinematic"), default="balanced")
    parser.add_argument("--rt-shadows", choices=("auto", "on", "off"), default="auto")
    parser.add_argument("--present", choices=("immediate", "mailbox", "fifo"))
    parser.add_argument("--require", choices=("submission", "display"), default="submission")
    parser.add_argument("--fullscreen", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.presents <= 0 or args.runs <= 0:
        parser.error("--presents and --runs must be positive")
    if not args.binary.is_file():
        parser.error(f"native Vulkan executable not found: {args.binary}")
    env = os.environ.copy()
    env.update(EXPLORA_BURST="1", EXPLORA_QUALITY=args.quality, EXPLORA_RT_SHADOWS=args.rt_shadows)
    # Collect the emitted JSON directly so a stale or shared output file cannot
    # stand in for a failed run. Preserve other scene controls in the environment.
    env.pop("EXPLORA_BENCH_JSON", None)
    if "EXPLORA_NO_GPU" in env:
        parser.error("unset EXPLORA_NO_GPU; this benchmark requires a rendered scene")
    if args.present:
        env["EXPLORA_PRESENT"] = args.present
    command = [str(args.binary.resolve()), "--benchmark", str(args.presents)]
    if args.fullscreen:
        command.append("--fullscreen")
    results = []
    for run in range(args.runs):
        completed = subprocess.run(command, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        print(completed.stdout, end="")
        if completed.returncode:
            sys.exit(f"run {run + 1} failed with exit code {completed.returncode}")
        records = []
        for line in completed.stdout.splitlines():
            if line.startswith("{"):
                record = json.loads(line)
                if record.get("schema") == "explora.frame-budget.v1":
                    records.append(record)
        if len(records) != 1:
            sys.exit(f"run {run + 1}: expected exactly one frame-budget record, found {len(records)}")
        record = records[0]
        if record["config"]["burst"] != 1 or not record["config"]["scene_rendered"]:
            sys.exit(f"run {run + 1}: capture did not render one complete scene per present")
        if record["submission_interval"]["samples"] != args.presents:
            sys.exit(f"run {run + 1}: incomplete submission measurement")
        results.append(record)
    required_key = f"{args.require}_target_met"
    passed = all(result[required_key] is True for result in results)
    artifact = {
        "schema": "explora.frame-budget-runs.v1",
        "required_target": args.require,
        "all_runs_pass": passed,
        "command": command,
        "environment": {key: value for key, value in env.items() if key.startswith("EXPLORA_")},
        "runs": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    print(f"{args.require} 1000 Hz / mean+p99 <= 1 ms: {'PASS' if passed else 'FAIL OR UNVERIFIED'}")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
