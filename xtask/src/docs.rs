//! The manual and the reference flag tables, taken from the CLI.
//!
//! A flag renamed in the parser and left alone in the docs is a page that
//! describes a command the binary no longer has. `vissue man` is the manual.
//! The tables are what `vissue --help` prints, command by command, which is
//! the text a seat reads. They are cogged into `docs/orgmode/reference.org`.
//! `docs --check` fails while either committed copy differs, the same way the
//! committed shell completions are checked.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `TAB` in clap_builder 4.6.7 `output/mod.rs`: the indent of a flag line.
const CLAP_TAB: usize = 2;
/// Spaces clap writes in place of a short flag so the long names line up
/// (`"    "` in `HelpTemplate::short`).
const CLAP_MISSING_SHORT: usize = 4;
/// `NEXT_LINE_INDENT` in clap_builder `help_template.rs`, added under a flag
/// when its help is on the next line.
const CLAP_NEXT_LINE_INDENT: usize = 8;

/// Pandoc is run with `--columns=400` (`docs/scripts/export_org_to_rst.sh`).
/// A longer org table row becomes a grid table, and a grid cell wraps. A wrap
/// inside `=verbatim=` leaves the published page with an unclosed literal.
const EXPORT_LINE_COLUMNS: usize = 400;

/// Regenerate `man/vissue.1` and the flag tables, or check them.
///
/// `--flags` prints the tables. `--check` exits with an error when the
/// committed manual or the cog block differs. `--bin` is the `vissue` to
/// read; without it, the binary next to this one (or the test harness) is
/// used.
pub(crate) fn run(flags: &[String]) -> Result<(), String> {
    let mut check_only = false;
    let mut print_flags = false;
    let mut bin: Option<PathBuf> = None;
    let mut args = flags.iter();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--check" => check_only = true,
            "--flags" => print_flags = true,
            "--bin" => {
                let path = args
                    .next()
                    .ok_or_else(|| "docs: --bin needs the path of the vissue binary".to_string())?;
                bin = Some(PathBuf::from(path));
            }
            other => return Err(format!("docs: unknown flag {other}\n\n{USAGE}")),
        }
    }
    if check_only && print_flags {
        return Err(
            "docs: --flags prints the tables and --check compares them; pass one".to_string(),
        );
    }
    let bin = resolve_bin(bin)?;
    if print_flags {
        print!("{}", flag_tables(&bin)?);
        return Ok(());
    }
    if check_only {
        return check(&bin);
    }
    write_outputs(&bin)
}

const USAGE: &str = "docs [--check|--flags] [--bin PATH]";

fn resolve_bin(bin: Option<PathBuf>) -> Result<PathBuf, String> {
    let path = bin.unwrap_or_else(vissue_binary);
    if path.is_file() {
        return Ok(path);
    }
    Err(format!(
        "docs: no vissue binary at {}; build it with `cargo build -p vissue-cli --bin vissue` or pass --bin",
        path.display()
    ))
}

fn vissue_binary() -> PathBuf {
    let name = format!("vissue{}", std::env::consts::EXE_SUFFIX);
    // `cargo run` places this binary next to `vissue`. `cargo test` places
    // the harness in `target/<profile>/deps`, one directory further down.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join(&name);
        if sibling.is_file() {
            return sibling;
        }
        if let Some(parent) = dir.parent() {
            let above = parent.join(&name);
            if above.is_file() {
                return above;
            }
        }
    }
    repo_root().join("target").join("debug").join(name)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn write_outputs(bin: &Path) -> Result<(), String> {
    let root = repo_root();
    write_if_changed(&root.join("man/vissue.1"), &man_page(bin)?)?;
    let reference = root.join("docs/orgmode/reference.org");
    let text = read(&reference)?;
    write_if_changed(&reference, &splice(&text, &flag_tables(bin)?)?)?;
    Ok(())
}

fn write_if_changed(path: &Path, contents: &str) -> Result<(), String> {
    if path.is_file() && read(path)? == contents {
        return Ok(());
    }
    fs::write(path, contents).map_err(|err| format!("write {}: {err}", path.display()))
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))
}

