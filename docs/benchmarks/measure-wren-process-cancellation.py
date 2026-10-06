"""Measure SIGKILL of isolated Wren processes, not cooperative VM cancellation."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import selectors
import signal
import statistics
import subprocess
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    report = dict(platform=platform.platform(), version='Wren 0.4.0',
                  executable_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  method='SIGKILL 5ms after host entry marker; time until process reaped. '
                  '3 warmups and 25 samples per scenario; blocking host sleeps 25ms. '
                  'New-process probe after each termination, no reuse or VM cleanup guarantee.',
                  runs=[])
    for scenario in ('loop', 'blocking_host'):
        for sample in range(-3, 25):
            child = subprocess.Popen([str(binary), scenario], stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE)
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(child.stdout, selectors.EVENT_READ)
                    assert selector.select(5), 'No entry marker'
                assert child.stdout.readline() == b'entered\n'
                entered = time.perf_counter_ns()
                time.sleep(0.005)
                assert child.poll() is None, 'Process exited before request'
                requested = time.perf_counter_ns()
                child.kill()
                child.wait(timeout=5)
                reaped = time.perf_counter_ns()
                stdout, stderr = child.communicate()
                assert child.returncode == -signal.SIGKILL, (stdout, stderr)
                assert not stderr, stderr
                probe = subprocess.run([str(binary), 'probe'], capture_output=True,
                                       text=True, timeout=5, check=True)
                assert probe.stdout.strip() == 'fresh_vm_result=3' and not probe.stderr
                report['runs'].append(dict(scenario=scenario, sample=sample,
                    entry_to_request_us=(requested-entered)/1000,
                    request_to_reaped_us=(reaped-requested)/1000,
                    host_returned_before_termination=b'host_returned' in stdout,
                    returncode=child.returncode, fresh_vm_result=3))
                args.output.write_text(json.dumps(report, indent=2) + '\n')
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=5)
                child.stdout.close()
                child.stderr.close()
        rows = [r for r in report['runs'] if r['scenario'] == scenario and r['sample'] >= 0]
        timings = [r['request_to_reaped_us'] for r in rows]
        print(scenario, 'min/median/max us', min(timings), statistics.median(timings),
              max(timings), 'host returned', sum(r['host_returned_before_termination'] for r in rows))


if __name__ == '__main__':
    main()
