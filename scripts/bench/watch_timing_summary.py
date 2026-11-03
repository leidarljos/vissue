#!/usr/bin/env python3
"""Summarise runs of the watch_timing example into the table in watch_timing.md.

Each run prints p25, p50, p75 and max per column. For each column this prints
the median over runs of p50, the median over runs of p75 less the median over runs of p25, and the largest max of any run.
That spread is not an interquartile range: neither median is a quartile of one distribution.
The max is kept whole, since the window has to outlast the worst gap rather than a typical one.
Numbers are read as decimals and rounded half up, so a 0.005 rounds the same way wherever it falls.

    python3 scripts/bench/watch_timing_summary.py scripts/bench/data/watch_timing/run-*.txt
"""

import statistics
import sys
from decimal import ROUND_HALF_UP, Decimal

# p25, p50, p75 and max: the four numbers the example prints per column.
COLUMN = 4


def parse(path):
    rows, tick = {}, None
    with open(path) as f:
        for line in f:
            fields = line.split()
            if line.startswith("poll tick"):
                tick = [Decimal(x) for x in fields[-COLUMN:]]
            elif len(fields) >= 2 + 2 * COLUMN and fields[1].isdigit():
                numbers = [Decimal(x) for x in fields[2:] if x != "-"]
                rows[fields[0]] = {
                    "events": int(fields[1]),
                    "span": numbers[:COLUMN],
                    "gap": numbers[COLUMN : 2 * COLUMN],
                    "first": numbers[2 * COLUMN :],
                }
    return rows, tick


def cents(x):
    return x.quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)


def summary(runs):
    p25, p50, p75, top = zip(*runs)
    spread = statistics.median(p75) - statistics.median(p25)
    return f"{cents(statistics.median(p50))} {cents(spread)} {cents(max(top))}"


def main(paths):
    parsed = [parse(p) for p in paths]
    print(f"{len(paths)} runs; median p50, median p75 - median p25, max, in ms")
    for method in parsed[0][0]:
        runs = [rows[method] for rows, _ in parsed if method in rows]
        events = max(r["events"] for r in runs)
        span = summary([r["span"] for r in runs])
        gap = summary([r["gap"] for r in runs])
        print(f"{method:<13} events {events:>3}  span {span:<22} gap {gap}")
        firsts = [r["first"] for r in runs if r["first"]]
        if firsts:
            print(f"{'':<13} first event, us: {summary(firsts)}")
    ticks = [tick for _, tick in parsed if tick]
    if ticks:
        print(f"poll tick     {summary(ticks)}")


if __name__ == "__main__":
    main(sys.argv[1:])
