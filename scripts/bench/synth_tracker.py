#!/usr/bin/env python3
"""Write a synthetic tracker: N issues over P projects, with blockers and parents.

Deterministic, so two runs at the same size time the same files. The shape
follows a real tracker: most issues open, a tail of DONE, a few blockers per
issue drawn from earlier ids, mostly in the same project, and a share of issues
under a parent. A parent marked ORDERED takes the next few issues as its
children, the way a plan is split into steps. Started and done issues carry the
logbook lines a claim and a close write, a started one holds a claim, and a
done one may name a deed. Bodies run a few lines, since parse cost is per line
as well as per heading.

The shares below are a shape, not a measurement of any one tracker. They are
set so that every rule `ready` applies has issues to act on: an open and a
closed blocker, one in another project, an ORDERED parent with open children
in sequence, and a NOBLOCKING child that steps out of that sequence.

    synth_tracker.py OUT_DIR --issues 10000 --projects 20

`--lead NAME=COUNT` puts COUNT of the issues in one project called NAME and
spreads the rest over the others, which is the shape of a tracker where one
project took most of the work:

    synth_tracker.py OUT_DIR --issues 4000 --projects 173 --lead ljos=3800
"""
import argparse
import random
from pathlib import Path

HEADER = """#+TITLE: {p} issues
#+VISSUE: 1
#+CATEGORY: {p}
#+TAGS: {{ bug(b) feature(f) task(t) chore(c) plan(p) }}
#+TAGS: docs(d) perf ignore ARCHIVE
#+EXCLUDE_TAGS: noexport
#+SELECT_TAGS: export
#+PRIORITIES: A C C
#+FILETAGS: :issues:{p}:noexport:

"""

WORDS = ("lease generation fence token holder quiet reclaim node graph deed "
         "accession manifest satchel proof head bridge passage window scorer "
         "ballot panel fuse voter tracker issue blocker parent plan claim").split()


def b36(n, width):
    digits = "0123456789abcdefghijklmnopqrstuvwxyz"
    out = ""
    for _ in range(width):
        out = digits[n % 36] + out
        n //= 36
    return out


def sentence(rng, n):
    return " ".join(rng.choice(WORDS) for _ in range(n))


def write_project(root, p, count, rng, earlier):
    """Write one project and return its ids. `earlier` holds every id written
    before it, which is where a cross-project blocker comes from."""
    d = root / "Issues" / p
    d.mkdir(parents=True, exist_ok=True)
    ids = []
    lines = [HEADER.format(p=p)]
    plan, plan_left = None, 0
    for k in range(count):
        iid = f"{p}-{b36(k, 4)}"
        state = rng.choices(["TODO", "STARTED", "BLOCKED", "DONE", "CANCELLED"], [6, 1, 1, 3, 1])[0]
        prio = rng.choice("ABC")
        kind = rng.choice(["bug", "feature", "task"])
        title = sentence(rng, 6)
        props = [f":ID:         {iid}", ":CREATED:    [2026-09-10 Thu]", f":TYPE:       {kind}"]
        pool = earlier if rng.random() < 0.1 else ids
        if pool and rng.random() < 0.4:
            blockers = rng.sample(pool, k=min(len(pool), rng.randint(1, 3)))
            props.append(":BLOCKED_BY: " + " ".join(blockers))
        if plan_left:
            parent, plan_left = plan, plan_left - 1
        elif ids and rng.random() < 0.25:
            parent = rng.choice(ids)
        else:
            parent = None
        if parent:
            props.append(f":PARENT:     {parent}")
            if rng.random() < 0.1:
                props.append(":NOBLOCKING: t")
        if rng.random() < 0.05:
            props.append(":ORDERED:    t")
            plan, plan_left = iid, rng.randint(2, 5)
        log = []
        if state == "STARTED":
            props.append(":CLAIMED_BY: seat-bench")
            props.append(":CLAIMED_AT: [2026-09-11 Fri 09:00]")
            log.append('- State "STARTED" from "TODO" [2026-09-11 Fri 09:00]')
        if state == "DONE":
            if rng.random() < 0.5:
                props.append(f":DEEDS:      deed-{b36(rng.randrange(36 ** 6), 6)}")
            log.append(f'- Note: "{sentence(rng, 8)}" [2026-09-12 Sat 17:31]')
            log.append('- State "DONE" from "STARTED" [2026-09-12 Sat 17:30]')
            log.append('- State "STARTED" from "TODO" [2026-09-11 Fri 09:00]')
        drawer = (":LOGBOOK:\n" + "\n".join(log) + "\n:END:\n") if log else ""
        body = "\n".join(sentence(rng, 10) for _ in range(rng.randint(2, 6)))
        lines.append(f"* {state} [#{prio}] {title}  :{kind}:\n:PROPERTIES:\n"
                     + "\n".join(props) + "\n:END:\n" + drawer + "\n" + body + "\n\n")
        ids.append(iid)
    (d / "issues.org").write_text("".join(lines).rstrip("\n") + "\n")
    return ids


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--issues", type=int, default=10000)
    ap.add_argument("--projects", type=int, default=20)
    ap.add_argument("--lead", metavar="NAME=COUNT", default=None)
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args()
    rng = random.Random(a.seed)
    root = Path(a.out)
    root.mkdir(parents=True, exist_ok=True)
    (root / "vissue.toml").write_text('prefix = "Issues"\n')
    earlier = []
    rest, others = a.issues, a.projects
    if a.lead:
        name, count = a.lead.split("=")
        earlier += write_project(root, name, int(count), rng, earlier)
        rest, others = a.issues - int(count), a.projects - 1
    for pi in range(others):
        count = rest // others + (pi < rest % others)
        earlier += write_project(root, f"proj{pi:03d}", count, rng, earlier)
    print(f"{len(earlier)} issues over {a.projects} projects under {root}")


if __name__ == "__main__":
    main()
