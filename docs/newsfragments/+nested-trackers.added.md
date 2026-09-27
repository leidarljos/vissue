A tracker one level down, `prefix/A/B/issues.org`, is listed as the project
`A/B` when its header carries `#+VISSUE:`, so `show`, `check` and the listings
resolve its `A/B-*` ids. An unstamped org file there stays out. A file
reached through a symlink at the top is listed once, under its top name.
