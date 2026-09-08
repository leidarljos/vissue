`backlinks` takes a deed accession as well as an issue id, and answers with the
issues standing on that product: `(cites)` for a `:DEEDS:` citation, and
`(body mention)` for prose naming it with nothing declared. The join across the
tracker and the deed store ran in one direction, so the question you have when a
product turns out to be wrong had no command.

The corpus decides which namespace the argument belongs to. A known issue id
stays an issue whatever it looks like, so a project literally named `deed` keeps
working. An accession nobody cited answers empty rather than failing, since the
product may be real and simply unused. Under routing, the command line and the
tool scan every tracker in reach: a product has no project of its own.
