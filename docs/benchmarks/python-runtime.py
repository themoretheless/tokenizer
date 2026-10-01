"""CPython counterpart of Rush's runtime benchmark; no external packages."""
import functools
import os
import platform
import statistics
import time
import sys

iterations = int(os.environ.get('RUSH_BENCH_ITERS', '100'))
items = int(os.environ.get('RUSH_BENCH_ITEMS', '1000'))
assert iterations > 0 and 1 <= items <= 1_000_000
selected = os.environ.get('RUSH_BENCH_CASE')
operation = os.environ.get('RUSH_BENCH_OPERATION')
operations = ('compile', 'prepared_run', 'compile_and_run', 'verify')
assert operation is None or operation in operations
workloads = {
    'closure': '''scale = lambda factor: lambda x: x * factor
twice = scale(2.0)
result = twice(21.0)
''',
    'collections': f'''values = list(map(float, range({items})))
mapped = list(map(lambda x: x * 2.0, values))
filtered = list(filter(lambda x: x % 3.0 == 0.0, mapped))
result = reduce(lambda total, x: total + x, filtered, 0.0)
''',
    'collections_lazy': f'''values = map(float, range({items}))
mapped = map(lambda x: x * 2.0, values)
filtered = filter(lambda x: x % 3.0 == 0.0, mapped)
result = reduce(lambda total, x: total + x, filtered, 0.0)
''',
}
assert selected is None or selected in workloads
expected_sum = sum(2 * x for x in range(items) if (2 * x) % 3 == 0)


def prepare(source):
    return compile(source, '<benchmark>', 'exec')


def run(program):
    # Every execution gets a fresh global environment, like Rush Program.run.
    environment = {'reduce': functools.reduce}
    exec(program, environment)
    return environment['result']


def measure(name, operation, action):
    for _ in range(10):
        action()
    samples = []
    for _ in range(7):
        start = time.perf_counter_ns()
        for _ in range(iterations):
            action()
        samples.append((time.perf_counter_ns() - start) / 1000 / iterations)
    print(f'{name}\t{operation}\t{iterations}\t{min(samples):.3f}\t'
          f'{statistics.median(samples):.3f}\t{max(samples):.3f}')


trace_enabled = os.environ.get('RUSH_BENCH_TRACE', '0')
assert trace_enabled in ('0', '1')
if trace_enabled == '1':
    import threading
    cancellation = threading.Event()
    def trace(frame, event, arg):
        if frame.f_code.co_filename != '<benchmark>':
            return None
        if cancellation.is_set():
            raise RuntimeError('requested cancellation')
        return trace
    sys.settrace(trace)
print(f'Trace cancellation checks enabled: {trace_enabled == "1"}')

print(f'{platform.python_implementation()} {platform.python_version()} · {platform.platform()}')
print(f'Collection input items: {items}; verified expected sum: {expected_sum}')
print(('One verified execution, no warmup' if operation == 'verify' else
       '7 samples, 10 warmup operations') + '; floating-point elements; no execution budget')
print('workload\toperation\titerations/sample\tmin_us\tmedian_us\tmax_us')
for name, source in workloads.items():
    if selected is not None and name != selected:
        continue
    program = prepare(source)
    start = time.perf_counter_ns()
    result = run(program)
    elapsed = (time.perf_counter_ns() - start) / 1000
    assert result == (42.0 if name == 'closure' else float(expected_sum)), (name, result)
    if operation == 'verify':
        print(f'{name}\tverify\t1\t{elapsed:.3f}\t{elapsed:.3f}\t{elapsed:.3f}')
        continue
    if operation in (None, 'compile'):
        measure(name, 'compile', lambda: prepare(source))
    if operation in (None, 'prepared_run'):
        measure(name, 'prepared_run', lambda: run(program))
    if operation in (None, 'compile_and_run'):
        measure(name, 'compile_and_run', lambda: run(prepare(source)))
