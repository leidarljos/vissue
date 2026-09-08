`vissue consensus <id> --gate` adds an exit status a shell hook can act on, and
prints the report either way so a failing hook leaves the reason on screen.

On one issue it exits non-zero unless the group agreed and one choice leads: a
plurality, a tie, a split and an oscillation are all cases where acting on the
number would be acting on agreement that is not there. Over `--children` it exits
non-zero when any child settled split or carries no ballots, which are the two
rows a parent cannot decide on a child's behalf.
