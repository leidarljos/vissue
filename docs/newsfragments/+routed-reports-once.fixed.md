`vissue claims`, `stale`, `export`, `graph` and `roadmap` over a routed
tracker parse each tracker once instead of once per project. On a vault of
173 projects, `vissue claims --json` went from 29 s and a 382 MB peak to
0.6 s and 153 MB, with the same output.
