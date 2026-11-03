# What the owner's watcher sees of one save

The basis for `COALESCE_MS` and `GEN_POLL_MS` in `crates/vissue-serve/src/unix/watcher.rs`.

Every Linux number below was taken on an Intel Xeon (family 6, model 207: Emerald Rapids) at 2.4 GHz.
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
The longest gap of each save that raised two or more events, 1226 saves over every method, had a median of 0.23 ms, a 95th percentile (p95) of 2.08 ms and a max of 3.75 ms.
In-place writes raised one event in 214 of their 240 saves, and a save with one event has no gap.
From an in-place write returning to its first event took 37.8 µs median, with an IQR of 61.0 µs and 1104.6 µs at worst.

## One poll tick

A tick reads the generation, lists the projects and stats each project's file.
Over 1000 ticks per run (`TICKS` in the example) on the 173-project tracker, 6000 in all, it took 1.149 ms median, with an IQR of 0.049 ms, a p95 of 1.571 ms and 8.356 ms at worst.

## One rebuild

```sh
bash scripts/bench/rebuild_timing.sh OUT
python3 scripts/bench/watch_timing_summary.py OUT/run-*.txt
```

`rebuild_timing.sh` makes the same 173-project tracker and writes six runs, each with a header naming the machine, as `OUT/run-N.txt`.
Each run is the ignored test `full_rebuild_timing` in release, which does 100 full rebuilds (`REBUILDS`).
A rebuild parses every project, compares the result with the catalog and takes it, as the watcher's rebuild does when a project changed.
The six Linux runs are in `scripts/bench/data/rebuild_timing/linux/`, recorded at commit 240ce87.
Over 600 rebuilds, one took 23.417 ms median, with an IQR of 1.254 ms, a p95 of 28.069 ms and 41.573 ms at worst.

## macOS