/// The committed manual and the cogged flag tables, compared to this binary.
fn check(bin: &Path) -> Result<(), String> {
    let root = repo_root();
    let mut problems = Vec::new();

    let man = man_page(bin)?;
    let man_path = root.join("man/vissue.1");
    let committed_man = read(&man_path)?;
    if man != committed_man {
        problems.push(format!(
            "man/vissue.1 is stale; regenerate with `cargo run -q -p xtask -- docs`\n{}",
            unified(&committed_man, &man)
        ));
    }

    let reference = root.join("docs/orgmode/reference.org");
    let text = read(&reference)?;
    let committed = generated_region(&text)?;
    let tables = flag_tables(bin)?;
    if tables != committed {
        problems.push(format!(
            "docs/orgmode/reference.org flag tables are stale; regenerate with `cargo run -q -p xtask -- docs`\n{}",
            unified(committed, &tables)
        ));
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

fn unified(old: &str, new: &str) -> String {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let before = dir.join(format!("vissue-docs-old-{pid}"));
    let after = dir.join(format!("vissue-docs-new-{pid}"));
    if fs::write(&before, old).is_err() || fs::write(&after, new).is_err() {
        return String::new();
    }
    let out = Command::new("diff")
        .arg("-u")
        .arg(&before)
        .arg(&after)
        .output();
    let _ = fs::remove_file(&before);
    let _ = fs::remove_file(&after);
    out.map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

/// `# ]]]` closes the cog Python. The tables are everything after it, up to
/// `# [[[end]]]`.
fn generated_region(text: &str) -> Result<&str, String> {
    let (before, after) = split_block(text)?;
    let rest = &text[before.len()..];
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let region_len = rest
        .len()
        .checked_sub(after.len())
        .ok_or_else(|| "docs/orgmode/reference.org cog block ends before it starts".to_string())?;
    Ok(&rest[..region_len])
}

fn splice(text: &str, generated: &str) -> Result<String, String> {
    let (before, after) = split_block(text)?;
    let mut generated = generated.to_string();
    if !generated.ends_with('\n') {
        generated.push('\n');
    }
    Ok(format!("{before}\n{generated}{after}"))
}

fn split_block(text: &str) -> Result<(&str, &str), String> {
    const CODE_END: &str = "# ]]]";
    const BLOCK_END: &str = "# [[[end]]]";
    let Some(start) = text.find(CODE_END) else {
        return Err(
            "docs/orgmode/reference.org is missing the cog block that holds the flag tables"
                .to_string(),
        );
    };
    let after_code = start + CODE_END.len();
    let Some(rel) = text[after_code..].find(BLOCK_END) else {
        return Err("docs/orgmode/reference.org cog block has no `# [[[end]]]`".to_string());
    };
    Ok((&text[..after_code], &text[after_code + rel..]))
}

fn man_page(bin: &Path) -> Result<String, String> {
    invoke(bin, ["man"])
}

fn flag_tables(bin: &Path) -> Result<String, String> {
    let root_help = parse_help(&invoke(bin, ["--help"])?)?;
    let shared = shared_names(&root_help.options)?;
    let mut out = render_shared(&root_help.options)?;
    for command in &root_help.commands {
        render_tree(
            bin,
            std::slice::from_ref(&command.spec),
            &command.help,
            &shared,
            &mut out,
        )?;
    }
    ensure_export_lines(&out)?;
    Ok(out)
}

/// Long names of the flags on the root command.
///
/// Clap copies each `global = true` flag onto every subcommand's help, so
/// the reference lists them once. A command's own table is then the flags
/// that command adds.
fn shared_names(options: &[Item]) -> Result<BTreeSet<String>, String> {
    options.iter().map(|item| long_name(&item.spec)).collect()
}

fn render_tree(
    bin: &Path,
    path: &[String],
    parent_about: &str,
    shared: &BTreeSet<String>,
    out: &mut String,
) -> Result<(), String> {
    let name = path
        .last()
        .ok_or_else(|| "docs: empty command path".to_string())?;
    if path.iter().filter(|part| *part == name).count() > 1 {
        return Err(format!("docs: help for {} lists itself", path.join(" ")));
    }
    // `vissue help --help` is parsed as "explain the --help command", which
    // is not a command, so the help command has no help page of its own.
    // The line in its parent's command list is the description clap gives it.
    if name == "help" {
        push_chunk(out, &render_help_leaf(path, parent_about)?);
        return Ok(());
    }
    let doc = parse_help(&invoke(bin, help_args(path))?)
        .map_err(|err| format!("{}: {err}", path.join(" ")))?;
    push_chunk(out, &render_command(path, &doc, shared)?);
    for sub in &doc.commands {
        let mut child = path.to_vec();
        child.push(sub.spec.clone());
        render_tree(bin, &child, &sub.help, shared, out)?;
    }
    Ok(())
}

fn help_args(path: &[String]) -> Vec<String> {
    let mut args = path.to_vec();
    args.push("--help".to_string());
    args
}

fn push_chunk(out: &mut String, chunk: &str) {
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(chunk);
}

fn render_shared(options: &[Item]) -> Result<String, String> {
    let mut blocks = vec!["** Shared flags".to_string()];
    blocks.push(option_table(options)?);
    Ok(join_blocks(&blocks))
}

fn render_help_leaf(path: &[String], about: &str) -> Result<String, String> {
    let mut blocks = vec![heading(path)];
    if !about.is_empty() {
        blocks.push(org_prose(about)?);
    }
    Ok(join_blocks(&blocks))
}

fn render_command(
    path: &[String],
    doc: &HelpDoc,
    shared: &BTreeSet<String>,
) -> Result<String, String> {
    let mut blocks = vec![heading(path)];
    for paragraph in &doc.about {
        blocks.push(org_prose(paragraph)?);
    }
    blocks.push(usage_line(&doc.usage)?);
    if !doc.arguments.is_empty() {
        blocks.push(argument_table(&doc.arguments)?);
    }
    let mut owned = Vec::new();
    for item in &doc.options {
        if !shared.contains(&long_name(&item.spec)?) {
            owned.push(item.clone());
        }
    }
    if !owned.is_empty() {
        blocks.push(option_table(&owned)?);
    } else if doc.arguments.is_empty() {
        blocks.push("No flags of its own.".to_string());
    }
    Ok(join_blocks(&blocks))
}

fn heading(path: &[String]) -> String {
    format!("** ={}=", path.join(" "))
}

fn usage_line(usage: &str) -> Result<String, String> {
    if usage.contains('=') || usage.contains('|') {
        return Err(format!("usage cannot be marked verbatim: {usage}"));
    }
    Ok(format!("Usage: ={usage}="))
}

fn join_blocks(blocks: &[String]) -> String {
    let mut out = String::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            out.push_str("\n\n");
        }
        out.push_str(block.trim_end_matches('\n'));
    }
    out.push('\n');
    out
}

fn argument_table(items: &[Item]) -> Result<String, String> {
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        if item.spec.contains('=') {
            return Err(format!("argument cannot be marked verbatim: {}", item.spec));
        }
        rows.push((format!("={}=", item.spec), org_prose(&item.help)?));
    }
    Ok(table("Argument", "Meaning", &rows))
}

