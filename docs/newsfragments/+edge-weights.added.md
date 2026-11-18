Edges can carry an importance. A trailing `*N` on a `:BLOCKED_BY:`,
`:PARENT:`, `:DISCOVERED_FROM:`, `:PIVOTED_TO:` token, or on an Org link
target, is that edge's weight. Leave the marker off and the weight is 1, so
a tracker file that names no weights is unchanged.
`vissue update ID --block OTHER --weight N` sets a blocker weight, and
`--edge parent`, `discovered-from`, `pivoted-to`, or `link:ID` with
`--weight N` sets the others. Within one priority, `ready` now ranks by how
much weighted work an issue unblocks, so that order can change even when no
weight is written. The set of ready issues does not change. `related` counts
a declared edge in proportion to its weight. `show` prints a Weights line
when a weight is not 1, and JSON adds `edge_weights` only then.
