"""Standalone JSON/JSONL host; validation and IO are outside the transform."""
import json
import math
import sys
from python_records_transform import summarize


def invalid_constant(value):
    raise ValueError(f'Invalid JSON constant: {value}')


def parse(text):
    return json.loads(text, parse_constant=invalid_constant)


def record(row):
    if not isinstance(row, dict) or not isinstance(row.get('group'), str):
        raise ValueError('group must be a string')
    amount = row.get('amount')
    if type(amount) not in (int, float) or not math.isfinite(float(amount)):
        raise ValueError('amount must be finite')
    if type(row.get('active')) is not bool:
        raise ValueError('active must be boolean')
    return row['group'], float(amount), row['active']


def lines(stream):
    while True:
        line = stream.readline(65537)
        if not line:
            return
        if len(line) > 65536:
            raise ValueError('line exceeds byte limit')
        yield record(parse(line.decode('utf-8')))


def main(args):
    if len(args) == 2 and args[0] == '--jsonl':
        with open(args[1], 'rb') as stream:
            result = summarize(lines(stream))
    elif len(args) == 1:
        with open(args[0], encoding='utf-8') as stream:
            rows = parse(stream.read())
        if not isinstance(rows, list) or len(rows) > 1000:
            raise ValueError('Expected array of at most 1000 rows')
        result = summarize([record(row) for row in rows])
    else:
        raise ValueError('Expected [--jsonl] input path')
    print(json.dumps(result, ensure_ascii=False, allow_nan=False))


if __name__ == '__main__':
    try:
        main(sys.argv[1:])
    except (ValueError, OverflowError, OSError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
