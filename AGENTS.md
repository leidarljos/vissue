# Working on vissue

Reads return the file as it is stored. A heading that looks like
credential material is still returned. `body-excerpt` and
`recall --excerpts` put a `[warn]` line with the text. `vissue_org`
puts that line at the start of the tool result, ahead of the heading.
`vissue://issue/{id}` keeps the heading as the org content and adds a
second content item with the same line. `show --org` and a packed
satchel write the line to stderr, so the bytes written to a file stay
the heading.

A control connection whose peer uid does not match the server is
served. stderr carries a `[warn]` line. The socket stays mode `0600`
and its directory mode `0700`.

Do not add a check that withholds a read or drops a connection in
order to keep something back. Unknown ids, blocker cycles, stale
generations, and a claim on a closed issue are still errors: those
writes would record a state the file cannot represent.
