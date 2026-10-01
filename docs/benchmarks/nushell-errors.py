"""Nushell CLI diagnostics in separate processes, not VM recovery."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('--nu', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
nu = str(args.nu.resolve())
version = subprocess.run([nu,'--version'],text=True,capture_output=True,check=True).stdout.strip()
cases = [
    ('syntax', 'let value =\n', 'nu::parser::'),
    ('runtime', 'let value = null\n$value.missing\n', 'nu::shell::'),
    ('command', 'error make {msg: "command rejected operation"}\n', 'command rejected operation'),
]
results = []
with tempfile.TemporaryDirectory(prefix='rush-nu-errors-') as directory:
    for name, source, expected in cases:
        path = Path(directory) / f'{name}-case.nu'
        path.write_text(source)
        result = subprocess.run([nu,'--no-config-file','--no-history','--error-style','plain',str(path)],
                                capture_output=True,text=True,timeout=10)
        assert result.returncode != 0, (name,result)
        assert expected in result.stderr, (name,result.stderr)
        assert path.name in result.stderr, (name,result.stderr)
        results.append(dict(case=name,exit_code=result.returncode,stderr=result.stderr))
        next_process = subprocess.run([nu,'--no-config-file','--no-history','-c','1 + 2'],
                                      capture_output=True,text=True,timeout=10,check=True)
        assert next_process.stdout.strip() == '3'
args.output.write_text(json.dumps(dict(version=version,method='separate CLI processes',cases=results),indent=2)+'\n')
print(f'Nushell {version}: three diagnostic cases passed; subsequent independent processes returned 3')
