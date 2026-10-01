"""Whole-process JSONL timing, including startup and file IO; verified outputs."""
import argparse
import datetime
import json
from pathlib import Path
import platform
import re
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--worker', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--rss', action='store_true', help='macOS /usr/bin/time -l peak RSS')
    parser.add_argument('--lua', type=Path, help='Built rush-lua-records executable')
    parser.add_argument('--luau', type=Path, help='Built rush-luau-records executable')
    parser.add_argument('--rhai', type=Path, help='Built Rhai records executable')
    parser.add_argument('--koto', type=Path, help='Built Koto records executable')
    parser.add_argument('--wren', type=Path, help='Built rush-wren-records executable')
    parser.add_argument('--groups', type=int, default=10, help='Possible grouping keys, 1..1000')
    args = parser.parse_args()
    if args.worker:
        from python_records import main as worker
        worker(['--jsonl', str(args.worker)])
        return
    if not args.output:
        parser.error('--output is required outside worker mode')
    if args.rss and sys.platform != 'darwin':
        parser.error('--rss currently requires macOS')
    if not 1 <= args.groups <= 1000:
        parser.error('--groups must be between 1 and 1000')
    build = subprocess.run(['cargo','build','--locked','--release','-p',
        'themoretheless-tokenizer-rush','--example','record_summary','--message-format=json'],
        cwd=ROOT, capture_output=True, text=True, check=True)
    artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
    binaries = [x['executable'] for x in artifacts if x.get('reason') == 'compiler-artifact'
        and x.get('target', {}).get('name') == 'record_summary' and x.get('executable')]
    assert len(binaries) == 1
    engines = ['Rush', 'CPython'] + [engine for engine, path in
        [('Lua', args.lua), ('Luau', args.luau), ('Rhai', args.rhai), ('Koto', args.koto), ('Wren', args.wren)] if path]
    rounds = ((10 + len(engines) - 1) // len(engines)) * len(engines)
    report = dict(date=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        platform=platform.platform(), python=sys.version,
        method=f'whole process; warm filesystem cache; 2 warmups, {rounds} cyclic rounds',
        engines=engines,
        possible_groups=args.groups,
        python_adapter='python_records.py; standalone host without benchmark imports',
        rss=args.rss, runs=[])
    with tempfile.TemporaryDirectory(prefix='rush-json-timing-') as directory:
        path = Path(directory) / 'records.jsonl'
        for items in (1000, 10000):
            totals = {}
            with path.open('w') as stream:
                for i in range(items):
                    group = 'группа-' + str(i % args.groups)
                    amount = (i % 17 - 8) * 0.5
                    active = i % 4 != 0
                    stream.write(json.dumps(dict(group=group, amount=amount, active=active),
                        ensure_ascii=False) + '\n')
                    if active:
                        totals[group] = totals.get(group, 0.0) + amount
            expected = [dict(group=k,total=v) for k,v in totals.items()]
            commands = {'Rush':[binaries[0],'--jsonl',str(path)],
                'CPython':[sys.executable,str(Path(__file__).with_name('python_records.py')),'--jsonl',str(path)]}
            for engine, binary in [('Lua', args.lua), ('Luau', args.luau), ('Rhai', args.rhai), ('Koto', args.koto), ('Wren', args.wren)]:
                if binary:
                    commands[engine] = [str(binary.resolve()), '--jsonl', str(path)]
            for round_index in range(-2, rounds):
                offset = round_index % len(engines)
                order = engines[offset:] + engines[:offset]
                for engine in order:
                    start = time.perf_counter_ns()
                    command = (['/usr/bin/time', '-l'] if args.rss else []) + commands[engine]
                    result = subprocess.run(command,cwd=ROOT,capture_output=True,
                        text=True,timeout=30,check=True)
                    elapsed = (time.perf_counter_ns()-start)/1e6
                    assert json.loads(result.stdout) == expected, (engine,items,result.stdout)
                    resources = {}
                    if args.rss:
                        matches = re.findall(r'^\s*(\d+)\s+maximum resident set size$', result.stderr, re.M)
                        assert len(matches) == 1, result.stderr
                        resources = dict(peak_rss_bytes=int(matches[0]), resource_output=result.stderr)
                    if round_index >= 0:
                        report['runs'].append(dict(engine=engine,items=items,round=round_index,
                            milliseconds=elapsed,verified=True,result_groups=len(expected),**resources))
                        args.output.write_text(json.dumps(report,indent=2)+'\n')
    for items in (1000,10000):
        for engine in engines:
            times = [r['milliseconds'] for r in report['runs'] if r['engine']==engine and r['items']==items]
            print(f'{engine}\t{items}\t{min(times):.3f}\t{statistics.median(times):.3f}\t{max(times):.3f}')
            if args.rss:
                peaks = [r['peak_rss_bytes']/1048576 for r in report['runs'] if r['engine']==engine and r['items']==items]
                print(f'{engine}\t{items}\tRSS_MiB\t{min(peaks):.3f}\t{statistics.median(peaks):.3f}\t{max(peaks):.3f}')

if __name__ == '__main__':
    main()
