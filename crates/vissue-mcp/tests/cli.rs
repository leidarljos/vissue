#![allow(missing_docs)]

use std::process::Command;

#[test]
fn version_flag_prints_the_binary_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_vissue-mcp"))
        .arg("--version")
        .output()
        .expect("run vissue-mcp --version");

    assert!(output.status.success(), "status: {}", output.status);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("vissue-mcp {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

/// Every tool the server advertises, asked of the server itself.
///
/// The list comes from an MCP `tools/list` over stdio, which is the same answer any
/// agent gets: names, descriptions, and the input schema each tool's arguments are
/// validated against. Reading `server.rs` as text is a guess at that answer, and it
/// was a wrong guess three times -- deriving an args struct from a tool's name
/// reported six tools as unimplemented when several share one struct, and scanning
/// for a Rust type in a source line says nothing about what the tool actually
/// accepts from a caller.
///
/// One handshake answers for all forty-three tools, and the process ends when its
/// stdin closes.
fn advertised_tools() -> &'static Vec<Tool> {
    static TOOLS: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    TOOLS.get_or_init(|| {
        use std::io::Write as _;

        let mut child = Command::new(env!("CARGO_BIN_EXE_vissue-mcp"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn vissue-mcp");

        let mut stdin = child.stdin.take().expect("stdin");
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"surface-check","version":"0"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
        ] {
            writeln!(stdin, "{line}").expect("write to vissue-mcp");
        }
        drop(stdin); // the server reads to end of input, so this is what ends it

        let out = child.wait_with_output().expect("wait for vissue-mcp");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let listed = stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find(|msg| msg["id"] == 2)
            .unwrap_or_else(|| {
                panic!(
                    "no reply to tools/list. stdout: {stdout}\nstderr: {}",
                    String::from_utf8_lossy(&out.stderr)
                )
            });
        let tools: Vec<Tool> = serde_json::from_value(listed["result"]["tools"].clone())
            .expect("tools/list carries a tool array");
        assert!(
            tools.len() > 20,
            "the server advertises {} tools, too few to be the real surface",
            tools.len()
        );
        tools
    })
}

#[derive(serde::Deserialize)]
struct Tool {
    name: String,
    #[serde(rename = "inputSchema")]
    input_schema: InputSchema,
}

#[derive(serde::Deserialize)]
struct InputSchema {
    #[serde(default)]
    properties: std::collections::BTreeMap<String, Property>,
    #[serde(default)]
    required: Vec<String>,
}

#[derive(serde::Deserialize)]
struct Property {
    /// One name, or a name and `"null"` for an argument a caller may omit.
    #[serde(default)]
    r#type: serde_json::Value,
}

impl Property {
    /// The JSON type names this property accepts.
    fn types(&self) -> Vec<String> {
        match &self.r#type {
            serde_json::Value::String(one) => vec![one.clone()],
            serde_json::Value::Array(many) => many
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// The JSON type a Rust type is advertised as, and whether it may be omitted.
///
/// The schema records the Rust type because that is what a maintainer edits. What a
/// caller sees is the JSON Schema the server derives from it, so the comparison
/// happens there: `Option<String>` has to arrive as a string a caller may omit, and
/// a field that quietly becomes required is a break in the tool's contract that
/// matching the Rust text would miss.
fn advertised_as(rust: &str) -> Option<(&'static str, bool)> {
    let (inner, optional) = match rust
        .strip_prefix("Option<")
        .and_then(|r| r.strip_suffix('>'))
    {
        Some(inner) => (inner.trim(), true),
        None => (rust.trim(), false),
    };
    let json = match inner {
        "String" | "char" | "&str" => "string",
        "bool" => "boolean",
        "u8" | "u16" | "u32" | "u64" | "usize" | "i32" | "i64" => "integer",
        "f32" | "f64" => "number",
        _ if inner.starts_with("Vec<") => "array",
        _ => return None,
    };
    Some((json, optional))
}

/// The reference documents every tool the server exposes.
///
/// A tool nobody wrote down is one an agent will not find, and the reference
/// is where the surface is published. Reading the source rather than a second
/// list keeps the two from drifting the way a hand-maintained copy does.
#[test]
fn the_reference_lists_every_tool() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let reference =
        std::fs::read_to_string(root.join("../../docs/orgmode/reference.org")).expect("reference");

    let missing: Vec<&str> = advertised_tools()
        .iter()
        .map(|t| t.name.as_str())
        .filter(|name| !reference.contains(&format!("={name}=")))
        .collect();
    assert!(
        missing.is_empty(),
        "not in docs/orgmode/reference.org: {missing:?}"
    );
}

/// The tool list carries every verb the schema names as a tool.
///
/// `vote` shipped on the command line and reached no agent for a whole session
/// because nothing asked this question. A verb the schema deliberately keeps off
/// the tool list carries its reason there, and `mutating_mcp_tools` skips those, so
/// this fails only for an omission nobody wrote down.
#[test]
fn the_tool_list_offers_every_tool_the_schema_names() {
    let tools = advertised_tools();
    let missing: Vec<String> = vissue_core::surface::mutating_mcp_tools()
        .into_iter()
        .filter(|name| !tools.iter().any(|t| &t.name == name))
        .collect();
    assert!(
        missing.is_empty(),
        "the schema names these tools and the server does not expose them: {missing:?}"
    );
}

/// Each tool takes the arguments the schema names for it.
///
/// The verb-level check says `vissue_create` exists. It does not say the tool can
/// set what the subcommand can set, and it could not: MCP `create` took neither
/// `deadline` nor `scheduled`, both of which the subcommand has always taken, so an
/// agent could not set a date a person could. Nothing was wrong on either side in
/// isolation, which is why only a check across the pair finds it.
///
/// The schema holds both spellings because some divergence is forced: `--type`
/// cannot be a Rust field named `type` and `--for` cannot be one named `for`. The
/// tool name is what is checked here, and a field the tool deliberately omits leaves
/// it empty and says why.
#[test]
fn each_tool_takes_the_arguments_the_schema_names() {
    let tools = advertised_tools();
    let mut wrong = Vec::new();
    for op in vissue_core::surface::operations() {
        // Only verbs whose fields the schema names. A read verb takes shared or no
        // arguments and declares none, so there is nothing here to satisfy.
        if op.mcp.is_empty() || !op.fields.iter().any(|f| !f.tool.is_empty()) {
            continue;
        }
        let Some(tool) = tools.iter().find(|t| t.name == op.mcp) else {
            continue; // the_tool_list_offers_every_tool_the_schema_names owns this
        };
        for field in &op.fields {
            if field.tool.is_empty() {
                continue;
            }
            let Some(property) = tool.input_schema.properties.get(&field.tool) else {
                wrong.push(format!("{} takes no {}", op.mcp, field.tool));
                continue;
            };
            // The type too, not only the name. A field going from a number to a
            // string keeps its name, so a name check cannot see it.
            let Some((json, optional_by_type)) = advertised_as(&field.tool_type) else {
                continue; // a type the schema leaves empty, or one with no JSON shape
            };
            let advertised = property.types();
            if !advertised.iter().any(|t| t == json) {
                wrong.push(format!(
                    "{}.{} is {} and arrives as {advertised:?}",
                    op.mcp, field.tool, field.tool_type
                ));
            }
            // Optionality is part of the contract a caller relies on: a field that
            // becomes required breaks every caller that omitted it, and one that
            // stops being required is a validation the tool no longer does.
            let optional = optional_by_type || field.omittable;
            let required = tool.input_schema.required.contains(&field.tool);
            if optional && required {
                wrong.push(format!(
                    "{}.{} is {} and the tool requires it",
                    op.mcp, field.tool, field.tool_type
                ));
            }
            if !optional && !required {
                wrong.push(format!(
                    "{}.{} is {} and the tool takes it as optional",
                    op.mcp, field.tool, field.tool_type
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "the schema names arguments these tools do not take: {wrong:?}"
    );
}

/// And every tool the server exposes is in the schema.
///
/// Every other check here runs schema to surface: it catches a verb the schema names
/// and the surface lacks. Nothing ran the other way, so a tool that exists and the
/// schema omits was invisible, which is the same asymmetry that let verbs drift in
/// the first place. It had already bitten: the schema claimed `normalize` had no tool
/// while `vissue_normalize` was sitting in the server, and every check passed.
#[test]
fn every_tool_the_server_exposes_is_in_the_schema() {
    let known: Vec<String> = vissue_core::surface::operations()
        .into_iter()
        .filter(|o| !o.mcp.is_empty())
        .map(|o| o.mcp)
        .collect();

    let unknown: Vec<&str> = advertised_tools()
        .iter()
        .map(|t| t.name.as_str())
        .filter(|t| !known.iter().any(|k| k == t))
        .collect();
    assert!(
        unknown.is_empty(),
        "these tools exist and no schema row mentions them: {unknown:?}"
    );
}
