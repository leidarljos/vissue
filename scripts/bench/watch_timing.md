# What the owner's watcher sees of one save

The basis for `COALESCE_MS` and `GEN_POLL_MS` in `crates/vissue-serve/src/unix/watcher.rs`.

Every number below outside the macOS section was taken on an Intel Xeon (family 6, model 207: Emerald Rapids) at 2.4 GHz.
It has 8 cores at 1 thread each.
It ran Ubuntu 24.04.5 with Linux 6.12.94 on ext4, rustc 1.97.1 and `cargo build --release`, on CPU only.
The editors were vim 9.1 and Emacs 29.3, with git 2.43.0.
The six runs are in `scripts/bench/data/watch_timing/linux/`, recorded at commit 2593c2a, and each keeps the header its run printed.

```sh
bash scripts/bench/watch_timing.sh OUT
python3 scripts/bench/watch_timing_summary.py OUT/run-*.txt
```

`watch_timing.sh` makes the 173-project tracker of `read_verbs.md`, builds the example in release and writes six runs, each with a header naming the machine, as `OUT/run-N.txt`.
The summary prints each run's table and the pooled one.
With `OUT` as `scripts/bench/data/watch_timing/linux`, it prints the tables below.

The example watches a copy of the fixture vault through `notify` as `vissue serve` does, and counts the events that are not reads, as the watcher does.
It saves `atlas/issues.org` in 40 rounds, once by each method per round.
Each run starts its rounds one method further along the list, so each method's place in the round moves from run to run.
`vissue-note` is `ops::note`, the path every vissue write takes.
`git-checkout` switches between two commits that differ in the file, both made before the watch starts.
The example keeps no statistics of its own.
It prints one raw `save` line per save and one `tick` line per poll tick, and the summary computes every number from those.

## One save

The aim here is to find the longest gap between the events of one save.
The table pools the 40 saves per method of each of the six runs, 240 in all, in ms.
Its median and interquartile range (IQR) are of that one distribution.
The max is the largest of any save, since the window has to outlast the worst gap.

| method         | saves | events | span median | span IQR | span max | gap median | gap IQR | gap max |
|----------------|------:|-------:|------------:|---------:|---------:|-----------:|--------:|--------:|
| `vissue-note`  |   240 |     15 |        1.71 |     0.57 |     4.55 |       0.00 |    0.03 |    3.75 |
| vim            |   240 |     16 |        4.31 |     0.77 |     6.78 |       0.00 |    0.04 |    3.26 |
| git checkout   |   240 |      3 |        0.00 |     0.00 |     1.15 |       0.00 |    0.00 |    1.14 |
| emacs          |   240 |      4 |        0.27 |     0.10 |     0.53 |       0.06 |    0.22 |    0.44 |
| write + rename |   240 |      5 |        0.01 |     0.01 |     0.34 |       0.00 |    0.00 |    0.32 |
| in-place write |   240 |      2 |        0.00 |     0.00 |     0.03 |       0.01 |    0.01 |    0.03 |

Events is the most that one save raised.
The largest gap, 3.75 ms in a `vissue-note` save, is in run 4.
Run 4 spanned a pause of the virtual machine: run 5 started 34 min after it, against about 4.5 min between the other runs.
No gap in any other run was above 3.26 ms.
From an in-place write returning to its first event took 37.8 µs median, with an IQR of 61.0 µs and 1104.6 µs at worst.

## The window

The coalescing window must be longer than the largest gap, or one save is rebuilt twice.
On Linux the largest gap was 3.75 ms, so the window must be at least 4 ms there.
On macOS it was 109.69 ms (see below), so the window must be at least 110 ms there.
The ceiling below caps the window, the poll and a rebuild at 1000 ms together.
With a 250 ms poll and a 30 ms rebuild, that leaves at most 720 ms for the window.
200 ms is a choice inside that range of 110 to 720 ms; the measurements fix only the range.
It is 53.33 times the largest gap on Linux and 1.82 times the largest on macOS.

## macOS

