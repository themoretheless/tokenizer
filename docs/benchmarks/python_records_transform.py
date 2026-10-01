"""Aggregate validated (group, amount, active) tuples in first-seen order."""
import math


def summarize(records):
    totals = {}
    for group, amount, active in records:
        if active:
            total = totals.get(group, 0.0) + amount
            if not math.isfinite(total):
                raise ValueError('total must be finite')
            totals[group] = total
    return [{'group': group, 'total': total} for group, total in totals.items()]
