"""macOS sampling profile of a verified prepared Rush workload."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--case', choices=['closure', 'collections', 'collections_lazy',
                                          'cells', 'cells_cycles', 'surface'], required=True)
    parser.add_argument('--iterations', type=int, default=10000)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    assert platform.system() == 'Darwin', 'Requires macOS sample'
    assert args.iterations > 0
    output = args.output.resolve()
    env = dict(os.environ, CARGO_PROFILE_BENCH_DEBUG='1')
    command = ['cargo', 'bench', '--locked', '-p', 'themoretheless-tokenizer-rush',
               '--bench', 'runtime', '--no-run', '--message-format=json']
    build = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True, check=True)
    artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
    binaries = [item['executable'] for item in artifacts
                if item.get('reason') == 'compiler-artifact' and item.get('executable')
                and item.get('target', {}).get('name') == 'runtime']
    assert len(binaries) == 1, binaries
    env.update(RUSH_BENCH_CASE=args.case, RUSH_BENCH_OPERATION='prepared_run',
               RUSH_BENCH_ITERS=str(args.iterations), RUSH_BENCH_ITEMS='1000')
    with output.with_suffix('.bench.txt').open('w') as log:
        child = subprocess.Popen(binaries, cwd=ROOT, env=env, stdout=log, stderr=log)
        try:
            time.sleep(0.5)
            assert child.poll() is None, 'Benchmark finished before sampling'
            sample = subprocess.run(['/usr/bin/sample', str(child.pid), '5', '1',
                                     '-file', str(output)], capture_output=True, text=True, timeout=30)
            metadata = dict(platform=platform.platform(), case=args.case, items=1000,
                iterations=args.iterations, build_command=command, executable=binaries[0],
                executable_sha256=hashlib.sha256(Path(binaries[0]).read_bytes()).hexdigest(),
                bench_debug='1', sample_returncode=sample.returncode,
                sample_stdout=sample.stdout, sample_stderr=sample.stderr,
                method='5s sample at 1ms interval; benchmark validates result before timing; '
                'prepared runs with fresh runtime; instrumented timings are not baseline timings')
            output.with_suffix('.json').write_text(json.dumps(metadata, indent=2) + '\n')
            sample.check_returncode()
            metadata['benchmark_returncode'] = child.wait(timeout=180)
            output.with_suffix('.json').write_text(json.dumps(metadata, indent=2) + '\n')
            assert metadata['benchmark_returncode'] == 0, 'Benchmark failed'
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
    print(output)


if __name__ == '__main__':
    main()
