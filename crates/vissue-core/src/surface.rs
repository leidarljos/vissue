//! The operation set, read from the encoded Cap'n Proto constant that
//! `capnp compile` writes from `schema/vissue.capnp`. The command line, the
//! control socket and the MCP tool list are each checked against it. The
//! generated file is committed; regenerating it is a maintainer step.

use crate::vissue_capnp::{OPERATIONS, operation};

/// One verb, named on each surface that carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    /// Subcommand name, as clap spells it.
    pub cli: String,
    /// Control-socket method, empty when the verb has none.
    pub socket: String,
    /// MCP tool name, empty when the verb is deliberately not a tool.
    pub mcp: String,
    /// Whether the verb changes a file.
    pub mutates: bool,
    /// Whether the verb only makes sense in the process it is typed into.
    pub local: bool,
    /// Other names the command line answers to for this verb.
    pub aliases: Vec<String>,
    /// The operation this verb is a narrower spelling of, empty for the ordinary case.
    pub shorthand_for: String,
    /// Why a surface is empty, when one is.
    pub note: String,
    /// Fields the verb takes, each named per surface.
    pub fields: Vec<Field>,
}

/// One field of one verb, named per surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Flag name without dashes, empty when absent or positional.
    pub cli: String,
    /// MCP argument name, empty when the tool does not take it.
    pub tool: String,
    /// Control-socket parameter name, empty when the method does not take it.
    pub socket: String,
    /// Why a surface is empty, or why the names differ.
    pub note: String,
    /// Whether a caller may leave the field out on the wire, whatever the type says.
    pub omittable: bool,
    /// Rust type of the tool argument, empty when the tool does not take it.
    pub tool_type: String,
    /// Rust type of the socket parameter, empty when the method does not take it.
    pub socket_type: String,
}

/// The operation set as the schema states it.
///
/// # Panics
///
/// Panics if the encoded constant cannot be read, which would mean the committed
/// generated file is corrupt rather than that a caller did anything wrong.
#[must_use]
pub fn operations() -> Vec<Operation> {
    let list = OPERATIONS
        .get()
        .expect("the encoded operation set in vissue_capnp.rs is unreadable");
    list.iter().map(read_one).collect()
}

fn read_one(row: operation::Reader<'_>) -> Operation {
    let text = |r: ::capnp::Result<::capnp::text::Reader<'_>>| -> String {
        r.ok()
            .and_then(|t| t.to_str().ok().map(str::to_string))
            .unwrap_or_default()
    };
    let fields = row
        .get_fields()
        .map(|list| {
            list.iter()
                .map(|f| Field {
                    cli: text(f.get_cli()),
                    tool: text(f.get_tool()),
                    socket: text(f.get_socket()),
                    note: text(f.get_note()),
                    omittable: f.get_omittable(),
                    tool_type: text(f.get_tool_type()),
                    socket_type: text(f.get_socket_type()),
                })
                .collect()
        })
        .unwrap_or_default();
    Operation {
        cli: text(row.get_cli()),
        socket: text(row.get_socket()),
        mcp: text(row.get_mcp()),
        mutates: row.get_mutates(),
        local: row.get_local(),
        aliases: row
            .get_aliases()
            .map(|list| {
                list.iter()
                    .filter_map(|a| a.ok().and_then(|t| t.to_str().ok().map(str::to_string)))
                    .collect()
            })
            .unwrap_or_default(),
        shorthand_for: text(row.get_shorthand_for()),
        note: text(row.get_note()),
        fields,
    }
}

/// Verbs the schema records as reaching the socket, mutating or not.
#[must_use]
pub fn socket_methods() -> Vec<String> {
    operations()
        .into_iter()
        .filter(|o| !o.socket.is_empty())
        .map(|o| o.socket)
        .collect()
}

/// Every subcommand the schema knows, including the local-only ones.
#[must_use]
pub fn cli_verbs() -> Vec<String> {
    operations()
        .into_iter()
        .filter(|o| !o.cli.is_empty())
        .flat_map(|o| std::iter::once(o.cli).chain(o.aliases))
        .collect()
}

/// Flags clap puts on every subcommand, which a per-verb row does not repeat.
///
/// # Panics
///
/// Panics if the encoded constant cannot be read, which would mean the committed
/// generated file is corrupt.
#[must_use]
pub fn global_flags() -> Vec<String> {
    crate::vissue_capnp::GLOBAL_FLAGS
        .get()
        .expect("the encoded global flag list is unreadable")
        .iter()
        .filter_map(|f| f.ok().and_then(|t| t.to_str().ok().map(str::to_string)))
        .collect()
}

/// Every mutating verb's socket method, skipping any the schema leaves empty.
#[must_use]
pub fn mutating_socket_methods() -> Vec<String> {
    operations()
        .into_iter()
        .filter(|o| o.mutates && !o.socket.is_empty())
        .map(|o| o.socket)
        .collect()
}

