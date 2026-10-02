#!/usr/bin/env python3
"""Compare optimized, correctness-checking Rush bench executables sequentially."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("before", type=Path)
parser.add_argument("after", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--bench", choices=["loading", "runtime", "coroutine"], default="loading")
parser.add_argument("--operation", choices=["compile", "prepared_run", "compile_and_run"], default="prepared_run")
parser.add_argument("--pairs", type=int, default=6)
parser.add_argument("--iterations", type=int, default=200)
parser.add_argument("--cases", default="")
args = parser.parse_args()
assert args.pairs > 0 and args.iterations > 0
cases = args.cases.split(",") if args.cases else (["plain", "host", "modules_1", "modules_8", "modules_24"] if args.bench == "loading" else ["while", "for", "yield"] if args.bench == "coroutine" else ["closure", "collections", "collections_lazy", "cells", "cells_cycles", "surface"])
executables = {name: path.resolve() for name, path in [("before", args.before), ("after", args.after)]}
result = {
    "method": "fresh processes in alternating order; correctness assertions, 10 warmups and 7 samples per process; no profiler/allocator instrumentation",
    "bench": args.bench,
    "operation": "instantiate" if args.bench == "loading" else "spawn_resume_drop" if args.bench == "coroutine" else args.operation,
    "iterations": args.iterations,
    "pairs": args.pairs,
    "binaries": {name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()} for name, path in executables.items()},
    "records": [],
    "summary": {},
}
for case in cases:
    medians = {"before": [], "after": []}
    for pair in range(args.pairs):
        for name in (["before", "after"] if pair % 2 == 0 else ["after", "before"]):
            env = os.environ.copy()
            env["RUSH_BENCH_ITERS"] = str(args.iterations)
            if args.bench == "loading":
                env["RUSH_LOADING_CASE"] = case
            elif args.bench == "runtime":
                env["RUSH_BENCH_CASE"] = case
                env["RUSH_BENCH_OPERATION"] = args.operation
                env["RUSH_BENCH_ITEMS"] = "1000"
            run = subprocess.run([str(executables[name])], env=env, text=True, capture_output=True, check=True)
            rows = [line.split("\t") for line in run.stdout.splitlines() if line.startswith(case + "\t")]
            assert len(rows) == 1, run.stdout
            row = rows[0]
            samples = list(map(float, row[-3:]))
            assert len(row) == (6 if args.bench == "runtime" else 5), row
            medians[name].append(samples[1])
            result["records"].append({"case": case, "pair": pair, "binary": name, "min_us": samples[0], "median_us": samples[1], "max_us": samples[2]})
    before, after = (statistics.median(medians[name]) for name in ["before", "after"])
    result["summary"][case] = {"before_median_us": before, "after_median_us": after, "speedup": before / after,
                              "before_process_range_us": [min(medians["before"]), max(medians["before"])], "after_process_range_us": [min(medians["after"]), max(medians["after"])]}
    print(f"{case}: {before:.3f} -> {after:.3f} us ({before/after:.2f}x)", flush=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
