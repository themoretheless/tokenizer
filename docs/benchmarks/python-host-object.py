"""Host-owned point exposed through a weak reference; functional checks only."""
import math
import platform
import weakref


class Owner:
    def __init__(self, value):
        self.value = value


class Point:
    def __init__(self, owner):
        self._owner = weakref.ref(owner)

    def _resolve(self):
        owner = self._owner()
        if owner is None:
            raise RuntimeError('point was deleted')
        return owner

    def get(self):
        return self._resolve().value

    def move(self, delta):
        owner = self._resolve()
        if type(delta) not in (int, float):
            raise TypeError('delta must be numeric')
        value = owner.value + delta
        if not math.isfinite(value):
            raise ValueError('position must be finite')
        owner.value = value


def read_point(point):
    if not isinstance(point, Point):
        raise TypeError('expected Point')
    return point.get()


def main():
    owner = Owner(2.0)
    environment = {'point': Point(owner), 'read_point': read_point}

    def run(source):
        exec(compile(source, '<host-object>', 'exec'), environment)
        return environment.get('result')

    def rejects(source, kind, message):
        try:
            run(source)
        except kind as error:
            assert message in str(error)
        else:
            raise AssertionError(f'Expected {kind.__name__}: {source}')

    assert run('alias = point\npoint.move(3.0)\nresult = read_point(alias)') == 5.0
    assert owner.value == 5.0
    rejects('result = read_point(42)', TypeError, 'expected Point')
    rejects('point.move(float("inf"))', ValueError, 'position must be finite')
    assert owner.value == 5.0
    del owner
    for source in ['result = point.get()', 'result = alias.get()', 'result = read_point(alias)']:
        rejects(source, RuntimeError, 'point was deleted')
    assert run('result = 1+2') == 3
    print(f'CPython {platform.python_version()}: alias mutation, wrong argument type, '
          'nonfinite rejection, deleted owner and interpreter reuse passed')


if __name__ == '__main__':
    main()