/// Every mutating verb's subcommand.
#[must_use]
pub fn mutating_cli_verbs() -> Vec<String> {
    operations()
        .into_iter()
        .filter(|o| o.mutates && !o.cli.is_empty())
        .map(|o| o.cli)
        .collect()
}

/// Every mutating verb's MCP tool, skipping the ones deliberately absent.
#[must_use]
pub fn mutating_mcp_tools() -> Vec<String> {
    operations()
        .into_iter()
        .filter(|o| o.mutates && !o.mcp.is_empty())
        .map(|o| o.mcp)
        .collect()
}

/// One operation as the schema text states it.
///
/// Everything the encoded constant can drift from without changing a surface
/// name or a field count, which is what made a note-only edit invisible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaRow {
    /// Subcommand name.
    pub cli: String,
    /// Control-socket method.
    pub socket: String,
    /// MCP tool name.
    pub mcp: String,
    /// The operation's own note.
    pub note: String,
    /// One note per field, in order, empty where a field has none.
    pub field_notes: Vec<String>,
}

/// The schema as its text states it, one [`SchemaRow`] per operation, for
/// comparison with the encoded constant. A small reader for one list of flat
/// records, not a Cap'n Proto parser.
#[must_use]
pub fn parse_schema_text(text: &str) -> Vec<SchemaRow> {
    let Some(start) = text.find("const operations") else {
        return Vec::new();
    };
    let body = &text[start..];
    let mut out: Vec<SchemaRow> = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let quoted = |name: &str| -> Option<String> {
            let at = trimmed.find(&format!("{name} = "))?;
            let rest = &trimmed[at..];
            let open = rest.find('"')?;
            let close = rest[open + 1..].find('"')?;
            Some(rest[open + 1..open + 1 + close].to_string())
        };
        // An operation row opens with its three surface names on one line.
        if trimmed.starts_with("( cli = ")
            && trimmed.contains("mutates = ")
            && let (Some(cli), Some(socket), Some(mcp)) =
                (quoted("cli"), quoted("socket"), quoted("mcp"))
        {
            out.push(SchemaRow {
                cli,
                socket,
                mcp,
                note: String::new(),
                field_notes: Vec::new(),
            });
            continue;
        }
        // An operation's note is on its own line under the row it belongs to.
        // It is read because a note-only edit is otherwise invisible: the
        // surfaces and the field count are unchanged, so the comparison passed
        // while the constant still carried the previous prose.
        if trimmed.starts_with("note = ")
            && let (Some(note), Some(last)) = (quoted("note"), out.last_mut())
        {
            last.note = note;
            continue;
        }
        // A field row also opens with `( cli = `, so it is told apart by carrying a
        // `tool =` and no `socket = "issue/`. Its note rides on the same line.
        if trimmed.starts_with("( cli = ")
            && trimmed.contains("tool = ")
            && !trimmed.contains("mutates = ")
            && let Some(last) = out.last_mut()
        {
            last.field_notes.push(quoted("note").unwrap_or_default());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_text() -> String {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/vissue.capnp");
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// The committed generated file and the schema text say the same thing.
    #[test]
    fn the_generated_constant_matches_the_schema_text() {
        let from_text = parse_schema_text(&schema_text());
        assert!(
            !from_text.is_empty(),
            "no operations parsed from the schema text; the reader and the file have diverged"
        );
        let from_bytes: Vec<SchemaRow> = operations()
            .into_iter()
            .map(|o| SchemaRow {
                cli: o.cli,
                socket: o.socket,
                mcp: o.mcp,
                note: o.note,
                field_notes: o.fields.into_iter().map(|f| f.note).collect(),
            })
            .collect();
        assert_eq!(
            from_text, from_bytes,
            "schema/vissue.capnp and the committed vissue_capnp.rs disagree; \
             regenerate it, see schema/README.md"
        );
    }

    /// Notes are compared too, so a note-only edit without a regeneration fails.
    #[test]
    fn a_note_the_schema_states_reaches_the_constant() {
        let ops = operations();
        let backlinks = ops
            .iter()
            .find(|o| o.cli == "backlinks")
            .expect("backlinks is in the operation set");
        assert!(
            backlinks.note.contains("scan every routed tracker"),
            "the command line and tool half of the split is missing: {:?}",
            backlinks.note
        );
        assert!(
            backlinks.note.contains("the layout it was started on"),
            "the socket half of the split is missing: {:?}",
            backlinks.note
        );
    }

    /// The helpers three checks now share, so their own behaviour is pinned rather
    /// than assumed by each caller.

    #[test]
    fn the_schema_constant_reads_back() {
        let ops = operations();
        assert!(
            ops.len() >= 10,
            "the encoded operation set looks truncated: {ops:?}"
        );
        assert!(ops.iter().any(|o| o.cli == "create"));
        assert!(ops.iter().any(|o| o.cli == "vote"), "vote is missing");
    }

    /// Every mutating verb reaches the socket, itself or through the verb it
    /// is a shorthand for.
    #[test]
    fn every_mutating_verb_reaches_the_socket() {
        let ops = operations();
        let mut missing = Vec::new();
        for op in ops.iter().filter(|o| o.mutates && o.socket.is_empty()) {
            if op.shorthand_for.is_empty() {
                missing.push(format!("{} reaches no socket method", op.cli));
                continue;
            }
            match ops.iter().find(|o| o.cli == op.shorthand_for) {
                None => missing.push(format!(
                    "{} is a shorthand for {}, which is in no row",
                    op.cli, op.shorthand_for
                )),
                Some(target) if target.socket.is_empty() => missing.push(format!(
                    "{} is a shorthand for {}, which reaches no socket method either",
                    op.cli, op.shorthand_for
                )),
                Some(_) => {}
            }
        }
        assert!(
            missing.is_empty(),
            "these change a file and no socket method can: {missing:?}"
        );
    }

    /// A shorthand takes a subset of the fields of the verb it shortens.
    #[test]
    fn a_shorthand_takes_a_subset_of_what_it_shortens() {
        let ops = operations();
        let mut wrong = Vec::new();
        for op in ops.iter().filter(|o| !o.shorthand_for.is_empty()) {
            let Some(target) = ops.iter().find(|o| o.cli == op.shorthand_for) else {
                continue; // reported by every_mutating_verb_reaches_the_socket
            };
            for field in &op.fields {
                if field.cli.is_empty() {
                    continue;
                }
                if !target.fields.iter().any(|f| f.cli == field.cli) {
                    wrong.push(format!(
                        "{} takes --{} and {} does not",
                        op.cli, field.cli, target.cli
                    ));
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "these shorthands take fields the verb they shorten does not: {wrong:?}"
        );
    }

    /// A shorthand names a verb other than itself, and that verb is not itself a
    /// shorthand. A cycle or a chain would make the reachability argument circular.
    #[test]
    fn a_shorthand_points_at_a_verb_that_stands_on_its_own() {
        let ops = operations();
        let mut wrong = Vec::new();
        for op in ops.iter().filter(|o| !o.shorthand_for.is_empty()) {
            if op.shorthand_for == op.cli {
                wrong.push(format!("{} is a shorthand for itself", op.cli));
            }
            if let Some(target) = ops
                .iter()
                .find(|o| o.cli == op.shorthand_for)
                .filter(|t| !t.shorthand_for.is_empty())
            {
                wrong.push(format!(
                    "{} shortens {}, which shortens {}",
                    op.cli, target.cli, target.shorthand_for
                ));
            }
        }
        assert!(wrong.is_empty(), "{wrong:?}");
    }

    /// A surface left empty says why, so a deliberate omission cannot pass for an
    /// oversight or the other way round.
    #[test]
    fn a_missing_surface_carries_its_reason() {
        for o in operations() {
            if o.socket.is_empty() || o.mcp.is_empty() {
                assert!(
                    !o.note.is_empty(),
                    "{} leaves a surface empty and says nothing about why",
                    o.cli
                );
            }
        }
    }

    /// Names are not blank and not accidentally duplicated.
    #[test]
    fn the_names_are_distinct_and_present() {
        let ops = operations();
        // A row may have no subcommand (`vissue_org` is `show --org`); it must
        // reach some surface.
        for o in &ops {
            assert!(
                !(o.cli.is_empty() && o.socket.is_empty() && o.mcp.is_empty()),
                "an operation reaches no surface at all: {o:?}"
            );
        }
        // Empty is not a name. Two rows without a subcommand are two operations the
        // command line reaches through a flag, not a collision.
        let mut clis: Vec<&str> = ops
            .iter()
            .map(|o| o.cli.as_str())
            .filter(|c| !c.is_empty())
            .collect();
        clis.sort_unstable();
        let before = clis.len();
        clis.dedup();
        assert_eq!(before, clis.len(), "two operations share a subcommand");

        // Two subcommands may share a method (`identity` and `whoami`) when the
        // rows say so; an undocumented duplicate is refused.
        let mut by_method: std::collections::BTreeMap<&str, Vec<&Operation>> =
            std::collections::BTreeMap::new();
        for o in &ops {
            if !o.socket.is_empty() {
                by_method.entry(o.socket.as_str()).or_default().push(o);
            }
        }
        for (method, sharers) in by_method {
            if sharers.len() < 2 {
                continue;
            }
            let silent: Vec<&str> = sharers
                .iter()
                .filter(|o| o.note.is_empty())
                .map(|o| o.cli.as_str())
                .collect();
            assert!(
                silent.is_empty(),
                "{method} answers for {silent:?} and none of them says why"
            );
        }
    }
}
