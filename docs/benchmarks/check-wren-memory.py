"""Run allocator rejection in a disposable child; never inside the host process."""
import argparse
import hashlib
import json
import platform
from pathlib import Path
import resource
import signal
import subprocess


def no_core():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    report = dict(platform=platform.platform(), version='Wren 0.4.0',
                  executable_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  method='Requested VM allocation bytes; headers, host allocations and RSS excluded. '
                  'Prepared call with baseline + 256 KiB cap; NULL on rejected allocation. '
                  'Each scenario uses a separate process with core dumps disabled.', runs=[])
    for mode in ('unlimited', 'capped'):
        result = subprocess.run([str(binary), mode], capture_output=True, text=True,
                                timeout=10, preexec_fn=no_core)
        report['runs'].append(dict(mode=mode, returncode=result.returncode,
                                   stdout=result.stdout, stderr=result.stderr))
        args.output.write_text(json.dumps(report, indent=2) + '\n')
        if mode == 'unlimited':
            assert result.returncode == 0 and 'after_free=0 result=10000' in result.stdout, result
        else:
            assert 'allocator_denied ' in result.stderr, result
            assert 'call_returned ' not in result.stderr, result
            assert result.returncode in (-signal.SIGSEGV, -signal.SIGBUS, -signal.SIGABRT), result
        print(mode, result.returncode, result.stdout.strip(), result.stderr.strip())


if __name__ == '__main__':
    main()
