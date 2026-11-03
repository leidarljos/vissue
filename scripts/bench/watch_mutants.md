# The watcher's change check against two broken comparisons

The watcher calls a project changed when its parsed issues, line spans included, differ from the catalog's.
The comparison is in `crates/vissue-serve/src/unix/catalog.rs`.
Two tests in `crates/vissue-serve/tests/protocol.rs` are there to fail if that comparison is swapped for a weaker one.
`an_edit_that_parses_the_same_is_not_broadcast` fails if the comparison is on bytes.
`an_edit_that_only_moves_lines_is_broadcast` fails if the comparison drops line spans.

Each broken comparison is a diff in `data/watch_mutants/`.
Next to each diff is the output of the three change-check tests with it applied.
The byte mutant keeps each file's bytes from the last load or check and calls a project unchanged only when they match.
The span mutant sets `line_start` and `line_end` to 0 on both sides before comparing.

| comparison         | diff            | output         | result                                                                         |
|--------------------|-----------------|----------------|--------------------------------------------------------------------------------|
| parsed issues      | none            | `none.txt`     | 3 passed                                                                       |
| file bytes         | `bytes.diff`    | `bytes.txt`    | `an_edit_that_parses_the_same_is_not_broadcast` fails: `["atlas", "beacon"]`   |
| issues, no spans   | `no-spans.diff` | `no-spans.txt` | `an_edit_that_only_moves_lines_is_broadcast` fails: no notice within 60 s      |

`a_touch_that_changes_no_bytes_is_not_broadcast` passes under both, since a touch changes neither bytes nor issues.
The outputs were taken on the machine named in `watch_timing.md`.
The diffs apply to `catalog.rs` as at the commit that adds this file.

```sh
tests="a_touch_that_changes_no_bytes_is_not_broadcast an_edit_that_parses_the_same_is_not_broadcast an_edit_that_only_moves_lines_is_broadcast"
cargo test -p vissue-serve --test protocol -- $tests
for m in bytes no-spans; do
  git apply scripts/bench/data/watch_mutants/$m.diff
  cargo test -p vissue-serve --test protocol -- $tests
  git checkout crates/vissue-serve/src/unix/catalog.rs
done
```
