"""Check JSON record transformation parity; no timing claims."""
import argparse
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]

def summarize(text):
    rows = json.loads(text)
    if not isinstance(rows, list) or len(rows) > 1000:
        raise ValueError('Expected array of at most 1000 rows')
    return summarize_rows(rows)


def summarize_lines(path):
    def rows():
        with path.open('rb') as stream:
            while True:
                line = stream.readline(65537)
                if not line:
                    return
                if len(line) > 65536:
                    raise ValueError('line exceeds byte limit')
                yield json.loads(line.decode('utf-8'))
    return summarize_rows(rows())


def summarize_rows(rows):
    totals = {}
    for row in rows:
        if not isinstance(row, dict) or not isinstance(row.get('group'), str):
            raise ValueError('group must be a string')
        amount = row.get('amount')
        if type(amount) not in (int, float) or not math.isfinite(float(amount)):
            raise ValueError('amount must be finite')
        if type(row.get('active')) is not bool:
            raise ValueError('active must be boolean')
        if row['active']:
            group = row['group']
            totals[group] = totals.get(group, 0.0) + float(amount)
            if not math.isfinite(totals[group]):
                raise ValueError('total must be finite')
    return [{'group': group, 'total': total} for group, total in totals.items()]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--nu', type=Path, help='Optional Nushell executable for JSON array cases')
    parser.add_argument('--lua', type=Path, help='Optional rush-lua-records executable')
    parser.add_argument('--luau', type=Path, help='Optional rush-luau-records executable')
    parser.add_argument('--rhai', type=Path, help='Optional Rhai records executable')
    parser.add_argument('--koto', type=Path, help='Optional Koto records executable')
    parser.add_argument('--wren', type=Path, help='Optional rush-wren-records executable')
    args = parser.parse_args()
    build = subprocess.run(['cargo', 'build', '--locked', '--release', '-p',
        'themoretheless-tokenizer-rush', '--example', 'record_summary',
        '--message-format=json'], cwd=ROOT, capture_output=True, text=True, check=True)
    artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith('{')]
    binaries = [x['executable'] for x in artifacts if x.get('reason') == 'compiler-artifact'
        and x.get('target', {}).get('name') == 'record_summary' and x.get('executable')]
    assert len(binaries) == 1
    def row(group, amount, active=True):
        return dict(group=group, amount=amount, active=active)
    cases = [
        ('empty', [], []),
        ('unicode-order-filter', [row('Б"\n', 2.5), row('ignored', 99, False),
            row('next', -4), row('Б"\n', -1)],
            [{'group':'Б"\n', 'total':1.5}, {'group':'next', 'total':-4.0}]),
        ('inactive', [row('a', 2, False)], []),
        ('zero', [row('a', 0)], [{'group':'a', 'total':0.0}]),
        ('empty-and-nul-keys', [row('', 2), row('\0', 3), row('', -2)],
            [{'group':'', 'total':0.0}, {'group':'\0', 'total':3.0}]),
        ('inactive-before-active', [row('b', 9, False), row('a', 2), row('b', 3)],
            [{'group':'a', 'total':2.0}, {'group':'b', 'total':3.0}]),
        ('not-array', {}, None),
        ('non-object-row', [None], None),
        ('null-group', [row(None, 1)], None),
        ('numeric-active', [row('a', 1, 1)], None),
        ('missing-field', [{'group':'a','amount':1}], None),
        ('boolean-amount', [row('a', True)], None),
        ('string-amount', [row('a', '1')], None),
        ('inactive-invalid', [row('a', 'bad', False)], None),
        ('overflow', [row('a', 1e308), row('a', 1e308)], None),
        ('row-limit', [row('a', 1)] * 1001, None),
    ]
    results = []
    with tempfile.TemporaryDirectory(prefix='rush-json-') as directory:
        path = Path(directory) / 'input.json'
        for name, value, expected in cases:
            text = json.dumps(value, ensure_ascii=False, allow_nan=False)
            path.write_text(text)
            try:
                python = summarize(text)
                python_ok = True
            except ValueError:
                python, python_ok = None, False
            rush = subprocess.run([binaries[0], str(path)], capture_output=True,
                                  text=True, timeout=10, cwd=ROOT)
            assert python_ok == (expected is not None), (name, python)
            assert (rush.returncode == 0) == python_ok, (name, rush.stderr)
            if python_ok:
                assert json.loads(rush.stdout) == python == expected, (name, rush.stdout)
            additional = {}
            standalone = subprocess.run([sys.executable, str(Path(__file__).with_name('python_records.py')), str(path)],
                capture_output=True, text=True, timeout=10, cwd=ROOT)
            assert standalone.returncode == (0 if python_ok else 1), (name, standalone.stderr)
            if python_ok:
                assert json.loads(standalone.stdout) == expected, (name, standalone.stdout)
            additional.update(python_adapter_verified=True, python_stderr=standalone.stderr)
            for engine in ('lua', 'luau', 'rhai', 'koto', 'wren'):
                binary = getattr(args, engine)
                if binary:
                    result = subprocess.run([str(binary.resolve()), str(path)],
                        capture_output=True, text=True, timeout=10, cwd=ROOT)
                    assert result.returncode in (0, 1), (engine, name, result.returncode, result.stderr)
                    assert 'ERROR: AddressSanitizer' not in result.stderr, (engine, name, result.stderr)
                    assert (result.returncode == 0) == python_ok, (engine, name, result.stderr)
                    if python_ok:
                        assert json.loads(result.stdout) == expected, (engine, name, result.stdout)
                    additional[engine + '_verified'] = True
                    additional[engine + '_stderr'] = result.stderr
            if args.nu:
                nu = subprocess.run([str(args.nu.resolve()), '--no-config-file', '--no-history',
                    str(ROOT / 'docs/benchmarks/nushell-records.nu'), str(path)],
                    capture_output=True, text=True, timeout=10, cwd=ROOT)
                assert (nu.returncode == 0) == python_ok, (name, nu.stdout, nu.stderr)
                if python_ok:
                    assert json.loads(nu.stdout) == expected, (name, nu.stdout)
                additional.update(nushell_verified=True, nushell_stderr=nu.stderr)
            results.append(dict(case=name, accepted=python_ok, expected=expected,
                                rush_stderr=rush.stderr, **additional))
        valid_line = json.dumps(row('a', 1)).encode()
        line_cases = [
            ('empty', b'', []),
            ('crlf-final-no-newline', valid_line + b'\r\n' + valid_line,
                [{'group':'a', 'total':2.0}]),
            ('large-stream', (valid_line + b'\n') * 10000,
                [{'group':'a', 'total':10000.0}]),
            ('blank-line', valid_line + b'\n\n', None),
            ('malformed-second-line', valid_line + b'\n{', None),
            ('invalid-utf8', b'\xff', None),
            ('exact-byte-limit', valid_line + b' ' * (65536-len(valid_line)),
                [{'group':'a', 'total':1.0}]),
            ('over-byte-limit', valid_line + b' ' * (65537-len(valid_line)), None),
        ]
        for name, data, expected in line_cases:
            path.write_bytes(data)
            try:
                python = summarize_lines(path)
                python_ok = True
            except ValueError:
                python, python_ok = None, False
            rush = subprocess.run([binaries[0], '--jsonl', str(path)],
                capture_output=True, text=True, timeout=10, cwd=ROOT)
            assert python_ok == (expected is not None), (name, python)
            assert (rush.returncode == 0) == python_ok, (name, rush.stderr)
            if python_ok:
                assert json.loads(rush.stdout) == python == expected, (name, rush.stdout)
            additional = {}
            standalone = subprocess.run([sys.executable, str(Path(__file__).with_name('python_records.py')), '--jsonl', str(path)],
                capture_output=True, text=True, timeout=10, cwd=ROOT)
            assert standalone.returncode == (0 if python_ok else 1), (name, standalone.stderr)
            if python_ok:
                assert json.loads(standalone.stdout) == expected, (name, standalone.stdout)
            additional.update(python_adapter_verified=True, python_stderr=standalone.stderr)
            for engine in ('lua', 'luau', 'rhai', 'koto', 'wren'):
                binary = getattr(args, engine)
                if binary:
                    result = subprocess.run([str(binary.resolve()), '--jsonl', str(path)],
                        capture_output=True, text=True, timeout=10, cwd=ROOT)
                    assert result.returncode in (0, 1), (engine, name, result.returncode, result.stderr)
                    assert 'ERROR: AddressSanitizer' not in result.stderr, (engine, name, result.stderr)
                    assert (result.returncode == 0) == python_ok, (engine, name, result.stderr)
                    if python_ok:
                        assert json.loads(result.stdout) == expected, (engine, name, result.stdout)
                    additional[engine + '_verified'] = True
                    additional[engine + '_stderr'] = result.stderr
            results.append(dict(case='jsonl-' + name, accepted=python_ok,
                expected=expected, rush_stderr=rush.stderr, **additional))
    args.output.write_text(json.dumps({'method':'functional parity; no timings',
                                      'cases':results}, ensure_ascii=False, indent=2) + '\n')
    print(f'{len(results)} JSON scenarios passed for Rush and CPython')
    if args.nu:
        print(f'{len(cases)} JSON array scenarios also passed for Nushell')
    for engine in ('lua', 'luau', 'rhai', 'koto', 'wren'):
        if getattr(args, engine):
            print(f'{len(results)} JSON/JSONL scenarios also passed for {engine}')

if __name__ == '__main__':
    main()