fn option_table(items: &[Item]) -> Result<String, String> {
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        rows.push((render_option(&item.spec)?, org_prose(&item.help)?));
    }
    Ok(table("Flag", "Meaning", &rows))
}

fn table(left: &str, right: &str, rows: &[(String, String)]) -> String {
    let mut out = format!("| {left} | {right} |\n");
    out.push('|');
    out.push_str(&"-".repeat(left.len() + 2));
    out.push('+');
    out.push_str(&"-".repeat(right.len() + 2));
    out.push_str("|\n");
    for (flag, meaning) in rows {
        out.push_str(&format!("| {flag} | {meaning} |\n"));
    }
    out
}

fn render_option(spec: &str) -> Result<String, String> {
    let (short, rest) = match spec.strip_prefix('-') {
        Some(after) if !after.starts_with('-') => {
            let mut chars = after.chars();
            let Some(flag) = chars.next().filter(char::is_ascii_alphanumeric) else {
                return Err(format!("option has no short flag: {spec}"));
            };
            let mut rest = chars.as_str();
            if let Some(stripped) = rest.strip_prefix(',') {
                rest = stripped.trim_start();
            }
            (Some(flag), rest)
        }
        _ => (None, spec),
    };
    let Some(rest) = rest.strip_prefix("--") else {
        return Err(format!("option has no long name: {spec}"));
    };
    let (long, value) = match rest.split_once(char::is_whitespace) {
        Some((long, value)) => (long, Some(value.trim())),
        None => (rest, None),
    };
    if long.is_empty() {
        return Err(format!("option has no long name: {spec}"));
    }
    let mut out = String::new();
    if let Some(flag) = short {
        out.push_str(&format!("=-{flag}=, "));
    }
    out.push_str(&format!("=--{long}="));
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        if value.contains('=') {
            return Err(format!("option value cannot be marked verbatim: {value}"));
        }
        out.push(' ');
        out.push_str(&format!("={value}="));
    }
    Ok(out)
}

