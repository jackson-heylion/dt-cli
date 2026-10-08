#!/usr/bin/env python3
"""Compare native startup using interleaved, warmed offline invocations (no credentials)."""
import argparse
import hashlib
import json
import platform
import random
import statistics
import subprocess
import time
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binaries", nargs="+", type=Path)
    parser.add_argument("--samples", type=int, default=31)
    args = parser.parse_args()
    if args.samples < 3:
        parser.error("--samples must be at least 3")
    binaries = [path.resolve(strict=True) for path in args.binaries]
    commands = [["version"], ["help"], ["discover"], ["schema", "iam.workflow.list"],
                ["workflow", "list", "--kind", "todo", "--profile", "benchmark-offline", "--dry-run"]]
    results = {str(path): {"bytes": path.stat().st_size,
                           "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                           "commands": {}} for path in binaries}

    def run(path, command):
        start = time.perf_counter_ns()
        result = subprocess.run([str(path), *command], capture_output=True, check=True, timeout=10)
        elapsed = (time.perf_counter_ns() - start) / 1_000_000
        envelope = json.loads(result.stdout)
        if envelope.get("ok") is not True or result.stderr:
            raise RuntimeError(f"unexpected response: {path} {command}")
        return elapsed

    rng = random.Random(0)
    for command in commands:
        timings = {path: [] for path in binaries}
        for path in binaries:
            for _ in range(3):
                run(path, command)
        for _ in range(args.samples):
            order = binaries.copy()
            rng.shuffle(order)
            for path in order:
                timings[path].append(run(path, command))
        for path, values in timings.items():
            results[str(path)]["commands"][" ".join(command)] = {
                "median_ms": round(statistics.median(values), 3),
                "p95_ms": round(sorted(values)[int((len(values) - 1) * .95)], 3),
            }
    print(json.dumps({"platform": platform.platform(), "samples": args.samples,
                      "method": "warm process wall time; interleaved binaries; offline commands",
                      "results": results}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
