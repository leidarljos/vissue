A gate with `--quorum` takes `--deadline RFC3339`: past it, a vote still short of its quorum has lapsed, the issue gets one `quorum timed out` note, and the gate exits 3 instead of 1.
