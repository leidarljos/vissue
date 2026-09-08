@0xa1e7c747dbf99b93;

# The operation set, and the one place it is written down.
#
# Every verb this tracker offers reaches a caller through three surfaces: the
# command line, the control socket, and the MCP tool list. Each was declared in
# its own file in its own idiom, so a verb could exist on one and not the others,
# and repeatedly did: `vote` shipped on the command line alone, `append` had no
# socket method for as long as the socket existed, and a test asserting
# `issue/fold` was an unknown method became wrong the day fold got one.
#
# Nothing detected any of that. The reference-completeness tests covered the docs
# and not the surfaces, so the docs stayed honest about a surface set that was not.
#
# So the set lives here and the surfaces are checked against it. A verb added to
# this file that no socket method serves is a failing test naming the gap, and a
# verb added to a surface but not here is the same. Written in Cap'n Proto because
# a schema is the artefact a non-Rust client can read too; the wire formats are
# unchanged, and this file is not compiled into the build.
#
# The names are the contract. `cli` is the subcommand, `socket` the JSON-RPC
# method, `mcp` the tool. An empty `mcp` means the verb is deliberately absent
# from the tool list, and the reason belongs in `note`.

struct Operation {
  # One verb, named on each surface that carries it.

  cli @0 :Text;
  # Subcommand name, as clap spells it. Empty for an operation the command line
  # reaches through a flag on another verb rather than a verb of its own: the tool
  # `vissue_org` is `show --org`, and a tool cannot take a flag. The reason goes in
  # `note`, as with any empty surface.

  socket @1 :Text;
  # Control-socket method, or empty when the verb has none.

  mcp @2 :Text;
  # MCP tool name, or empty when the verb is deliberately not a tool.

  mutates @3 :Bool;
  # Whether the verb changes a file. A mutating verb must reach every surface,
  # because a caller that has to leave the socket for one write puts a hole in the
  # change stream exactly where that write was. A reading verb is held to the
  # surfaces it claims and not to all of them: several are absent from the socket
  # today, and the schema records that rather than pretending otherwise.

  aliases @7 :List(Text);
  # Other names the command line answers to for this verb: one verb under two
  # spellings, taking the same flags either way. A subcommand that merely does a
  # similar job with fewer flags is not this -- it is its own row with
  # `shorthandFor` set -- because a row of ten fields would then answer for a verb
  # that accepts three.

  shorthandFor @8 :Text;
  # The operation this verb is a narrower spelling of, empty for the ordinary case.
  # `q` is a shorthand for `create`: it takes three of create's fields and prints
  # only the id. A shorthand needs no surface of its own, because everything it can
  # do the operation it names can do, and that is checked rather than asserted --
  # the named operation has to exist, has to reach the socket if this one mutates,
  # and has to take every field this one takes. A mutating verb that reaches no
  # surface and names nothing is a hole in the change stream, which is the thing
  # this field must not become an excuse for.

  local @6 :Bool;
  # True for a verb that only makes sense in the process it is typed into: the
  # terminal UI, the HUD, shell completions, the man page, the server's own
  # lifecycle. These have no remote surface and are not gaps. Recorded so that
  # every subcommand appears here, because a verb the schema does not mention is a
  # verb no check can see, which is how a new one would arrive unnoticed.

  note @4 :Text;
  # Why a surface is empty, when one is. Empty otherwise.

  fields @5 :List(Field);
  # The fields the verb takes, each named on the surfaces that carry it.
  #
  # Naming a verb per surface stops the verb going missing. It says nothing about
  # the surfaces disagreeing on what to call a field, which is the next way this
  # drifts, and it had already happened: `vote` takes `--for` on the command line
  # and `choice` as a tool argument, and MCP `create` could set neither a deadline
  # nor a scheduled date that the subcommand has always taken.
  #
  # Positional arguments are not in here. `create` and `reject` take their title
  # positionally, and the first version of this list said `title` for both because
  # it was written from memory rather than from the parser.
}

