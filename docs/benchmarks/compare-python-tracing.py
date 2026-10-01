"""Alternate tracing on/off in fresh benchmark processes."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys

parser = argparse.ArgumentParser()
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
rows = []
cases = ('closure','collections','collections_lazy')
for case in cases:
    for round_index in range(4):
        for enabled in (('0','1') if round_index % 2 == 0 else ('1','0')):
            result = subprocess.run([sys.executable,str(Path(__file__).with_name('python-runtime.py'))],
                env=dict(os.environ,RUSH_BENCH_TRACE=enabled,RUSH_BENCH_OPERATION='prepared_run',
                         RUSH_BENCH_ITEMS='1000',RUSH_BENCH_ITERS='100',RUSH_BENCH_CASE=case),
                capture_output=True,text=True,check=True,timeout=30)
            lines = [line.split('\t') for line in result.stdout.splitlines() if line.startswith(case+'\t')]
            assert len(lines)==1 and len(lines[0])==6
            rows.append(dict(case=case,round=round_index,tracing=enabled=='1',median_us=float(lines[0][4]),stdout=result.stdout))
            args.output.write_text(json.dumps(dict(method='four alternating rounds; three workloads; N=1000; prepared execution',runs=rows),indent=2)+'\n')
for case in cases:
    for enabled in (False,True):
        values=[r['median_us'] for r in rows if r['case']==case and r['tracing']==enabled]
        print(f'{case} tracing={enabled}: {statistics.median(values):.3f} us')
