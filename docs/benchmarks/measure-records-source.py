"""Count checked-in adapter source, with explicit boundaries and file hashes."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASE = 'docs/benchmarks/'
SOURCES = {
    'Rush': {'script': ['crates/tokenizer-rush/examples/scripts/record-summary.r'],
             'host': ['crates/tokenizer-rush/examples/record_summary.rs']},
    'CPython': {'script': [BASE + 'python_records_transform.py'],
                'host': [BASE + 'python_records.py']},
    'Lua': {'script': [BASE + 'lua-records.lua'],
            'host': [BASE + 'lua-records-shared.rs']},
    'Luau': {'script': [BASE + 'lua-records.lua'],
             'host': [BASE + 'lua-records-shared.rs']},
    'Rhai': {'script': [BASE + 'rhai-runtime/records.rhai'],
             'host': [BASE + 'rhai-runtime/src/bin/records.rs']},
    'Koto': {'script': [BASE + 'koto-runtime/records.koto'],
             'host': [BASE + 'koto-runtime/src/bin/records.rs']},
    'Wren': {'script': [BASE + 'wren-records/records.wren'],
             'host': [BASE + 'wren-records/src/main.rs', BASE + 'wren-records/bridge.c'],
             'build': [BASE + 'wren-records/build.rs']},
    'Nushell': {'combined_cli': [BASE + 'nushell-records.nu']},
}


def measure(path):
    data = (ROOT / path).read_bytes()
    lines = data.decode('utf-8').splitlines()
    # These two files contain only trailing test modules after this marker.
    boundary = len(lines)
    if path in ('crates/tokenizer-rush/examples/record_summary.rs',
                BASE + 'lua-records-shared.rs'):
        boundary = lines.index('#[cfg(test)]')
    return dict(path=path, sha256=hashlib.sha256(data).hexdigest(),
                physical_lines=len(lines), counted_lines=boundary,
                nonblank_lines=sum(bool(line.strip()) for line in lines[:boundary]),
                excluded_trailing_test_lines=len(lines) - boundary)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = {'method': 'Physical and nonblank lines; comments included; trailing Rust tests excluded. '
              'Dependency code, manifests, locks, generated files and benchmark runners excluded. '
              'Build category contains the custom Wren build script only. '
              'Nushell combines IO, validation and transform and supports arrays only. '
              'Shared Lua/Luau sources are counted for each adapter, not summed across engines.',
              'engines': {}}
    for engine, roles in SOURCES.items():
        result['engines'][engine] = {}
        for role, paths in roles.items():
            files = [measure(path) for path in paths]
            counts = {key: sum(file[key] for file in files)
                      for key in ('counted_lines', 'nonblank_lines')}
            result['engines'][engine][role] = dict(files=files, **counts)
        print(engine, {role: value['nonblank_lines']
                       for role, value in result['engines'][engine].items()})
    args.output.write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
