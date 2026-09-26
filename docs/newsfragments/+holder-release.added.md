`hygiene` groups live claims by holder and shows each holder's newest
`CLAIMED_AT` stamp, flagging holders with nothing newer than
`stale_claim_days`. Logbook notes do not count, because they do not name
who wrote them. `vissue release --holder NAME` drops that identity's
claims in one step, leaves state, and writes a why-note on each ticket;
`--dry-run` prints the same report without writing.
