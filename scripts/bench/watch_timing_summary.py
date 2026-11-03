#!/usr/bin/env python3
"""Summarise runs of the watch_timing example and of the rebuild timing into the tables in watch_timing.md.

Each run file has raw lines in ns after any `#` header lines, which are printed as they are.
A watch_timing run has one `save` line per save and one `tick` line per poll tick, and a rebuild timing run one `rebuild` line per full rebuild.
This prints, for each run and then pooled over every save of every run, per method: the most events one save raised, and the median, interquartile range (IQR) and max of the span of a save and of the gaps inside one.
It then prints the same for the delay to an in-place write's first event, for the poll tick and for the full rebuild.

Quartiles are the type-7 estimator (Hyndman and Fan, 1996), the default of R and numpy: linear interpolation between the two samples around rank (n-1)p.
Times are kept as exact decimals of whole nanoseconds and rounded half up, so a value ending in 5 rounds the same way wherever it falls.

    python3 scripts/bench/watch_timing_summary.py RUN.txt...
"""

import sys
from decimal import ROUND_HALF_UP, Decimal

NS_PER_MS = Decimal(10**6)
NS_PER_US = Decimal(10**3)


def quantile(sorted_values, p):
    h = (len(sorted_values) - 1) * Decimal(p)
    lo = int(h)
    if lo + 1 >= len(sorted_values):
        return sorted_values[lo]
    return sorted_values[lo] + (h - lo) * (sorted_values[lo + 1] - sorted_values[lo])


def stats(values, unit, places):
    """Median, interquartile range (IQR) and max of `values` in ns, in `unit`, rounded half up."""
    if not values:
        return ["-", "-", "-"]
    v = sorted(Decimal(x) / unit for x in values)
    step = Decimal(1).scaleb(-places)
    out = [quantile(v, "0.5"), quantile(v, "0.75") - quantile(v, "0.25"), v[-1]]
    return [str(x.quantize(step, rounding=ROUND_HALF_UP)) for x in out]


def parse(path):
    header, saves, ticks, missing, rebuilds = [], {}, [], [], []
    with open(path) as f:
        for line in f:
            fields = line.split()
            if line.startswith("#"):
                header.append(line.rstrip("\n"))
            elif fields[:1] == ["save"]:
                method, events, span, first, gaps = fields[1], int(fields[3]), *fields[4:]
                s = saves.setdefault(method, {"events": [], "spans": [], "gaps": [], "firsts": []})
                s["events"].append(events)
                if span != "-":
                    s["spans"].append(int(span))
                if first != "-":
                    s["firsts"].append(int(first))
                if gaps != "-":
                    s["gaps"].extend(int(g) for g in gaps.split(","))
            elif fields[:1] == ["tick"]:
                ticks.append(int(fields[1]))
            elif fields[:1] == ["missing"]:
                missing.append(fields[1])
            elif fields[:1] == ["rebuild"]:
                rebuilds.append(int(fields[1]))
    return header, saves, ticks, missing, rebuilds


def merge(runs):
    pooled = {}
    for _, saves, *_ in runs:
        for method, s in saves.items():
            p = pooled.setdefault(method, {"events": [], "spans": [], "gaps": [], "firsts": []})
            for key in p:
                p[key].extend(s[key])
    return pooled


def table(saves, ticks, missing, rebuilds):
    if saves or missing:
        save_table(saves, missing)
    firsts = [x for s in saves.values() for x in s["firsts"]]
    if firsts:
        print("first event after an in-place write, us: median, IQR, max:", ", ".join(stats(firsts, NS_PER_US, 1)))
    if ticks:
        print(f"poll tick over {len(ticks)} ticks, ms: median, IQR, max:", ", ".join(stats(ticks, NS_PER_MS, 3)))
    if rebuilds:
        print(f"full rebuild over {len(rebuilds)} rebuilds, ms: median, IQR, max:", ", ".join(stats(rebuilds, NS_PER_MS, 3)))


def save_table(saves, missing):
    print("| method | saves | events | span median | span IQR | span max | gap median | gap IQR | gap max |")
    print("|---|--:|--:|--:|--:|--:|--:|--:|--:|")
    by_gap = sorted(saves, key=lambda m: -max(saves[m]["gaps"], default=0))
    for method in by_gap:
        s = saves[method]
        span = stats(s["spans"], NS_PER_MS, 2)
        gap = stats(s["gaps"], NS_PER_MS, 2)
        print(f"| {method} | {len(s['events'])} | {max(s['events'])} | " + " | ".join(span + gap) + " |")
    for method in sorted(set(missing)):
        print(f"| {method} | not installed |")


def main(paths):
    runs = [parse(p) for p in paths]
    for path, (header, saves, ticks, missing, rebuilds) in zip(paths, runs):
        print(f"## {path}")
        print("\n".join(header))
        table(saves, ticks, missing, rebuilds)
        print()
    print(f"## pooled over {len(runs)} runs")
    table(
        merge(runs),
        [t for _, _, ticks, _, _ in runs for t in ticks],
        [m for _, _, _, missing, _ in runs for m in missing],
        [r for *_, rebuilds in runs for r in rebuilds],
    )


if __name__ == "__main__":
    main(sys.argv[1:])
