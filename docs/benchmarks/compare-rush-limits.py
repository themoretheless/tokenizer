"""Alternate saved runtime benchmark binaries on one workload, with optional env overrides.

Modeled on compare-rush-literals.py; adds per-variant environment overrides so
limit budgets can be toggled without rebuilding the measured binary.
"""
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
    parser.add_argument('--case', required=True)
    parser.add_argument('--before-env', default='{}')
    parser.add_argument('--after-env', default='{}')
    parser.add_argument('--pairs', type=int, default=6)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binaries = {'before': args.before.resolve(), 'after': args.after.resolve()}
    extra_env = {'before': json.loads(args.before_env), 'after': json.loads(args.after_env)}
    report = dict(platform=platform.platform(),
        case=args.case,
        method='Alternating process pairs; each verifies the result, warms up 10 times, '
               'then measures 7 samples of 200 prepared runs. No profiler or counting allocator. '
               'No builds, tests or profilers run during the series.',
        environment=dict(RUSH_BENCH_CASE=args.case, RUSH_BENCH_ITEMS='1000',
                         RUSH_BENCH_ITERS='200', RUSH_BENCH_OPERATION='prepared_run'),
        variant_env=extra_env,
        binaries={name: dict(path=str(path), sha256=hashlib.sha256(path.read_bytes()).hexdigest())
                  for name, path in binaries.items()}, runs=[])
    for pair in range(args.pairs):
        for variant in (('before', 'after') if pair % 2 == 0 else ('after', 'before')):
            env = dict(os.environ, **report['environment'], **extra_env[variant])
            result = subprocess.run([str(binaries[variant])], env=env, capture_output=True,
                                    text=True, timeout=60, check=True)
            prefix = f"{args.case}\tprepared_run\t"
            rows = [line.split('\t') for line in result.stdout.splitlines()
                    if line.startswith(prefix)]
            assert len(rows) == 1 and rows[0][2] == '200', result.stdout
            assert 'verified expected sum' in result.stdout, result.stdout
            report['runs'].append(dict(pair=pair, variant=variant, stdout=result.stdout,
                stderr=result.stderr, min_us=float(rows[0][3]), median_us=float(rows[0][4]),
                max_us=float(rows[0][5])))
            args.output.write_text(json.dumps(report, indent=2) + '\n')
    summary = {}
    for variant in binaries:
        medians = [row['median_us'] for row in report['runs'] if row['variant'] == variant]
        summary[variant] = dict(median_of_medians_us=statistics.median(medians), medians=medians)
        print(variant, 'median of process medians, us:', summary[variant]['median_of_medians_us'],
              medians)
    report['summary'] = summary
    ratio = summary['after']['median_of_medians_us'] / summary['before']['median_of_medians_us']
    report['ratio_after_over_before'] = ratio
    print('after/before ratio:', round(ratio, 4))
    args.output.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
