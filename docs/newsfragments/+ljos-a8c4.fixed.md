`vissue merge-driver --install` now untracks a tracker's committed event log and lock files.
The files stay on disk.
A merge where both branches wrote no longer conflicts on `.vault-events.jsonl`.
