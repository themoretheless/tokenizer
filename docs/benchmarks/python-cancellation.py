"""Cooperative cancellation via CPython tracing; not arbitrary thread termination."""
import os
import platform
import sys
import threading
import time

class Cancelled(BaseException):
    pass

scenario = os.environ.get('RUSH_CANCEL_CASE', 'loop')
assert scenario in ('loop', 'blocking_host')
program = compile('entered()\nwhile True:\n    pass\n', '<cancel-benchmark>', 'exec')
print(f'CPython {platform.python_version()}; {scenario}; trace callback; 25 samples after 3 warmups; request 5ms after entry')
print('sample\trequest_to_return_us\treuse_result')
for sample in range(28):
    entered_event = threading.Event()
    cancel_event = threading.Event()
    requested = []

    def entered():
        entered_event.set()
        if scenario == 'blocking_host':
            time.sleep(0.025)

    def requester():
        if not entered_event.wait(5):
            raise RuntimeError('Script did not enter')
        time.sleep(0.005)
        requested.append(time.perf_counter_ns())
        cancel_event.set()

    def trace(frame, event, arg):
        if frame.f_code.co_filename != '<cancel-benchmark>':
            return None
        if cancel_event.is_set():
            raise Cancelled('requested cancellation')
        return trace

    thread = threading.Thread(target=requester)
    thread.start()
    cancelled = False
    previous_trace = sys.gettrace()
    try:
        sys.settrace(trace)
        exec(program, {'entered': entered})
    except Cancelled:
        returned = time.perf_counter_ns()
        cancelled = True
    finally:
        sys.settrace(previous_trace)
        thread.join(timeout=5)
    assert cancelled and not thread.is_alive() and len(requested) == 1
    environment = {}
    exec(compile('result = 1+2', '<reuse>', 'exec'), environment)
    assert environment['result'] == 3
    if sample >= 3:
        print(f'{sample-3}\t{(returned-requested[0])/1000:.3f}\t3')