struct Field {
  # One field of one verb, named per surface.
  #
  # Two names rather than one, because some divergence is forced rather than
  # sloppy: `--type` cannot be a Rust field called `type`, and `--for` cannot be a
  # field called `for`, since both are keywords. Recording the pair is honest about
  # that, where a single canonical name would either lie or forbid the flag.

  cli @0 :Text;
  # Flag name without the leading dashes, empty when the surface does not take it
  # or takes it positionally.

  tool @1 :Text;
  # MCP argument name, empty when the tool deliberately does not take it.

  socket @2 :Text;
  # Control-socket parameter name, empty when the method does not take it.

  note @3 :Text;
  # Why a surface is empty, or why the names differ. Empty when they agree.

  toolType @4 :Text;
  # The Rust type of the tool argument, empty when the tool does not take it.

  socketType @5 :Text;
  # The Rust type of the socket parameter, empty when the method does not take it.
  #
  # Recorded per surface rather than once, because two of them genuinely differ and
  # forcing one name over both would be a lie. `priority` is `Option<String>` as a
  # tool argument and `Option<char>` on the socket; `force` is `Option<bool>` and
  # `bool`. What this catches is a type changing on one side, which is the failure a
  # name check cannot see: a field going from a number to a string keeps its name.

  omittable @6 :Bool;
  # Whether a caller may leave this out on the wire, whatever the Rust type says.
  # `Option<T>` says it already. A plain `bool` behind serde's `default` says the
  # same thing and the type does not show it, so `force` and `dry_run` read as
  # required while every caller omits them. Recorded because what a caller must send
  # is the contract, and the Rust spelling is only evidence about it.
}


const globalFlags :List(Text) = [
  # Flags clap puts on every subcommand. Named once here rather than repeated on
  # forty rows, so the reverse check can subtract them and a per-verb row stays
  # about what the verb actually takes.
  "root", "prefix", "no-route", "help", "version"
];