fn long_name(spec: &str) -> Result<String, String> {
    let Some(rest) = spec.split("--").nth(1) else {
        return Err(format!("option has no long name: {spec}"));
    };
    let name = rest.split_whitespace().next().unwrap_or(rest);
    if name.is_empty() {
        return Err(format!("option has no long name: {spec}"));
    }
    Ok(name.to_string())
}

fn ensure_export_lines(tables: &str) -> Result<(), String> {
    for (index, line) in tables.lines().enumerate() {
        if line.chars().count() > EXPORT_LINE_COLUMNS {
            return Err(format!(
                "generated reference line {} is {} characters; docs/scripts/export_org_to_rst.sh runs pandoc at --columns={EXPORT_LINE_COLUMNS}, and a longer row becomes a grid table whose wrap can split =verbatim=",
                index + 1,
                line.chars().count()
            ));
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Item {
    spec: String,
    help: String,
}

struct HelpDoc {
    about: Vec<String>,
    usage: String,
    arguments: Vec<Item>,
    options: Vec<Item>,
    commands: Vec<Item>,
}

#[derive(Clone, Copy)]
enum Section {
    Arguments,
    Options,
    Commands,
}

enum LineKind {
    Blank,
    Entry(Item),
    Continuation(String),
}

fn parse_help(text: &str) -> Result<HelpDoc, String> {
    let lines: Vec<&str> = text.lines().collect();
    let usage_at = lines
        .iter()
        .position(|line| line.starts_with("Usage:"))
        .ok_or_else(|| "help has no Usage line".to_string())?;
    let usage = lines[usage_at]
        .trim_start_matches("Usage:")
        .trim()
        .to_string();
    if usage.is_empty() {
        return Err("help has an empty Usage line".to_string());
    }
    let mut index = usage_at + 1;
    let mut arguments = Vec::new();
    let mut options = Vec::new();
    let mut commands = Vec::new();
    while index < lines.len() {
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let Some(section) = section_of(lines[index]) else {
            return Err(format!("unexpected help line: {}", lines[index]));
        };
        index += 1;
        let (next, items) = parse_items(&lines, index, section)?;
        index = next;
        match section {
            Section::Arguments => arguments = items,
            Section::Options => options = items,
            Section::Commands => commands = items,
        }
    }
    // Every command clap builds has `--help`. An empty option list means the
    // parser lost the flags rather than that the command has none.
    if options.is_empty() {
        return Err("help showed no options".to_string());
    }
    Ok(HelpDoc {
        about: paragraphs(&lines[..usage_at]),
        usage,
        arguments,
        options,
        commands,
    })
}

fn section_of(line: &str) -> Option<Section> {
    match line {
        "Arguments:" => Some(Section::Arguments),
        "Options:" => Some(Section::Options),
        "Commands:" => Some(Section::Commands),
        _ => None,
    }
}

fn paragraphs(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in lines {
        if line.trim().is_empty() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(line.trim());
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn parse_items(
    lines: &[&str],
    mut index: usize,
    section: Section,
) -> Result<(usize, Vec<Item>), String> {
    let mut items = Vec::new();
    let mut current: Option<Item> = None;
    while index < lines.len() {
        if section_of(lines[index]).is_some() {
            break;
        }
        match classify(lines[index], section)? {
            LineKind::Blank => {}
            LineKind::Entry(item) => {
                if let Some(done) = current.take() {
                    items.push(done);
                }
                current = Some(item);
            }
            LineKind::Continuation(text) => {
                let Some(item) = current.as_mut() else {
                    return Err(format!("help text before its flag: {}", lines[index]));
                };
                if text.is_empty() {
                    // clap puts a blank indented line between the sentence and
                    // `[default: ...]` / `[possible values: ...]`.
                } else if item.help.is_empty() {
                    item.help = text;
                } else {
                    item.help.push(' ');
                    item.help.push_str(&text);
                }
            }
        }
        index += 1;
    }
    if let Some(done) = current {
        items.push(done);
    }
    Ok((index, items))
}

fn classify(line: &str, section: Section) -> Result<LineKind, String> {
    if line.trim().is_empty() {
        return Ok(LineKind::Blank);
    }
    let indent = line.len() - line.trim_start().len();
    let rest = line.trim_start();
    // Next-line help is indented by TAB + NEXT_LINE_INDENT. A flag line is
    // indented by TAB, plus the missing-short padding when it has no `-x`.
    if indent >= CLAP_TAB + CLAP_NEXT_LINE_INDENT {
        return Ok(LineKind::Continuation(rest.to_string()));
    }
    let entry_indent = indent == CLAP_TAB || indent == CLAP_TAB + CLAP_MISSING_SHORT;
    if !entry_indent {
        return Err(format!("unrecognised help line: {line:?}"));
    }
    let item = match section {
        Section::Options => parse_option(rest)?,
        Section::Arguments => parse_argument(rest)?,
        Section::Commands => parse_command_line(rest)?,
    };
    Ok(LineKind::Entry(item))
}

fn parse_option(rest: &str) -> Result<Item, String> {
    let spec_end = option_spec_end(rest)?;
    Ok(Item {
        spec: rest[..spec_end].trim().to_string(),
        help: rest[spec_end..].trim().to_string(),
    })
}

fn option_spec_end(rest: &str) -> Result<usize, String> {
    let mut index = 0usize;
    let chars: Vec<char> = rest.chars().collect();
    if chars.first() == Some(&'-') && chars.get(1) != Some(&'-') {
        if !chars.get(1).is_some_and(char::is_ascii_alphanumeric) {
            return Err(format!("not an option: {rest}"));
        }
        index = 2;
        if chars.get(index) == Some(&',') {
            index += 1;
            if chars.get(index) == Some(&' ') {
                index += 1;
            }
        }
    }
    if chars.get(index) != Some(&'-') || chars.get(index + 1) != Some(&'-') {
        return Err(format!("not an option: {rest}"));
    }
    index += 2;
    let long_at = index;
    while index < chars.len() && (chars[index].is_ascii_alphanumeric() || chars[index] == '-') {
        index += 1;
    }
    if index == long_at {
        return Err(format!("option has no long name: {rest}"));
    }
    if chars.get(index) == Some(&' ')
        && chars.get(index + 1) == Some(&'<')
        && let Some(end) = value_token_end(&chars, index + 1)
    {
        index = end;
    }
    Ok(index)
}

fn parse_argument(rest: &str) -> Result<Item, String> {
    let chars: Vec<char> = rest.chars().collect();
    let spec_end = argument_spec_end(&chars).ok_or_else(|| format!("not an argument: {rest}"))?;
    Ok(Item {
        spec: rest[..spec_end].to_string(),
        help: rest[spec_end..].trim().to_string(),
    })
}

fn argument_spec_end(chars: &[char]) -> Option<usize> {
    let close = match chars.first() {
        Some('<') => '>',
        Some('[') => ']',
        _ => return None,
    };
    let mut index = 1usize;
    if index >= chars.len() {
        return None;
    }
    while index < chars.len() && chars[index] != close {
        let name = chars[index];
        if !name.is_ascii_uppercase() && !name.is_ascii_digit() && name != '_' {
            return None;
        }
        index += 1;
    }
    if index == 1 || chars.get(index) != Some(&close) {
        return None;
    }
    index += 1;
    if chars.get(index..index + 3) == Some(&['.', '.', '.']) {
        index += 3;
    }
    Some(index)
}

fn value_token_end(chars: &[char], start: usize) -> Option<usize> {
    // Reuse the argument grammar: `<IDENT>` plus an optional `...`.
    argument_spec_end(&chars[start..]).map(|end| start + end)
}

fn parse_command_line(rest: &str) -> Result<Item, String> {
    let name_len = rest
        .chars()
        .take_while(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '-')
        .count();
    if name_len == 0 {
        return Err(format!("not a command: {rest}"));
    }
    Ok(Item {
        spec: rest[..name_len].to_string(),
        help: rest[name_len..].trim().to_string(),
    })
}

/// Help text as org that pandoc will not turn into a subscript, an en-dash,
/// or a link.
///
/// `` `code` `` becomes `=code=`. A bare `--flag` becomes `=--flag=` because
/// pandoc reads `--` as an en-dash. A token containing `_`, `$`, `~`, or `*`
/// is marked the same way: pandoc reads `_` as a subscript, so
/// `$XDG_RUNTIME_DIR` would otherwise publish as a broken subscript.
fn org_prose(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Ok(String::new());
    }
    let marked = backticks_to_verbatim(text)?;
    map_outside_verbatim(&marked, protect_plain)
}

fn backticks_to_verbatim(text: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            return Err(format!("unclosed backtick in help text: {text}"));
        };
        let inner = &after[..end];
        if inner.is_empty() || inner.contains('=') || inner.contains('\n') || inner.contains('|') {
            return Err(format!("backtick span cannot be marked verbatim: {inner}"));
        }
        out.push('=');
        out.push_str(inner);
        out.push('=');
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn map_outside_verbatim(
    text: &str,
    mut protect: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('=') {
        out.push_str(&protect(&rest[..start])?);
        let after = &rest[start + 1..];
        let Some(end) = after.find('=') else {
            return Err(format!("unclosed verbatim in help text: {text}"));
        };
        out.push('=');
        out.push_str(&after[..end]);
        out.push('=');
        rest = &after[end + 1..];
    }
    out.push_str(&protect(rest)?);
    Ok(out)
}

fn protect_plain(text: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut index = 0usize;
    while index < text.len() {
        let rest = &text[index..];
        if let Some(flag) = long_flag_at(rest) {
            out.push('=');
            out.push_str(flag);
            out.push('=');
            index += flag.len();
            continue;
        }
        let ch = rest.chars().next().expect("index is on a character");
        if matches!(ch, '[' | '<')
            && let Some(span) = bracket_span(rest, ch)
        {
            if span.contains('=') || span.contains('|') {
                return Err(format!("bracket span cannot be marked verbatim: {span}"));
            }
            out.push('=');
            out.push_str(span);
            out.push('=');
            index += span.len();
            continue;
        }
        if ch.is_whitespace() {
            out.push(ch);
            index += ch.len_utf8();
            continue;
        }
        let word_len = word_len(rest);
        if word_len == 0 {
            out.push(ch);
            index += ch.len_utf8();
            continue;
        }
        out.push_str(&wrap_token(&rest[..word_len])?);
        index += word_len;
    }
    Ok(out)
}

fn long_flag_at(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("--")?;
    let name_len = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .map(char::len_utf8)
        .sum::<usize>();
    if name_len == 0 {
        return None;
    }
    Some(&text[..2 + name_len])
}

fn bracket_span(text: &str, open: char) -> Option<&str> {
    let close = if open == '[' { ']' } else { '>' };
    let after = text.strip_prefix(open)?;
    let rel = after.find(close)?;
    Some(&text[..open.len_utf8() + rel + close.len_utf8()])
}

fn word_len(text: &str) -> usize {
    let mut end = 0usize;
    for (offset, ch) in text.char_indices() {
        if ch.is_whitespace() || ch == '[' || ch == '<' || text[offset..].starts_with("--") {
            break;
        }
        end = offset + ch.len_utf8();
    }
    end
}

fn wrap_token(token: &str) -> Result<String, String> {
    let lead = token
        .chars()
        .take_while(|ch| matches!(ch, '(' | '"' | '\''))
        .count();
    let trail = token
        .chars()
        .rev()
        .take_while(|ch| matches!(ch, ')' | '"' | '\'' | ',' | '.' | ';' | ':' | '!' | '?'))
        .count();
    let chars: Vec<char> = token.chars().collect();
    if lead + trail >= chars.len() {
        return Ok(token.to_string());
    }
    let core: String = chars[lead..chars.len() - trail].iter().collect();
    if !needs_verbatim(&core) {
        return Ok(token.to_string());
    }
    if core.contains('=') || core.contains('|') {
        return Err(format!("token cannot be marked verbatim: {core}"));
    }
    let lead: String = chars[..lead].iter().collect();
    let trail: String = chars[chars.len() - trail..].iter().collect();
    Ok(format!("{lead}={core}={trail}"))
}

fn needs_verbatim(core: &str) -> bool {
    core.contains('_')
        || core.contains('$')
        || core.contains('~')
        || core.contains('*')
        || core.contains('<')
        || core.contains('>')
        || core.contains('[')
        || core.contains(']')
}

/// Run the binary with stdout captured.
///
/// `NO_COLOR` keeps the bytes free of colour sequences. Help and the manual
/// are compared in the check, and a coloured run would differ from a plain
/// one without the CLI having changed.
fn invoke(
    bin: &Path,
    args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
) -> Result<String, String> {
    let out = Command::new(bin)
        .args(args)
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("CLICOLOR_FORCE", "0")
        .output()
        .map_err(|err| format!("run {}: {err}", bin.display()))?;
    if !out.status.success() {
        return Err(format!(
            "{} failed: {}",
            bin.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8(out.stdout).map_err(|err| format!("help is not utf-8: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_prose_marks_the_characters_org_would_misread() {
        let text = org_prose(
            "Ignore `$VISSUE_CONFIG` / `~/.config/vissue/config.toml` and --no-route, then ~/.vissue/run and $XDG_RUNTIME_DIR/vissue.",
        )
        .unwrap();
        assert!(text.contains("=$VISSUE_CONFIG="), "{text}");
        assert!(text.contains("=~/.config/vissue/config.toml="), "{text}");
        assert!(text.contains("=--no-route="), "{text}");
        assert!(text.contains("=~/.vissue/run="), "{text}");
        assert!(text.contains("=$XDG_RUNTIME_DIR/vissue="), "{text}");
        let plain = outside_verbatim(&text);
        assert!(
            !plain.contains('_'),
            "a raw underscore becomes a subscript on the site: {text}"
        );
        assert!(
            !plain.contains("--"),
            "a raw -- becomes an en-dash on the site: {text}"
        );
    }

    fn outside_verbatim(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        let mut inside = false;
        while let Some(at) = rest.find('=') {
            if !inside {
                out.push_str(&rest[..at]);
            }
            inside = !inside;
            rest = &rest[at + 1..];
        }
        if !inside {
            out.push_str(rest);
        }
        out
    }

    /// A renamed flag changes what `--help` prints. The committed manual and
    /// the cogged tables are that text, so the check fails until `docs`
    /// rewrites them.
    #[test]
    fn committed_manual_and_flag_tables_match_the_cli() {
        let bin = vissue_binary();
        assert!(
            bin.is_file(),
            "build the CLI first: cargo build -p vissue-cli --bin vissue (missing {})",
            bin.display()
        );
        let tables = flag_tables(&bin).unwrap();
        let help = invoke(&bin, ["--help"]).unwrap();
        let first = help
            .lines()
            .skip_while(|line| *line != "Commands:")
            .nth(1)
            .and_then(|line| line.split_whitespace().next())
            .expect("root help lists a command");
        assert!(
            tables.contains(&format!("** ={first}=")),
            "flag tables dropped {first}"
        );
        assert!(tables.contains("** Shared flags"), "{tables}");
        let man = man_page(&bin).unwrap();
        assert!(
            man.contains(".TH vissue"),
            "the manual page did not name the program"
        );
        if let Err(err) = check(&bin) {
            panic!("{err}");
        }
    }
}
