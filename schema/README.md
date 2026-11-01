# The operation set

`vissue.capnp` is where the verb set is written down, and the three surfaces are
checked against it: the command line, the control socket, and the MCP tool list.

Each surface used to declare its own verbs in its own idiom, so a verb could exist
on one and not the others. That happened repeatedly and nothing caught it. `vote`
shipped on the command line alone. `append` had no socket method for as long as the
socket existed. A test asserting `issue/fold` was an unknown method became wrong the
day fold got one. The docs tests were green throughout, because they checked that
the reference listed what existed rather than whether what should exist did.

Now three tests read this file and fail by name until their surface satisfies it:

| surface | test | crate |
|---|---|---|
| command line | `the_command_line_offers_every_verb_the_schema_names` | `vissue-cli` |
| control socket | `the_socket_answers_every_method_the_schema_names` | `vissue-serve` |
| MCP tools | `the_tool_list_offers_every_tool_the_schema_names` | `vissue-mcp` |

A verb that should not reach a surface leaves that field empty and says why in
`note`; the checks skip those, so a deliberate omission is distinguishable from a
forgotten one.

Each check asks its surface rather than reading the source that declares it. The
command line answers through `vissue surface`, which walks the built
`clap::Command`. The tools answer through an MCP `tools/list`, which returns the same
schema an agent is given. The socket answers by refusing: a parameter is present
because the method rejects a value of the wrong type for it, and required because the
method rejects the request without it.

## Regenerating

The schema's constant is encoded into `crates/vissue-core/src/schema/vissue_capnp.rs`,
which is committed. Rust reads the schema's own bytes rather than a copy of them.

Committed rather than generated during the build because `capnp` the compiler is not
installed on the machines that build this, and a build-time codegen step would turn
the guarantee into a broken build. The pure-Rust `capnp` runtime reads the encoded
constant anywhere.

After editing `vissue.capnp`:

```
schema/regen.sh
```

That needs `capnp` and the `capnpc-rust` plugin, and neither is needed to build or
test. Where both are present it is one command.

They often are not, because the compiler is a distro package and the plugin is a
`cargo install`. Name a host that has the plugin and the round trip is still one
command:

```
VISSUE_CAPNPC_SSH=builder schema/regen.sh
```

Without that, the script writes the request file and prints what to run where the
plugin is, because the request is the whole input the plugin needs and it moves
between machines.

The script refuses to run when the plugin's version does not match the `capnp`
runtime crate the generated file has to compile against. Nothing in the generated
file records which version wrote it, so a newer plugin against an older runtime is
the quiet version of the drift this schema exists to prevent.

Forgetting to run it is caught rather than shipped:
`the_generated_constant_matches_the_schema_text` reads the schema text and compares it
with the committed constant, so an edit without a regeneration fails the suite with
the instruction to regenerate.