const operations :List(Operation) = [
  ( cli = "create", socket = "issue/create", mcp = "vissue_create", mutates = true, local = false,
    aliases = [],
    note = "",
    fields = [
      ( cli = "", tool = "title", socket = "title", note = "no flag: the command line takes it as a positional argument, which is why the schema recorded every flag of this verb and not the parameter it is about", toolType = "String", socketType = "String" ),
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "String", socketType = "String" ),
      ( cli = "priority", tool = "priority", socket = "priority", note = "", toolType = "Option<String>", socketType = "Option<char>" ),
      ( cli = "type", tool = "issue_type", socket = "issue_type", note = "the flag cannot be a Rust field of that name, which is a keyword", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "tags", tool = "tags", socket = "tags", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "parent", tool = "parent", socket = "parent", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "body", tool = "body", socket = "body", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "deadline", tool = "deadline", socket = "deadline", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "scheduled", tool = "scheduled", socket = "scheduled", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "body-file", tool = "", socket = "", note = "no tool argument: a path resolves on the host running the server, not the caller's", toolType = "", socketType = "" ),
      ( cli = "", tool = "", socket = "agent", note = "socket only: it overrides the identity the connection was opened with, which the other surfaces take from the environment", toolType = "", socketType = "Option<String>" ),
      ( cli = "quiet", tool = "", socket = "", note = "command line only: it trims the output to the id, and a caller reading a field does not need it", toolType = "", socketType = "" )
    ] ),
  ( cli = "q", socket = "", mcp = "", mutates = true, local = false,
    aliases = [], shorthandFor = "create",
    note = "quick capture: a subcommand of its own rather than an alias of create, because it takes three of create's fields and prints only the id. No socket method or tool of its own, because create reaches both and takes every field this takes.",
    fields = [
      ( cli = "project", tool = "", socket = "", note = "", toolType = "", socketType = "" ),
      ( cli = "type", tool = "", socket = "", note = "the flag cannot be a Rust field of that name, which is a keyword", toolType = "", socketType = "" ),
      ( cli = "parent", tool = "", socket = "", note = "", toolType = "", socketType = "" )
    ] ),
  ( cli = "update", socket = "issue/update", mcp = "vissue_update", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "state", tool = "state", socket = "state", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "priority", tool = "priority", socket = "priority", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "block", tool = "block", socket = "block", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "unblock", tool = "unblock", socket = "unblock", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "if-state", tool = "if_state", socket = "if_state", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "if-gen", tool = "if_gen", socket = "if_gen", note = "", toolType = "Option<u64>", socketType = "Option<u64>" ),
      ( cli = "", tool = "", socket = "agent", note = "socket only: it overrides the identity the connection was opened with, which the other surfaces take from the environment", toolType = "", socketType = "Option<String>" )
    ] ),
  ( cli = "claim", socket = "issue/claim", mcp = "vissue_claim", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "force", tool = "force", socket = "force", note = "", toolType = "Option<bool>", socketType = "bool" , omittable = true ),
      ( cli = "", tool = "", socket = "agent", note = "socket only: it overrides the identity the connection was opened with, which the other surfaces take from the environment", toolType = "", socketType = "Option<String>" )
    ] ),
  ( cli = "note", socket = "issue/note", mcp = "vissue_note", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "", tool = "text", socket = "text", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" )
    ] ),
  ( cli = "append", socket = "issue/append", mcp = "vissue_append", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "text", tool = "text", socket = "text", note = "", toolType = "String", socketType = "String" ),
      ( cli = "file", tool = "", socket = "", note = "no tool argument: a path resolves on the host running the server, not the caller's", toolType = "", socketType = "" ),
      ( cli = "", tool = "", socket = "agent", note = "socket only: it overrides the identity the connection was opened with, which the other surfaces take from the environment", toolType = "", socketType = "Option<String>" )
    ] ),
  ( cli = "refile", socket = "issue/refile", mcp = "vissue_refile", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "to", tool = "to", socket = "to", note = "", toolType = "String", socketType = "String" )
    ] ),
  ( cli = "reject", socket = "issue/reject", mcp = "vissue_reject", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "to", tool = "to", socket = "to", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "reason", tool = "reason", socket = "reason", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "resolve", socket = "issue/resolve", mcp = "vissue_resolve", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "", tool = "state", socket = "state", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "state", tool = "state", socket = "state", note = "", toolType = "String", socketType = "String" )
    ] ),
  ( cli = "vote", socket = "issue/vote", mcp = "vissue_vote", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "for", tool = "choice", socket = "choice", note = "the flag cannot be a Rust field of that name, which is a keyword", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "", tool = "", socket = "agent", note = "socket only: it overrides the identity the connection was opened with, which the other surfaces take from the environment", toolType = "", socketType = "Option<String>" )
    ] ),
  ( cli = "deed", socket = "issue/deed", mcp = "vissue_deed", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it as they spell every other id", toolType = "String", socketType = "String" ),
      ( cli = "add", tool = "add", socket = "add", note = "repeatable on the command line; a list on the two remote surfaces, and absent on both lists is the read", toolType = "Option<Vec<String>>", socketType = "Vec<String>", omittable = true ),
      ( cli = "remove", tool = "remove", socket = "remove", note = "repeatable on the command line; a list on the two remote surfaces, and absent on both lists is the read", toolType = "Option<Vec<String>>", socketType = "Vec<String>", omittable = true )
    ] ),
  ( cli = "recall", socket = "issue/recall", mcp = "vissue_recall", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "no flag: the command line takes it as a positional argument", toolType = "String", socketType = "String" ),
      ( cli = "depth", tool = "depth", socket = "depth", note = "", toolType = "Option<usize>", socketType = "Option<usize>" ),
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "deeds-only", tool = "", socket = "", note = "a shell-substitutable list of accessions, which the remote surfaces already carry as a field of the structure they answer with", toolType = "", socketType = "" ),
      ( cli = "excerpts", tool = "excerpts", socket = "excerpts", note = "splice in what each input concluded, which lives in its body rather than in the deed it named", toolType = "Option<bool>", socketType = "bool", omittable = true )
    ] ),
  ( cli = "consensus", socket = "issue/consensus", mcp = "vissue_consensus", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "no flag: the command line takes it as a positional argument", toolType = "String", socketType = "String" ),
      ( cli = "children", tool = "children", socket = "children", note = "roll up over the issue's children instead of its own ballots", toolType = "Option<bool>", socketType = "bool", omittable = true ),
      ( cli = "json", tool = "", socket = "", note = "the socket answers in structure already and the tool answers the prose a reader needs, so neither takes a flag to ask", toolType = "", socketType = "" )
    ] ),
  ( cli = "fold", socket = "issue/fold", mcp = "vissue_fold", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "file", socket = "file", note = "the issue being acted on: positional on the command line, and the two remote surfaces spell it differently", toolType = "String", socketType = "String" ),
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "String", socketType = "Option<String>" )
    ] ),
  ( cli = "normalize", socket = "issue/normalize", mcp = "vissue_normalize", mutates = true, local = false,
    note = "",
    fields = [
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "dry-run", tool = "dry_run", socket = "dry_run", note = "", toolType = "Option<bool>", socketType = "bool" , omittable = true )
    ] ),
  ( cli = "agenda", socket = "issue/agenda", mcp = "vissue_agenda", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "days", tool = "days", socket = "days", note = "", toolType = "Option<i64>", socketType = "Option<i64>" ),
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "ancestors", socket = "issue/ancestors", mcp = "vissue_ancestors", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "no flag: the command line takes it as a positional argument, which is why the schema recorded every flag of this verb and not the parameter it is about. The tool spells it issue_id and the socket spells it id", toolType = "String", socketType = "String" ),
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "depth", tool = "depth", socket = "depth", note = "", toolType = "Option<usize>", socketType = "Option<usize>" )
    ] ),
  ( cli = "backlinks", socket = "issue/backlinks", mcp = "vissue_backlinks", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" )
    ] ),
  ( cli = "body-excerpt", socket = "issue/excerpt", mcp = "vissue_body_excerpt", mutates = false, local = false,
    note = "the method is named for the excerpt and the tool for the body it comes from",
    fields = [
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" )
    ] ),
  ( cli = "children", socket = "issue/children", mcp = "vissue_children", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" )
    ] ),
  ( cli = "claims", socket = "issue/claims", mcp = "vissue_claims", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "by", tool = "holder", socket = "holder", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "json", tool = "json", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "Option<bool>", socketType = "" ),
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "gen", socket = "events/gen", mcp = "vissue_gen", mutates = false, local = false,
    note = "",
    fields = [] ),
  ( cli = "events", socket = "events/since", mcp = "vissue_events", mutates = false, local = false,
    note = "the method names the sequence it reads from",
    fields = [
      ( cli = "limit", tool = "limit", socket = "limit", note = "", toolType = "Option<usize>", socketType = "Option<usize>" ),
      ( cli = "since", tool = "since", socket = "since", note = "", toolType = "Option<u64>", socketType = "u64" )
    ] ),
  ( cli = "identity", socket = "identity/get", mcp = "vissue_identity", mutates = false, local = false,
    note = "one method answers both identity and whoami",
    fields = [] ),
  ( cli = "whoami", socket = "identity/get", mcp = "vissue_whoami", mutates = false, local = false,
    note = "one method answers both identity and whoami; whoami is the older spelling",
    fields = [] ),
  ( cli = "impact", socket = "issue/impact", mcp = "vissue_impact", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "no flag: the command line takes it as a positional argument, which is why the schema recorded every flag of this verb and not the parameter it is about. The tool spells it issue_id and the socket spells it id", toolType = "String", socketType = "String" ),
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "depth", tool = "depth", socket = "depth", note = "", toolType = "Option<usize>", socketType = "Option<usize>" )
    ] ),
  ( cli = "list", socket = "issue/list", mcp = "vissue_list", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "state", tool = "state", socket = "state", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "projects", socket = "project/list", mcp = "vissue_projects", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" )
    ] ),
  ( cli = "ready", socket = "issue/ready", mcp = "vissue_ready", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "related", socket = "issue/related", mcp = "vissue_related", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "no flag: the command line takes it as a positional argument, which is why the schema recorded every flag of this verb and not the parameter it is about. The tool spells it issue_id and the socket spells it id", toolType = "String", socketType = "String" ),
      ( cli = "depth", tool = "depth", socket = "depth", note = "", toolType = "Option<usize>", socketType = "Option<usize>" ),
      ( cli = "format", tool = "format", socket = "", note = "command line only: it chooses a text rendering, and the remote surfaces answer in structure", toolType = "Option<String>", socketType = "" ),
      ( cli = "limit", tool = "limit", socket = "limit", note = "", toolType = "Option<usize>", socketType = "Option<usize>" )
    ] ),
  ( cli = "search", socket = "issue/search", mcp = "vissue_search", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "query", socket = "query", note = "no flag: the command line takes it as a positional argument, which is why the schema recorded every flag of this verb and not the parameter it is about", toolType = "String", socketType = "String" ),
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "limit", tool = "limit", socket = "limit", note = "", toolType = "Option<usize>", socketType = "Option<usize>" )
    ] ),
  ( cli = "show", socket = "issue/show", mcp = "vissue_show", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "org", tool = "", socket = "", note = "the tool list spells this vissue_org, which is its own row", toolType = "", socketType = "" )
    ] ),
  ( cli = "tree", socket = "issue/tree", mcp = "vissue_tree", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "", tool = "issue_id", socket = "id", note = "no flag: the command line takes it as a positional argument, which is why the schema recorded every flag of this verb and not the parameter it is about. The tool spells it issue_id and the socket spells it id", toolType = "String", socketType = "String" ),
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "format", tool = "format", socket = "format", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "check", socket = "issue/check", mcp = "vissue_check", mutates = false, local = false,
    note = "",
    fields = [] ),
  ( cli = "count", socket = "issue/count", mcp = "vissue_count", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "ready", tool = "ready", socket = "", note = "", toolType = "Option<bool>", socketType = "" ),
      ( cli = "state", tool = "state", socket = "state", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "cycles", socket = "issue/cycles", mcp = "vissue_cycles", mutates = false, local = false,
    note = "",
    fields = [] ),
  ( cli = "digest", socket = "issue/digest", mcp = "vissue_digest", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "json", tool = "", socket = "", note = "the remote surfaces answer in structure already, so they need no flag to ask for it", toolType = "", socketType = "" ),
      ( cli = "project", tool = "", socket = "", note = "command line only", toolType = "", socketType = "" ),
      ( cli = "quiet", tool = "", socket = "", note = "command line only: it trims the output to the id, and a caller reading a field does not need it", toolType = "", socketType = "" )
    ] ),
  ( cli = "export", socket = "issue/export", mcp = "vissue_export", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "graph", socket = "issue/graph", mcp = "vissue_graph", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "hygiene", socket = "issue/hygiene", mcp = "vissue_hygiene", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "stale-days", tool = "stale_days", socket = "stale_days", note = "", toolType = "Option<i64>", socketType = "Option<i64>" )
    ] ),
  ( cli = "mirror", socket = "", mcp = "vissue_mirror", mutates = false, local = false,
    note = "no socket method: rendering writes a file, and a file the server writes lands on the server's disk rather than the caller's",
    fields = [
      ( cli = "project", tool = "projects", socket = "", note = "repeated on the command line, a list as a tool argument", toolType = "Option<Vec<String>>", socketType = "" ),
      ( cli = "format", tool = "format", socket = "", note = "", toolType = "Option<String>", socketType = "" ),
      ( cli = "state", tool = "state", socket = "", note = "", toolType = "Option<String>", socketType = "" ),
      ( cli = "check", tool = "", socket = "", note = "selects the freshness check, which is a separate operation with its own method", toolType = "", socketType = "" ),
      ( cli = "out", tool = "", socket = "", note = "command line only: it names a file to write, and a file the server writes lands on the wrong disk", toolType = "", socketType = "" )
    ] ),
  ( cli = "ping", socket = "events/ping", mcp = "vissue_ping", mutates = false, local = false,
    note = "appends to the event log, so two calls answer differently; it changes no issue, which is why it is not a mutating verb",
    fields = [
      ( cli = "detail", tool = "detail", socket = "detail", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "roadmap", socket = "issue/roadmap", mcp = "vissue_roadmap", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "project", tool = "project", socket = "project", note = "", toolType = "Option<String>", socketType = "Option<String>" )
    ] ),
  ( cli = "wait", socket = "events/wait", mcp = "vissue_wait", mutates = false, local = false,
    note = "",
    fields = [
      ( cli = "id", tool = "id", socket = "id", note = "", toolType = "Option<String>", socketType = "Option<String>" ),
      ( cli = "last", tool = "last", socket = "last", note = "", toolType = "Option<u64>", socketType = "u64" , omittable = true ),
      ( cli = "poll-ms", tool = "poll_ms", socket = "poll_ms", note = "", toolType = "Option<u64>", socketType = "Option<u64>" ),
      ( cli = "timeout-ms", tool = "timeout_ms", socket = "timeout_ms", note = "", toolType = "Option<u64>", socketType = "Option<u64>" ),
      ( cli = "until-terminal", tool = "until_terminal", socket = "", note = "the method infers it from `id` being present", toolType = "Option<bool>", socketType = "" )
    ] ),
  ( cli = "waiting-on", socket = "issue/waiting_on", mcp = "vissue_waiting_on", mutates = false, local = false,
    note = "",
    fields = [] ),
  ( cli = "stale", socket = "issue/stale", mcp = "", mutates = false, local = false,
    note = "no tool: a stale sweep is a maintenance report rather than an agent action",
    fields = [
      ( cli = "days", tool = "", socket = "days", note = "no tool: the verb has none", toolType = "", socketType = "i64" ),
      ( cli = "project", tool = "", socket = "project", note = "no tool: the verb has none", toolType = "", socketType = "Option<String>" )
    ] ),
  ( cli = "completions", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "bash", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "zsh", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "fish", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "elvish", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "powershell", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "man", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "keys", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "hud", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "tui", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "restart", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "status", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "stop", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "serve", socket = "", mcp = "", mutates = false, local = true,
    note = "local only: it acts on this process rather than on the corpus",
    fields = [] ),
  ( cli = "", socket = "", mcp = "vissue_org", mutates = false, local = false,
    note = "no subcommand of its own: the command line spells it `show --org`, and a tool cannot take a flag",
    fields = [] ),
  ( cli = "", socket = "issue/mirror_check", mcp = "vissue_mirror_check", mutates = false, local = false,
    note = "no subcommand of its own: the command line spells it `mirror --check PATH`, and a tool cannot take a flag",
    fields = [
      ( cli = "", tool = "path", socket = "path", note = "the file being judged; the command line passes it to --check", toolType = "String", socketType = "String" ),
      ( cli = "", tool = "projects", socket = "projects", note = "empty means the stamp's own list, since the file records what it covered", toolType = "Option<Vec<String>>", socketType = "Vec<String>", omittable = true )
    ] ),
];
