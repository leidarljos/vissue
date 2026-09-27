`vissue merge-driver` merges three versions of one `issues.org` by heading, for
git's merge driver. Logbooks take the union of both sides, ballots merge per
voter, `DEEDS`, `BLOCKED_BY`, `FILES` and `VISSUE_TAGS` merge per token, and a
state that both sides moved takes the later logged move. A field both sides
changed some other way keeps ours and is written on the heading as a
merge-conflict note, with theirs kept in a `MERGE_CONFLICT` drawer. A side
that does not parse leaves git's ordinary text conflict. Register it in a
tracker repository with `vissue merge-driver --install`, then commit the
`.gitattributes` line it writes; each clone runs `--install` once.
