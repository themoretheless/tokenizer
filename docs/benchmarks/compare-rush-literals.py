"""Alternate saved before/after runtime benchmark binaries on the same workload."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binaries = {'before': args.before.resolve(), 'after': args.after.resolve()}
    report = dict(platform=platform.platform(),
        method='Six alternating process pairs; each verifies result, warms up 10 times, '
               'then measures 7 samples of 200 prepared runs. No profiler or counting allocator.',
        environment=dict(RUSH_BENCH_CASE='collections_lazy', RUSH_BENCH_ITEMS='1000',
                         RUSH_BENCH_ITERS='200', RUSH_BENCH_OPERATION='prepared_run'),
        binaries={name: dict(path=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest())
                  for name, path in binaries.items()}, runs=[])
    env = dict(os.environ, **report['environment'])
    for pair in range(6):
        for variant in (('before', 'after') if pair % 2 == 0 else ('after', 'before')):
            result = subprocess.run([str(binaries[variant])], env=env, capture_output=True,
                                    text=True, timeout=30, check=True)
            rows = [line.split('\t') for line in result.stdout.splitlines()
                    if line.startswith('collections_lazy\tprepared_run\t')]
            assert len(rows) == 1 and rows[0][2] == '200', result.stdout
            assert 'verified expected sum: 333666' in result.stdout
            report['runs'].append(dict(pair=pair, variant=variant, stdout=result.stdout,
                stderr=result.stderr, min_us=float(rows[0][3]), median_us=float(rows[0][4]),
                max_us=float(rows[0][5])))
            args.output.write_text(json.dumps(report, indent=2) + '\n')
    for variant in binaries:
        medians = [row['median_us'] for row in report['runs'] if row['variant'] == variant]
        print(variant, 'median of process medians, us:', statistics.median(medians), medians)


if __name__ == '__main__':
    main()
