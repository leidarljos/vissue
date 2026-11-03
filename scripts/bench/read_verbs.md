# list, ready, search and show on a 4000-issue tracker

Every number below was taken on an Intel Xeon (family 6, model 207: Emerald
Rapids) at 2.4 GHz, 8 cores at 1 thread each, Linux 6.12, rustc 1.97.1,
`cargo build --release` (fat LTO), CPU only.
"Before" is commit `4b78738`; "after" is the commit that adds this file.

## Trackers

    python3 scripts/bench/synth_tracker.py /tmp/synth4k     --issues 4000 --projects 10  --lead ljos=3800
    python3 scripts/bench/synth_tracker.py /tmp/synth4k-173 --issues 4000 --projects 173 --lead ljos=3800

Both hold 3800 issues in `ljos` (a 1.96 MB file) and spread the other 200 over
the remaining projects, 2.1 MB of Org in all.
The 173 is the project count of the large vault recorded in `catalog.rs`
(`with_shared_corpus`); 10 is the same issues with few projects.
Every verb ran with `VISSUE_NO_ROUTE=1`, and `show` asked for `ljos-01gr`, the
middle issue of `ljos`.

## Where the time went

- Every verb parses the corpus once: `load_recs` takes 11 to 14 ms of the
  17 ms `list` takes on 10 projects, most of it the one large file.
- `show` parses its own project three times: the router's lookup, the lookup
  in `report::show`, and the file again for its preamble; about 8 ms each.
- `ready` with no project asks once per project over the whole corpus, and
  each ask rebuilt, from every issue, the set of open ids and the index of
  parents and children.
  On 173 projects that is 173 indexes of 4000 issues: 67 of the 97 ms.

The last was the dominant cost, and the cut: `ready` now looks up only the ids
its candidates name, the blockers they cite and the parents they sit under,
in one pass over the corpus and a second only when such a parent is
`ORDERED`, and in none when no candidate names any.

## Results

Wall time, median of 40 runs after 5 warm-up runs
(`hyperfine -N -w 5 -r 40`), in ms; run to run it moves by 1 to 2 ms.
Instructions are counted by `valgrind --tool=callgrind` with
`RAYON_NUM_THREADS=1`, in millions; they move by less than 0.01% between runs.

10 projects:

| verb                    | before ms | after ms | before Minstr | after Minstr |
|-------------------------|----------:|---------:|--------------:|-------------:|
| `list`                  |      17.2 |     17.0 |         151.2 |        151.1 |
| `list --json`           |      20.1 |     21.8 |         177.6 |        177.6 |
| `ready`                 |      24.5 |     18.9 |         180.8 |        162.9 |
| `ready --json`          |      20.5 |     20.0 |         183.1 |        165.3 |
| `search lease`          |      22.5 |     23.0 |         195.9 |        196.2 |
| `show ljos-01gr`        |      26.1 |     27.5 |         281.7 |        281.7 |
| `show --json ljos-01gr` |      27.0 |     25.9 |         228.3 |        228.3 |

173 projects:

| verb                    | before ms | after ms | before Minstr | after Minstr |
|-------------------------|----------:|---------:|--------------:|-------------:|
| `list`                  |      30.5 |     29.2 |         362.4 |        360.2 |
| `list --json`           |      22.4 |     22.2 |         201.7 |        199.6 |
| `ready`                 |      97.5 |     31.9 |        1132.1 |        368.7 |
| `ready --json`          |      87.3 |     20.4 |         947.7 |        184.4 |
| `search lease`          |      22.4 |     23.4 |         208.9 |        209.2 |
| `show ljos-01gr`        |      27.7 |     28.3 |         286.3 |        286.3 |
| `show --json ljos-01gr` |      29.3 |     31.9 |         244.1 |        244.1 |

`scripts/bench/same_output.sh BEFORE AFTER TRACKER` diffs what the two builds
print, and the exit code, for `list` and `ready` over all projects and each
one, as text and JSON, `count`, `search`, and `show` of every 97th id as text,
JSON and Org: 175 invocations on 10 projects and 827 on 173, all identical.
`crates/vissue-core/tests/synth_vault.rs` holds `list` and `ready` on a small
tracker from the same generator to what they printed before.

## Left

- On 173 projects, `list` and `ready` now spend 54% of their instructions in
  `report::format_issue_rows`, which hashes every id in the corpus into a map
  once per project.
- `show` would take about 16 ms less parsing its project once.
- `count --ready` does not apply the `ORDERED` rule `ready` applies: on the
  small tracker it counts 63 where `ready` prints 60 rows.
