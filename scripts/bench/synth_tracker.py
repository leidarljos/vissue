#!/usr/bin/env python3
"""Write a synthetic tracker: N issues over P projects, with blockers and parents.

Deterministic, so two runs at the same size time the same files. The shape
follows a real tracker: most issues open, a tail of DONE, a few blockers per
issue drawn from earlier ids in the same project, and a share of issues under a
parent. Bodies are a paragraph, since parse cost is per line as well as per
heading.

    synth_tracker.py OUT_DIR --issues 10000 --projects 20
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--issues", type=int, default=10000)
    ap.add_argument("--projects", type=int, default=20)
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args()
    rng = random.Random(a.seed)
    root = Path(a.out)
    root.mkdir(parents=True, exist_ok=True)
    per = a.issues // a.projects
    (root / "vissue.toml").write_text('prefix = "Issues"\n')
    for pi in range(a.projects):
        p = f"proj{pi:02d}"
        d = root / "Issues" / p
        d.mkdir(parents=True, exist_ok=True)
        ids = []
        lines = [HEADER.format(p=p)]
        for k in range(per):
            iid = f"{p}-{b36(k, 4)}"
            state = rng.choices(["TODO", "STARTED", "DONE"], [6, 1, 3])[0]
            prio = rng.choice("ABC")
            kind = rng.choice(["bug", "feature", "task"])
            title = " ".join(rng.choice(WORDS) for _ in range(6))
            props = [f":ID:         {iid}", ":CREATED:    [2026-09-10 Thu]", f":TYPE:       {kind}"]
            if ids and rng.random() < 0.4:
                blockers = rng.sample(ids, k=min(len(ids), rng.randint(1, 3)))
                props.append(":BLOCKED_BY: " + " ".join(blockers))
            if ids and rng.random() < 0.25:
                props.append(f":PARENT:     {rng.choice(ids)}")
            body = " ".join(rng.choice(WORDS) for _ in range(40))
            lines.append(f"* {state} [#{prio}] {title}  :{kind}:\n:PROPERTIES:\n"
                         + "\n".join(props) + "\n:END:\n\n" + body + "\n\n")
            ids.append(iid)
        (d / "issues.org").write_text("".join(lines))
    print(f"{a.issues} issues over {a.projects} projects under {root}")


if __name__ == "__main__":
    main()