On macOS `notify` 8.2.1 creates its FSEvents stream with a latency of 0 s and `kFSEventStreamCreateFlagNoDefer`.
Apple documents the flag ([kFSEventStreamCreateFlagNoDefer](https://developer.apple.com/documentation/coreservices/kfseventstreamcreateflagnodefer)).
With that flag, FSEvents sends an event at once when it follows more than the latency of quiet.
So FSEvents adds no documented delay of its own.
But Apple states no bound on how far apart it sends the events of one save, so this section measures it.

Every measured number in this section comes from the six runs in `scripts/bench/data/watch_timing/macos/`.
The macOS job of [Test run 41598252561](https://github.com/leidarljos/vissue/actions/runs/41598252561) recorded them at commit e4a7f74.
That runner is an Apple M1 (Virtual) with 3 cores, running macOS 26.6.2 on arm64 with APFS.
It used rustc 1.97.1 and `cargo build --release`, on CPU only.
The editors were vim 9.1 and Emacs 31.1, with git 2.56.0.
Each file keeps the header its run printed.
On every push to main, that job records six release runs with `scripts/bench/watch_timing.sh`, each with its header, against the same 173-project tracker as on Linux.
It prints each run's table and the pooled one, and keeps the raw runs as the `watch-timing-macos` artifact.
The runs here are that artifact from the run above.
This prints the same tables from them:

```sh
python3 scripts/bench/watch_timing_summary.py scripts/bench/data/watch_timing/macos/run-*.txt
```

The table pools the 40 saves per method of each of the six runs, 240 in all, in ms.
As on Linux, its median and interquartile range (IQR) are of that one distribution.

| method         | saves | events | span median | span IQR | span max | gap median | gap IQR | gap max |
|----------------|------:|-------:|------------:|---------:|---------:|-----------:|--------:|--------:|
| `vissue-note`  |   240 |     15 |        0.03 |     0.01 |   128.20 |       0.00 |    0.00 |  109.69 |
| vim            |   240 |     26 |        0.01 |     0.01 |    54.59 |       0.00 |    0.00 |   51.82 |
| emacs          |   240 |      7 |        0.01 |     0.01 |    16.83 |       0.00 |    0.00 |   16.82 |
| write + rename |   240 |      9 |        0.02 |     0.01 |    14.36 |       0.00 |    0.00 |   14.35 |
| git checkout   |   240 |      8 |        0.00 |     0.00 |    12.37 |       0.00 |    0.00 |   12.35 |
| in-place write |   240 |      3 |        0.02 |     0.00 |     0.04 |       0.01 |    0.01 |    0.04 |

The span median is at most 0.03 ms for every method, so FSEvents mostly sends the events of one save together.
The largest gap was 109.69 ms, in a `vissue-note` save of run 1.
Run 1 also had the largest vim gap, 51.82 ms.
No gap in any other run was above 18.73 ms.
From an in-place write returning to its first event took 14345.6 µs median, with an IQR of 5504.9 µs and 19205.4 µs at worst.
That delay comes before the first event, so it adds to the time a change takes to reach clients, not to the gaps the window has to cover.
Over 6000 ticks, 1000 per run (`TICKS`), a poll tick took 7.533 ms median, with an IQR of 2.540 ms and 50.190 ms at worst.

The window of 200 ms is 1.82 times the largest gap measured here.
That margin is under 2, so a save with a gap twice as long as the largest here would be rebuilt twice.

## One poll tick

A tick reads the generation, lists the projects and stats each project's file.
Over 1000 ticks per run (`TICKS` in the example) on the 173-project tracker, 6000 in all, it took 1.149 ms median, with an IQR of 0.049 ms and 8.356 ms at worst.
At one tick per 250 ms the median tick costs 0.46% of one core while idle.
If every tick were as slow as the worst, the poll would cost 3.34%.
The poll can be at most 1000 - 200 - 30 = 770 ms under the ceiling below, and 250 ms is a choice inside that.

## The ceiling

A change reaches clients after the window and a rebuild.
A change only the poll sees waits a poll interval more.
`list` parses every file once, as a full rebuild does.
On the same tracker it takes about 30 ms (`read_verbs.md`).
So 200 + 250 + 30 = 480 ms.
That is inside the one second a user waits without losing their train of thought (Nielsen, *Usability Engineering*, 1993, ch. 5).
