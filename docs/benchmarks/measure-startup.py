"""Fresh process + engine + trivial compile/run; filesystem caches are not flushed."""
import argparse
import datetime
import hashlib
import json
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]

def build(arguments, target):
    result = subprocess.run(['cargo', 'build', '--locked', '--offline', '--release',
        *arguments, '--message-format=json'], cwd=ROOT, capture_output=True, text=True, check=True)
    artifacts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
    binaries = [item['executable'] for item in artifacts if item.get('reason') == 'compiler-artifact'
        and item.get('target', {}).get('name') == target and item.get('executable')]
    assert len(binaries) == 1, binaries
    return binaries[0]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--wren-source', type=Path, required=True)
    parser.add_argument('--nu', type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='rush-startup-') as temporary:
        commands = {'Rush': [build(['-p', 'themoretheless-tokenizer-rush', '--example', 'startup'], 'startup')],
            'CPython': [sys.executable, '-I', '-S', '-c', 'print(1+2)']}
        for engine in ['Lua', 'Luau', 'Rhai', 'Koto']:
            target = f'rush-{engine.lower()}-startup' if engine in ['Lua', 'Luau'] else 'startup'
            commands[engine] = [build(['--manifest-path', f'docs/benchmarks/{engine.lower()}-runtime/Cargo.toml',
                '--bin', target], target)]
        wren = str(Path(temporary) / 'wren-startup')
        subprocess.run(['sh', str(ROOT / 'docs/benchmarks/wren-runtime/build.sh'),
            str(args.wren_source.resolve()), wren, 'startup'], cwd=ROOT, check=True)
        commands['Wren'] = [wren]
        commands['Nushell'] = [str(args.nu.resolve()), '--no-config-file', '--no-history', '-c', '1 + 2']
        report = dict(date=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=platform.platform(),
            python=sys.version, method='fresh process and engine; compile/run 1+2; first launch separate; 16 cyclic rounds; binaries read for SHA256 before launches; OS caches not flushed',
            commands=commands, executable_sha256={engine: hashlib.sha256(Path(command[0]).read_bytes()).hexdigest()
                for engine, command in commands.items()}, runs=[])
        engines = list(commands)
        for round_index in range(-1, 16):
            offset = max(round_index, 0) % len(engines)
            for engine in engines[offset:] + engines[:offset]:
                start = time.perf_counter_ns()
                result = subprocess.run(commands[engine], cwd=ROOT, capture_output=True, text=True, timeout=10)
                elapsed = (time.perf_counter_ns() - start) / 1e6
                assert result.returncode == 0, (engine, result.returncode, result.stderr)
                assert result.stdout.strip() == '3', (engine, result.stdout, result.stderr)
                report['runs'].append(dict(engine=engine, round=round_index, milliseconds=elapsed,
                    verified=True, stderr=result.stderr))
                args.output.write_text(json.dumps(report, indent=2) + '\n')
        for engine in engines:
            rows = [row for row in report['runs'] if row['engine'] == engine]
            times = [row['milliseconds'] for row in rows if row['round'] >= 0]
            print(f'{engine}\tfirst={rows[0]["milliseconds"]:.3f}\t{min(times):.3f}\t{statistics.median(times):.3f}\t{max(times):.3f}')

if __name__ == '__main__':
    main()