On macOS `notify` 8.2.1 creates its FSEvents stream with a latency of 0 s and `kFSEventStreamCreateFlagNoDefer`.
Apple documents the flag ([kFSEventStreamCreateFlagNoDefer](https://developer.apple.com/documentation/coreservices/kfseventstreamcreateflagnodefer)).
With that flag, FSEvents sends an event at once when it follows more than the latency of quiet.
So FSEvents adds no documented delay of its own.
But Apple states no bound on how far apart it sends the events of one save, so this section measures it.

Every measured number in this section comes from the six runs in `scripts/bench/data/watch_timing/macos/` and the six in `scripts/bench/data/rebuild_timing/macos/`.
The macOS job of [Test run 41598252561](https://github.com/leidarljos/vissue/actions/runs/41598252561) recorded the watch runs at commit e4a7f74.
The macOS job of [Test run 41628529006](https://github.com/leidarljos/vissue/actions/runs/41628529006) recorded the rebuild runs at commit ee289fd.
That runner is an Apple M1 (Virtual) with 3 cores, running macOS 26.6.2 on arm64 with APFS.
It used rustc 1.97.1 and `cargo build --release`, on CPU only.
The editors were vim 9.1 and Emacs 31.1, with git 2.56.0.
Each file keeps the header its run printed.
On every push to main, that job records six release runs with `scripts/bench/watch_timing.sh` and six with `scripts/bench/rebuild_timing.sh`, each with its header, against the same 173-project tracker as on Linux.
It prints each run's table and the pooled one, and keeps the raw runs as the `watch-timing-macos` and `rebuild-timing-macos` artifacts.
The runs here are those artifacts from the two runs above.
This prints the same tables from them:

```sh
python3 scripts/bench/watch_timing_summary.py scripts/bench/data/watch_timing/macos/run-*.txt
python3 scripts/bench/watch_timing_summary.py scripts/bench/data/rebuild_timing/macos/run-*.txt
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
Every save raised two or more events, and the longest gap of each of the 1440 had a median of 0.01 ms, a p95 of 0.03 ms and a max of 109.69 ms.
From an in-place write returning to its first event took 14345.6 µs median, with an IQR of 5504.9 µs and 19205.4 µs at worst.
That delay comes before the first event, so it adds to the time a change takes to reach clients, not to the gaps the window has to cover.
Over 6000 ticks, 1000 per run (`TICKS`), a poll tick took 7.533 ms median, with an IQR of 2.540 ms, a p95 of 13.875 ms and 50.190 ms at worst.
Over 600 rebuilds, 100 per run (`REBUILDS`), a full rebuild took 47.271 ms median, with an IQR of 7.892 ms, a p95 of 64.091 ms and 132.728 ms at worst.

## The ceiling

A change reaches clients one window after its last event, plus a rebuild.
A change only the poll sees can land just after a tick reads it, so it first waits a poll interval and the next tick.
One second is how long a user waits without losing their train of thought (Nielsen, *Usability Engineering*, 1993, ch. 5).
With `P` the poll interval, `W` the window, `T` the slowest tick and `R` the slowest rebuild, the poll path has to meet `P + T + W + R <= 1000 ms`.
A change the watcher sees waits its first event and the span of its save in place of `P + T`.
The largest of those on macOS, 19.21 ms to a first event and a 128.20 ms span, add to 147.41 ms, under `P + T` = 300.19 ms there.
The first event is timed only for in-place writes, so that check covers them and the poll path is the one the budgets below meet.

## The budgets

The window has to outlast the longest gap of one save, `W > G`, or that save is rebuilt twice.
`G` is the max and not the p95, since one save over the window is a double rebuild.
The poll has to outlast the slowest tick, `P > T`, or the poll skips ticks (`MissedTickBehavior::Skip`) and a change only the poll sees waits longer than `P + T`.
Both have to leave the ceiling above.
So:

- `G < W <= 1000 - P - T - R`
- `T < P <= 1000 - W - T - R`

The inputs, in ms, pooled over the six runs per platform:

| input                        | Linux median | Linux p95 | Linux max | macOS median | macOS p95 | macOS max |
|------------------------------|-------------:|----------:|----------:|-------------:|----------:|----------:|
| longest gap of one save, `G` |         0.23 |      2.08 |      3.75 |         0.01 |      0.03 |    109.69 |
| poll tick, `T`               |        1.149 |     1.571 |     8.356 |        7.533 |    13.875 |    50.190 |
| full rebuild, `R`            |       23.417 |    28.069 |    41.573 |       47.271 |    64.091 |   132.728 |

With `P` = 250 ms, the window is bounded to 3.75 < `W` <= 700.071 ms on Linux and 109.69 < `W` <= 567.082 ms on macOS.
With `W` = 200 ms, the poll is bounded to 8.356 < `P` <= 750.071 ms on Linux and 50.190 < `P` <= 617.082 ms on macOS.

The data gives each constant a range, not a value.
The window's floor is one save: no gap in any macOS run but run 1 was above 18.73 ms, so six runs do not bound the tail, and a multiple of the max would be a factor picked, not measured.
Inside its range, the poll trades its idle cost against the wait of a change only the poll sees.
At 250 ms the median tick costs 0.46% of one core on Linux and 3.01% on macOS, and the p95 tick 0.63% and 5.55%.
No standard sets what an idle owner may spend, so the cost does not fix `P` either.
So `COALESCE_MS` = 200 and `GEN_POLL_MS` = 250 are measured budgets, not derived values.
Each is a choice inside its range, checked against the runs in `scripts/bench/data/watch_timing/` and `scripts/bench/data/rebuild_timing/`.

Their margins:

- The window is 200 / 3.75 = 53.33 times `G` on Linux and 200 / 109.69 = 1.82 times on macOS.
  That macOS margin is under 2, so a save with a gap twice as long as the largest here would be rebuilt twice.
- The poll is 250 / 8.356 = 29.92 times `T` on Linux and 250 / 50.190 = 4.98 times on macOS.
- `P + T + W + R` is 250 + 8.356 + 200 + 41.573 = 499.929 ms on Linux and 250 + 50.190 + 200 + 132.728 = 632.918 ms on macOS.
  That is 500.071 and 367.082 ms under the ceiling.
