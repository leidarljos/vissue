# What the owner's watcher sees of one save

The basis for `COALESCE_MS` and `GEN_POLL_MS` in `crates/vissue-serve/src/unix/watcher.rs`.

Every number below was taken on an Intel Xeon (family 6, model 207: Emerald Rapids) at 2.4 GHz.
It has 8 cores at 1 thread each.
It ran Linux 6.12 on ext4, rustc 1.97.1 and `cargo build --release`, on CPU only.
The editors were vim 9.1 and Emacs 29.3, with git 2.43.0.
The example and the watcher were as at the commit that adds `data/watch_timing/`, which changes no code they run.

```sh
python3 scripts/bench/synth_tracker.py /tmp/synth4k-173 --issues 4000 --projects 173 --lead ljos=3800
cargo build --release -p vissue-serve --example watch_timing
for n in 1 2 3 4 5 6; do
  ./target/release/examples/watch_timing /tmp/synth4k-173 Issues > scripts/bench/data/watch_timing/run-$n.txt
done
python3 scripts/bench/watch_timing_summary.py scripts/bench/data/watch_timing/run-*.txt
```

The example watches a copy of the fixture vault through `notify` as `vissue serve` does.
It saves `atlas/issues.org` 40 times by each method and counts the events that are not reads, as the watcher does.
`vissue-note` is `ops::note`, the path every vissue write takes.
`git-checkout` switches between two commits that differ in the file.
Each run prints p25, p50, p75 and max per column.
The six raw outputs are in `data/watch_timing/`.

## One save

The aim here is to find the longest gap between the events of one save.
The table covers six runs of 40 saves per method, in ms.
For the median, each run gives its p50, and the table shows the median of the six.
The interquartile range (IQR) is the median p75 across runs less the median p25.
The max is the largest of any run, since the window has to outlast the worst gap.

| method         | events | span median | span IQR | span max | gap median | gap IQR | gap max |
|----------------|-------:|------------:|---------:|---------:|-----------:|--------:|--------:|
| git checkout   |      5 |        0.10 |     0.26 |    11.95 |       0.03 |    0.08 |   11.94 |
| vim            |     17 |        3.90 |     1.02 |     9.05 |       0.00 |    0.04 |    4.64 |
| `vissue-note`  |     15 |        1.58 |     0.66 |     2.90 |       0.00 |    0.02 |    2.40 |
| write + rename |      5 |        0.03 |     0.09 |     0.97 |       0.00 |    0.01 |    0.97 |
| emacs          |      4 |        0.29 |     0.11 |     0.92 |       0.08 |    0.22 |    0.67 |
| in-place write |      2 |        0.00 |     0.00 |     0.02 |       0.01 |    0.00 |    0.02 |

Events is the most that one save raised.
The `git checkout` maximum is over 100 times its p75.
A few of its 240 saves have one long gap, and the window has to cover those.
From an in-place write returning to its first event took 32.5 µs median, with an IQR of 53.5 µs and 580 µs at worst.

## The window

The coalescing window must be longer than the largest gap, or one save is rebuilt twice.
The largest gap was 11.94 ms, so the window must be at least 12 ms.
The ceiling below caps the window, the poll and a rebuild at 1000 ms together.
With a 250 ms poll and a 30 ms rebuild, that leaves at most 720 ms for the window.
200 ms is a choice inside that range of 12 to 720 ms; the measurements fix only the range.
At 200 ms one save is still one rebuild while its longest gap stays under 200 ms, 16 times the longest measured here.

## macOS

macOS was not measured.
On macOS `notify` 8.2.1 creates its FSEvents stream with a latency of 0 s and `kFSEventStreamCreateFlagNoDefer`.
Apple documents the flag ([kFSEventStreamCreateFlagNoDefer](https://developer.apple.com/documentation/coreservices/kfseventstreamcreateflagnodefer)).
With that flag, FSEvents sends an event at once when it follows more than the latency of quiet.
So FSEvents adds no documented delay of its own.
But Apple states no bound on how far apart it sends the events of one save.
The macOS margin is therefore unverified.
The macOS CI job runs the example after its tests and prints the same table.
A run there can fill this in.

## One poll tick

A tick reads the generation, lists the projects and stats each project's file.
Over 1000 ticks per run on the 173-project tracker it took 1.05 ms median, with an IQR of 0.02 ms and 7.60 ms at worst.
At one tick per 250 ms the median tick costs 0.42% of one core while idle.
If every tick were as slow as the worst, the poll would cost 3.0%.
The poll can be at most 1000 - 200 - 30 = 770 ms under the ceiling below, and 250 ms is a choice inside that.

## The ceiling

A change reaches clients after the window and a rebuild.
A change only the poll sees waits a poll interval more.
`list` parses every file once, as a full rebuild does.
On the same tracker it takes about 30 ms (`read_verbs.md`).
So 200 + 250 + 30 = 480 ms.
That is inside the one second a user waits without losing their train of thought (Nielsen, *Usability Engineering*, 1993, ch. 5).
