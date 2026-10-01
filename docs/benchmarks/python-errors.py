"""Fixed syntax/runtime/host diagnostic cases; no subjective quality ranking."""
import platform
import traceback

class HostError(Exception):
    pass

def host_fail():
    raise HostError('host rejected operation')

def reuse(environment):
    exec(compile('result = 1+2', 'reuse-case.py', 'exec'), environment)
    assert environment['result'] == 3

def main():
    environment = {'host_fail': host_fail}
    reports = []
    try:
        compile('value =\n', 'syntax-case.py', 'exec')
    except SyntaxError as error:
        assert error.filename == 'syntax-case.py' and error.lineno == 1
        reports.append(('SYNTAX', ''.join(traceback.format_exception(error))))
    else:
        raise AssertionError('Expected SyntaxError')
    reuse(environment)
    for source, filename, kind, line in [
        ('value = None\nvalue.missing', 'runtime-case.py', AttributeError, 2),
        ('host_fail()', 'host-case.py', HostError, 1),
    ]:
        program = compile(source, filename, 'exec')
        try:
            exec(program, environment)
        except kind as error:
            frames = traceback.extract_tb(error.__traceback__)
            assert any(frame.filename == filename and frame.lineno == line for frame in frames)
            if kind is HostError:
                assert str(error) == 'host rejected operation'
            reports.append((filename, ''.join(traceback.format_exception(error))))
        else:
            raise AssertionError(f'Expected {kind.__name__}')
        reuse(environment)
    environment['HostError'] = HostError
    exec(compile('try:\n    host_fail()\nexcept HostError as error:\n    result = str(error)\n',
                 'caught-host.py', 'exec'), environment)
    assert environment['result'] == 'host rejected operation'
    print(f'CPython {platform.python_version()}: syntax/runtime/host diagnostics and interpreter reuse passed')
    for label, report in reports:
        print(label)
        print(report)

if __name__ == '__main__':
    main()
