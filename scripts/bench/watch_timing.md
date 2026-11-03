# What the owner's watcher sees of one save

The basis for `COALESCE_MS` and `GEN_POLL_MS` in
`crates/vissue-serve/src/unix/watcher.rs`.

Every number below was taken on an Intel Xeon (family 6, model 207: Emerald
Rapids) at 2.4 GHz, 8 cores at 1 thread each, Linux 6.12 on ext4, rustc
1.97.1, `cargo build --release`, CPU only, vim 9.1, GNU Emacs 29.3,
git 2.43.0, at the commit that adds this file.
macOS was not measured; run the same command on a Mac to fill that in.

    python3 scripts/bench/synth_tracker.py /tmp/synth4k-173 --issues 4000 --projects 173 --lead ljos=3800
    cargo run --release -p vissue-serve --example watch_timing /tmp/synth4k-173 Issues

The example watches a copy of the fixture vault through `notify` as
`vissue serve` does, saves `atlas/issues.org` 40 times by each method, and
counts the events that are not reads, as the watcher does.
`vissue-note` is `ops::note`, the path every vissue write takes;
`git-checkout` switches between two commits that differ in the file.

## One save

Two runs, worst of each column across both:

| method         | events | span ms | largest gap ms |
|----------------|-------:|--------:|---------------:|
| in-place write |      2 |     0.0 |           0.02 |
| write + rename |      5 |     1.3 |           1.25 |
| `vissue-note`  |     15 |     5.0 |           4.77 |
| vim            |     17 |     8.2 |           3.56 |
| emacs          |      4 |     0.6 |           0.44 |
| git checkout   |      5 |     8.4 |           8.21 |

From an in-place write returning to its first event: 23 and 44 µs median,
3.9 ms at worst.

The coalescing window must be longer than the largest gap, or one save is
rebuilt twice: 8.2 ms here, against a window of 200 ms.

## One poll tick

Generation read, project listing and a stat per project, 1000 ticks over the
173-project tracker: 1.04 and 1.06 ms median, 3.4 ms at worst.
At one tick per 250 ms that is under 0.5% of one core while idle.

## The ceiling

A change reaches clients after the window and a rebuild; one only the poll
sees waits a poll interval more. `list` on the same tracker, which parses
every file once as a full rebuild does, takes about 30 ms
(`read_verbs.md`). So 200 + 250 + 30 = 480 ms, inside the one second a user
waits without losing their train of thought (Nielsen, *Usability
Engineering*, 1993, ch. 5).
