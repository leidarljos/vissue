`hygiene` groups live claims by holder and shows each holder's newest claim
or note date, flagging holders with nothing newer than `stale_claim_days`.
`vissue release --holder NAME` drops that identity's claims in one step,
leaves state, and writes a why-note on each ticket; `--dry-run` prints the
same report without writing.
