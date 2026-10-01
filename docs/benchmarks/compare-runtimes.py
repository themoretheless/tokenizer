"""Build once, then rotate available engines across independent benchmark processes."""
import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time
import tempfile

root = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--engines', nargs='+', choices=['Rush', 'CPython', 'Rhai', 'Koto', 'Lua', 'Luau', 'Wren'],
                    default=['Rush', 'CPython', 'Rhai', 'Koto', 'Lua', 'Luau'])
parser.add_argument('--cases', nargs='+',
                    choices=['closure', 'collections', 'collections_lazy'],
                    default=['closure', 'collections'])
parser.add_argument('--operations', nargs='+', choices=['compile', 'prepared_run', 'compile_and_run'],
                    default=['compile', 'prepared_run', 'compile_and_run'])
parser.add_argument('--wren-source', type=Path)
args = parser.parse_args()
if len(set(args.operations)) != len(args.operations):
    parser.error('Duplicate operation')
if 'Wren' in args.engines:
    if args.operations != ['prepared_run']:
        parser.error('Wren currently supports only --operations prepared_run')
    if not args.wren_source or not (args.wren_source / 'src/include/wren.h').is_file():
        parser.error('Wren requires --wren-source pointing to extracted Wren 0.4.0')
if len(set(args.engines)) != len(args.engines):
    parser.error('Duplicate engine')
if len(set(args.cases)) != len(args.cases):
    parser.error('Duplicate case')
if 'Rhai' in args.engines and 'collections_lazy' in args.cases:
    parser.error('The Rhai adapter does not implement collections_lazy')

def build(command, target):
    completed = subprocess.run(command + ['--message-format=json'], cwd=root,
                               check=True, capture_output=True, text=True)
    artifacts = [json.loads(line) for line in completed.stdout.splitlines() if line.startswith('{')]
    executables = [x['executable'] for x in artifacts if x.get('reason') == 'compiler-artifact'
                   and x.get('target', {}).get('name') == target and x.get('executable')]
    assert len(executables) == 1, executables
    return executables[0]

temporary = tempfile.TemporaryDirectory(prefix="rush-comparison-")
commands = {}
for engine in args.engines:
    if engine == 'CPython':
        commands[engine] = [sys.executable, str(root / 'docs/benchmarks/python-runtime.py')]
    elif engine == 'Wren':
        executable = str(Path(temporary.name) / 'wren-bench')
        subprocess.run(['sh', str(root / 'docs/benchmarks/wren-runtime/build.sh'),
                        str(args.wren_source.resolve()), executable], check=True, cwd=root)
        commands[engine] = [executable]
    elif engine == 'Rush':
        commands[engine] = [build(['cargo', 'bench', '--locked', '-p',
            'themoretheless-tokenizer-rush', '--bench', 'runtime', '--no-run'], 'runtime')]
    else:
        name = engine.lower()
        commands[engine] = [build(['cargo', 'build', '--locked', '--release',
            '--manifest-path', f'docs/benchmarks/{name}-runtime/Cargo.toml'],
            f'rush-{name}-comparison')]
report = {'date': datetime.datetime.now(datetime.timezone.utc).isoformat(),
          'platform': platform.platform(), 'python': sys.version,
          'method': f'{len(commands)} rounds; rotated engine order; 7 samples x 100 operations; 10 warmups',
          'engines': args.engines, 'cases': args.cases, 'operations': args.operations, 'items': 1000,
          'runs': []}
engines = list(commands)
for case in args.cases:
    for operation in args.operations:
        for round_index in range(len(engines)):
            for engine in engines[round_index:] + engines[:round_index]:
                env = dict(os.environ, RUSH_BENCH_CASE=case, RUSH_BENCH_OPERATION=operation,
                           RUSH_BENCH_ITEMS='1000', RUSH_BENCH_ITERS='100')
                start = time.perf_counter()
                result = subprocess.run(commands[engine], cwd=root, env=env, text=True,
                                        capture_output=True, timeout=60, check=True)
                process_ms = (time.perf_counter() - start) * 1000
                rows = [line.split('\t') for line in result.stdout.splitlines()
                        if line.startswith(case + '\t' + operation + '\t')]
                assert len(rows) == 1 and len(rows[0]) == 6, result.stdout
                values = [float(x) for x in rows[0][3:]]
                report['runs'].append(dict(engine=engine, case=case, operation=operation,
                    round=round_index, min_us=values[0], median_us=values[1], max_us=values[2],
                    process_ms=process_ms, stdout=result.stdout, stderr=result.stderr))
                args.output.write_text(json.dumps(report, indent=2) + '\n')
for case in args.cases:
    for operation in args.operations:
        for engine in engines:
            values = [r['median_us'] for r in report['runs'] if
                      (r['case'], r['operation'], r['engine']) == (case, operation, engine)]
            print(f'{case}\t{operation}\t{engine}\t{min(values):.3f}\t'
                  f'{statistics.median(values):.3f}\t{max(values):.3f}')
