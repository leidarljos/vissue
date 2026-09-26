//! Org 9.8 syntax the tracker has to get right.
//!
//! An `issues.org` is an ordinary Org document. The verbs treat a top-level
//! TODO heading as an issue; everything else in the file is still Org, and a
//! construct Org would not treat as a headline or a property drawer must not
//! become one here.
//!
//! The rules follow the Org 9.8 manual:
//! <https://orgmode.org/manual/>.

use crate::model::TODO_KEYWORDS;

/// Keywords Org writes on the planning line, in the order Org writes them.
pub const PLANNING_KEYS: &[&str] = &["CLOSED", "SCHEDULED", "DEADLINE"];

/// Whether `c` may appear in an Org tag (`[[:alnum:]_@#%]+`).
pub fn is_org_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '@' | '#' | '%')
}

/// A line Org reads as a headline: one or more stars at column 0, then a space.
///
/// Manual 2.1. A line that merely *looks* starred (`**bold**`) is not one.
/// Leading whitespace takes the line off the left margin, so it is not one
/// either.
pub fn is_headline(line: &str) -> bool {
    let stars = line.len() - line.trim_start_matches('*').len();
    stars > 0 && line[stars..].starts_with(' ')
}

/// A level-one headline: `* ` at column 0. That is an issue site.
pub fn is_top_level_headline(line: &str) -> bool {
    line.starts_with("* ")
}

/// `:NAME:` alone on a line, which is how every drawer opens (manual 2.7).
pub fn opens_a_drawer(trimmed: &str) -> bool {
    trimmed.len() > 2
        && trimmed.starts_with(':')
        && trimmed.ends_with(':')
        && !trimmed.eq_ignore_ascii_case(":END:")
        && !trimmed[1..trimmed.len() - 1].contains(char::is_whitespace)
}

/// Nesting of greater blocks (`#+BEGIN_SRC` … `#+END_SRC`) and dynamic
/// blocks (`#+BEGIN: clocktable` … `#+END:`).
///
/// Manual 2.8, 12.6, 16.2. Content inside a block is literal: a line that
/// looks like a headline or a drawer is not one.
#[derive(Debug, Default, Clone)]
pub struct BlockNest {
    depth: usize,
}

impl BlockNest {
    /// Empty nest, at file (or heading) scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a previous line opened a block that has not yet closed.
    pub fn inside(&self) -> bool {
        self.depth > 0
    }

    /// Observe `line`. Returns true when the line is part of a block,
    /// including the `#+BEGIN` / `#+END` lines themselves.
    pub fn observe(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        if is_block_end(trimmed) {
            self.depth = self.depth.saturating_sub(1);
            return true;
        }
        if is_block_begin(trimmed) {
            self.depth += 1;
            return true;
        }
        self.depth > 0
    }
}

/// Walk that hides both greater/dynamic blocks and Org Babel result
/// regions (manual 16).
///
/// vissue never evaluates a source block. It only has to recognise the
/// syntax Babel writes, so a `#+RESULTS:` payload is not an issue and
/// does not define an `:ID:`.
#[derive(Debug, Default, Clone)]
pub struct OrgScan {
    blocks: BlockNest,
    results: ResultsState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum ResultsState {
    #[default]
    Out,
    /// Just saw `#+RESULTS:`; the next element is the payload.
    Awaiting,
    /// The payload is a greater or dynamic block.
    ViaBlock,
    Drawer,
    Table,
    FixedWidth,
    List,
    Headline {
        stars: usize,
    },
}

impl OrgScan {
    /// Empty scan, at file (or heading) scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the previous line left us inside a block or a results payload.
    pub fn inside(&self) -> bool {
        self.blocks.inside() || !matches!(self.results, ResultsState::Out)
    }

    /// Observe `line`. Returns true when the line is not document
    /// structure: it belongs to a block or to a Babel results element.
    pub fn observe(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();

        if self.blocks.inside() || is_block_end(trimmed) {
            let in_block = self.blocks.observe(line);
            if !self.blocks.inside() && matches!(self.results, ResultsState::ViaBlock) {
                self.results = ResultsState::Out;
            }
            return in_block || matches!(self.results, ResultsState::ViaBlock);
        }

        if is_results_keyword(line.trim()) {
            self.results = ResultsState::Awaiting;
            return true;
        }

        if is_block_begin(trimmed) {
            if matches!(self.results, ResultsState::Awaiting) {
                self.results = ResultsState::ViaBlock;
            }
            return self.blocks.observe(line);
        }

        match self.results {
            ResultsState::Out => false,
            ResultsState::ViaBlock => {
                self.results = ResultsState::Out;
                false
            }
            ResultsState::Awaiting => {
                if line.trim().is_empty() {
                    return true;
                }
                self.start_result_element(line)
            }
            ResultsState::Drawer => {
                if line.trim().eq_ignore_ascii_case(":END:") {
                    self.results = ResultsState::Out;
                }
                true
            }
            ResultsState::Table => {
                if is_org_table_line(line.trim()) {
                    true
                } else {
                    self.results = ResultsState::Out;
                    self.observe(line)
                }
            }
            ResultsState::FixedWidth => {
                if is_fixed_width_line(line) {
                    true
                } else {
                    self.results = ResultsState::Out;
                    self.observe(line)
                }
            }
            ResultsState::List => {
                if line.trim().is_empty() {
                    self.results = ResultsState::Out;
                    return true;
                }
                if is_org_list_line(line) || is_list_continuation(line) {
                    true
                } else {
                    self.results = ResultsState::Out;
                    self.observe(line)
                }
            }
            ResultsState::Headline { stars } => {
                if is_headline(line) {
                    let n = headline_stars(line);
                    if n > 0 && n <= stars {
                        self.results = ResultsState::Out;
                        return self.observe(line);
                    }
                }
                true
            }
        }
    }

    fn start_result_element(&mut self, line: &str) -> bool {
        let trimmed = line.trim();
        if opens_a_drawer(trimmed) {
            self.results = ResultsState::Drawer;
            return true;
        }
        if is_org_table_line(trimmed) {
            self.results = ResultsState::Table;
            return true;
        }
        if is_fixed_width_line(line) {
            self.results = ResultsState::FixedWidth;
            return true;
        }
        if is_org_list_line(line) {
            self.results = ResultsState::List;
            return true;
        }
        if is_headline(line) {
            self.results = ResultsState::Headline {
                stars: headline_stars(line),
            };
            return true;
        }
        // A file link, a scalar paragraph, or anything else Babel dumps as
        // one element: this line is the payload.
        self.results = ResultsState::Out;
        true
    }
}

/// `#+RESULTS:` / `#+RESULTS[hash]:` / `#+RESULTS: name` (manual 16.6).
pub fn is_results_keyword(trimmed: &str) -> bool {
    let Some(rest) = strip_hash_plus(trimmed.trim()) else {
        return false;
    };
    let Some(after) =
        strip_keyword_prefix(rest, "RESULTS").or_else(|| strip_keyword_prefix(rest, "RESULT"))
    else {
        return false;
    };
    let after = after.trim_start();
    if after.starts_with(':') {
        return true;
    }
    if after.starts_with('[') {
        return after.contains(':');
    }
    false
}

/// `#+CALL: name(...)` (manual 16.5 / Library of Babel).
pub fn is_babel_call(trimmed: &str) -> bool {
    let Some(rest) = strip_hash_plus(trimmed.trim()) else {
        return false;
    };
    strip_keyword_prefix(rest, "CALL").is_some_and(|after| after.starts_with(':'))
}

/// Affiliated keyword that binds to the next element (manual 16.3, org-element).
pub fn is_affiliated_keyword(trimmed: &str) -> bool {
    let Some(rest) = strip_hash_plus(trimmed.trim()) else {
        return false;
    };
    let Some((key, _)) = rest.split_once(':') else {
        return false;
    };
    let key = key.trim();
    if starts_ignore_ascii(key, "ATTR_") {
        return true;
    }
    matches!(
        key.to_ascii_uppercase().as_str(),
        "CAPTION"
            | "DATA"
            | "HEADER"
            | "HEADERS"
            | "LABEL"
            | "NAME"
            | "PLOT"
            | "RESNAME"
            | "RESULT"
            | "RESULTS"
            | "SOURCE"
            | "SRCNAME"
            | "TBLNAME"
    )
}

/// The `#+BEGIN_SRC lang switches :headers` line (manual 16.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrcBlockHead<'a> {
    /// Language token, `python` or `org`.
    pub lang: &'a str,
    /// Switches such as `-n -r`.
    pub switches: &'a str,
    /// Header arguments, starting at the first `:key`.
    pub headers: &'a str,
}

/// Parse a source-block opening line. Other greater blocks yield `None`.
pub fn parse_src_begin(line: &str) -> Option<SrcBlockHead<'_>> {
    let rest = strip_hash_plus(line.trim_start())?;
    let after = strip_keyword_prefix(rest, "BEGIN_SRC")?;
    let after = after.trim_start();
    if after.is_empty() {
        return None;
    }
    let (lang, rest) = first_word(after).unwrap_or((after, ""));
    let rest = rest.trim_start();
    let (switches, headers) = match rest.find(':') {
        Some(i) => (rest[..i].trim(), rest[i..].trim()),
        None => (rest.trim(), ""),
    };
    Some(SrcBlockHead {
        lang,
        switches,
        headers,
    })
}

/// `:key value` pairs from a header-args string (manual 16.3).
pub fn parse_header_args(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while let Some(idx) = rest.find(':') {
        rest = rest[idx + 1..].trim_start();
        if rest.is_empty() {
            break;
        }
        let (key, after) = match rest.find(char::is_whitespace) {
            Some(i) => (&rest[..i], rest[i..].trim_start()),
            None => (rest, ""),
        };
        if key.is_empty() {
            break;
        }
        let (value, next) = next_header_value(after);
        out.push((key.to_string(), value.to_string()));
        rest = next;
    }
    out
}

fn next_header_value(s: &str) -> (&str, &str) {
    if s.is_empty() || s.starts_with(':') {
        return ("", s);
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            break;
        }
        i += 1;
    }
    // Back up so we split on a char boundary.
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    (s[..i].trim(), s[i..].trim_start())
}

/// Noweb references `<<name>>` / `<<name(args)>>` (manual 16.11).
pub fn noweb_refs(body: &str) -> Vec<&str> {
    let mut refs = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("<<") {
        let after = &rest[start + 2..];
        let Some(end) = after.find(">>") else {
            break;
        };
        let inner = after[..end].trim();
        if !inner.is_empty() && !inner.contains('\n') && !refs.contains(&inner) {
            refs.push(inner);
        }
        rest = &after[end + 2..];
    }
    refs
}

/// Inline `src_lang{body}` / `src_lang[headers]{body}` (manual 16.2).
pub fn inline_src_spans(text: &str) -> Vec<(&str, &str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find("src_") {
        let after = &rest[idx + 4..];
        let lang_len = after
            .find(|c: char| c.is_whitespace() || c == '[' || c == '{')
            .unwrap_or(after.len());
        if lang_len == 0 {
            rest = &after[1.min(after.len())..];
            continue;
        }
        let lang = &after[..lang_len];
        let mut tail = &after[lang_len..];
        let mut headers = "";
        if let Some(inner) = tail.strip_prefix('[') {
            let Some(end) = inner.find(']') else {
                rest = tail;
                continue;
            };
            headers = &inner[..end];
            tail = &inner[end + 1..];
        }
        let Some(inner) = tail.strip_prefix('{') else {
            rest = tail;
            continue;
        };
        let Some(end) = inner.find('}') else {
            rest = tail;
            continue;
        };
        out.push((lang, headers, &inner[..end]));
        rest = &inner[end + 1..];
    }
    out
}

/// Inline `call_name(args)` / `call_name[hdr](args)` (manual 16.5).
pub fn inline_call_names(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find("call_") {
        let after = &rest[idx + 5..];
        let name_len = after
            .find(|c: char| c.is_whitespace() || c == '[' || c == '(')
            .unwrap_or(after.len());
        if name_len == 0 {
            rest = &after[1.min(after.len())..];
            continue;
        }
        let name = &after[..name_len];
        let tail = &after[name_len..];
        if tail.starts_with('(') || tail.starts_with('[') {
            out.push(name);
        }
        rest = tail;
    }
    out
}

fn strip_hash_plus(trimmed: &str) -> Option<&str> {
    trimmed.strip_prefix("#+")
}

fn strip_keyword_prefix<'a>(s: &'a str, keyword: &str) -> Option<&'a str> {
    if s.len() >= keyword.len()
        && s.is_char_boundary(keyword.len())
        && s[..keyword.len()].eq_ignore_ascii_case(keyword)
    {
        Some(&s[keyword.len()..])
    } else {
        None
    }
}

fn headline_stars(line: &str) -> usize {
    line.len() - line.trim_start_matches('*').len()
}

fn is_org_table_line(trimmed: &str) -> bool {
    trimmed.starts_with('|')
}

fn is_fixed_width_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    matches!(
        trimmed.as_bytes(),
        [b':'] | [b':', b' ', ..] | [b':', b'\t', ..]
    ) && !opens_a_drawer(trimmed)
        && !trimmed.eq_ignore_ascii_case(":END:")
}

fn is_org_list_line(line: &str) -> bool {
    if is_headline(line) {
        return false;
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with("- ") || trimmed.starts_with("+ ") {
        return true;
    }
    let Some((token, rest)) = first_word(trimmed) else {
        return false;
    };
    let rest = rest.trim_start();
    if rest.is_empty() && !token.ends_with('.') && !token.ends_with(')') {
        return false;
    }
    let bare = token.trim_end_matches(['.', ')']);
    if bare.is_empty() || bare == token {
        return false;
    }
    bare.chars().all(|c| c.is_ascii_digit())
        || (bare.len() == 1 && bare.chars().all(|c| c.is_ascii_alphabetic()))
}

fn is_list_continuation(line: &str) -> bool {
    !is_headline(line)
        && (line.starts_with(' ') || line.starts_with('\t'))
        && !line.trim().is_empty()
}

fn is_block_begin(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("#+") else {
        return false;
    };
    starts_ignore_ascii(rest, "BEGIN_") || starts_ignore_ascii(rest, "BEGIN:")
}

fn is_block_end(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("#+") else {
        return false;
    };
    starts_ignore_ascii(rest, "END_") || starts_ignore_ascii(rest, "END:")
}

fn starts_ignore_ascii(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// File-local TODO keywords from `#+TODO:` lines, plus the house set.
///
/// Manual 5.2.5. Fast-access keys (`TODO(t)`, `WAIT(w@)`) are stripped.
/// Several `#+TODO:` lines accumulate. The house keywords stay recognised
/// so a preamble that only lists a subset does not drop STARTED headings.
pub fn todo_keywords_from_preamble(preamble: &str) -> Vec<String> {
    todo_keywords_from_lines(&preamble.lines().collect::<Vec<_>>())
}

/// The file's TODO sequence as Org reads it: the keywords before the `|`
/// are open, the ones after it are done; a line with no `|` makes its last
/// keyword the done one (Org manual 5.2.1). The house five are always in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoSequence {
    /// Keywords an issue can still be worked under.
    pub open: Vec<String>,
    /// Keywords that close an issue.
    pub done: Vec<String>,
}

impl Default for TodoSequence {
    fn default() -> Self {
        Self::house()
    }
}

impl TodoSequence {
    /// The house five alone.
    #[must_use]
    pub fn house() -> Self {
        Self {
            open: ["TODO", "STARTED", "BLOCKED"].map(str::to_string).to_vec(),
            done: ["DONE", "CANCELLED"].map(str::to_string).to_vec(),
        }
    }

    /// Whether `state` is a keyword this file declares, either side.
    #[must_use]
    pub fn knows(&self, state: &str) -> bool {
        self.open.iter().any(|k| k == state) || self.done.iter().any(|k| k == state)
    }

    /// Whether `state` closes an issue in this file.
    #[must_use]
    pub fn is_done(&self, state: &str) -> bool {
        self.done.iter().any(|k| k == state)
    }

    /// Every keyword, open ones first, for a message.
    #[must_use]
    pub fn all(&self) -> Vec<String> {
        self.open.iter().chain(self.done.iter()).cloned().collect()
    }
}

/// The TODO sequence of a file from its `#+TODO:`, `#+SEQ_TODO:` and
/// `#+TYP_TODO:` lines (Org manual 5.2.4), over the house five.
#[must_use]
pub fn todo_sequence_from_lines(lines: &[&str]) -> TodoSequence {
    let mut seq = TodoSequence::house();
    for line in lines {
        let trimmed = line.trim();
        let Some(rest) = ["TODO", "SEQ_TODO", "TYP_TODO"]
            .iter()
            .find_map(|name| strip_file_keyword(trimmed, name))
        else {
            continue;
        };
        let names: Vec<&str> = rest
            .split_whitespace()
            .map(|token| token.split('(').next().unwrap_or(token))
            .filter(|name| !name.is_empty())
            .collect();
        let bar = names.iter().position(|n| *n == "|");
        // No bar: Org takes the last keyword as the done one.
        let done_from = bar.map_or(names.len().saturating_sub(1), |b| b + 1);
        for (i, name) in names.iter().enumerate() {
            if *name == "|" || seq.knows(name) {
                continue;
            }
            if i >= done_from {
                seq.done.push((*name).to_string());
            } else {
                seq.open.push((*name).to_string());
            }
        }
    }
    seq
}

/// Whether an Org timestamp carries a repeater: `+1w`, `++1w`, `.+1w`
/// (Org manual 8.3.3).
#[must_use]
pub fn has_repeater(value: &str) -> bool {
    value
        .split_whitespace()
        .any(|tok| parse_repeater(tok.trim_end_matches('>')).is_some())
}

/// `(kind, count, unit)` of a repeater token, kind being `+`, `++` or `.+`.
fn parse_repeater(tok: &str) -> Option<(&str, u32, char)> {
    let (kind, rest) = if let Some(r) = tok.strip_prefix(".+") {
        (".+", r)
    } else if let Some(r) = tok.strip_prefix("++") {
        ("++", r)
    } else {
        ("+", tok.strip_prefix('+')?)
    };
    let unit = rest.chars().last()?;
    if !"hdwmy".contains(unit) {
        return None;
    }
    let count: u32 = rest[..rest.len() - 1].parse().ok()?;
    (count > 0).then_some((kind, count, unit))
}

fn add_interval(date: chrono::NaiveDate, count: u32, unit: char) -> Option<chrono::NaiveDate> {
    use chrono::{Days, Months};
    match unit {
        'd' => date.checked_add_days(Days::new(u64::from(count))),
        'w' => date.checked_add_days(Days::new(u64::from(count) * 7)),
        'm' => date.checked_add_months(Months::new(count)),
        'y' => date.checked_add_months(Months::new(count * 12)),
        // An hourly repeater on a date without a time has nothing to move.
        _ => Some(date),
    }
}

/// The timestamp after one repeat, as Org shifts it when a repeating task
/// is marked done (manual 8.3.3): `+` moves one interval on from the date
/// it holds, `++` moves by intervals until the date is past `today`, `.+`
/// moves one interval on from `today`. The day name follows the date;
/// time, repeater and warning stay as written. A timestamp without a
/// repeater, or a range, is `None`.
#[must_use]
pub fn shift_repeating_timestamp(value: &str, today: chrono::NaiveDate) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.contains("--") {
        return None;
    }
    let (open, close) = if trimmed.starts_with('<') && trimmed.ends_with('>') {
        ('<', '>')
    } else if trimmed.starts_with('[') && trimmed.ends_with(']') {
        ('[', ']')
    } else {
        return None;
    };
    let inner = &trimmed[1..trimmed.len() - 1];
    let tokens: Vec<&str> = inner.split_whitespace().collect();
    let date_tok = tokens.first()?;
    let mut date = chrono::NaiveDate::parse_from_str(date_tok, "%Y-%m-%d").ok()?;
    let mut rest: Vec<String> = Vec::new();
    let mut repeater = None;
    for tok in &tokens[1..] {
        if let Some(r) = parse_repeater(tok) {
            repeater = Some(r);
            rest.push((*tok).to_string());
        } else if tok.chars().all(|c| c.is_ascii_alphabetic())
            && tok.len() <= 3
            && repeater.is_none()
        {
            // The day name; recomputed below.
        } else {
            rest.push((*tok).to_string());
        }
    }
    let (kind, count, unit) = repeater?;
    date = match kind {
        "+" => add_interval(date, count, unit)?,
        "++" => {
            let mut next = add_interval(date, count, unit)?;
            let mut guard = 0;
            while next <= today && guard < 10_000 {
                next = add_interval(next, count, unit)?;
                guard += 1;
            }
            next
        }
        _ => add_interval(today, count, unit)?,
    };
    let mut out = format!("{open}{} {}", date.format("%Y-%m-%d"), date.format("%a"));
    for tok in rest {
        out.push(' ');
        out.push_str(&tok);
    }
    out.push(close);
    Some(out)
}

/// Same as [`todo_keywords_from_preamble`], from already-split lines.
pub fn todo_keywords_from_lines(lines: &[&str]) -> Vec<String> {
    let mut keywords: Vec<String> = TODO_KEYWORDS.iter().map(|s| (*s).to_string()).collect();
    for line in lines {
        let trimmed = line.trim();
        let Some(rest) = strip_file_keyword(trimmed, "TODO") else {
            continue;
        };
        for token in rest.split_whitespace() {
            if token == "|" {
                continue;
            }
            let name = token.split('(').next().unwrap_or(token);
            if name.is_empty() {
                continue;
            }
            if !keywords.iter().any(|k| k == name) {
                keywords.push(name.to_string());
            }
        }
    }
    keywords
}

/// Tags from `#+FILETAGS:` (manual 6 / in-buffer settings).
pub fn filetags_from_preamble(preamble: &str) -> Vec<String> {
    tag_settings_from_preamble(preamble).filetags
}

/// A declared tag on `#+TAGS:`, with an optional fast-selection key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSpec {
    /// Tag text, Org's `[[:alnum:]_@#%]+` or a `{regex}` group member.
    pub name: String,
    /// Fast tag selection key (`TAG(k)`).
    pub key: Option<char>,
}

/// File-level tag, export, and publish settings (manual 6, 13.2, 17.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSettings {
    /// Tags every heading inherits, as if from a hypothetical level 0.
    pub filetags: Vec<String>,
    /// Tags declared on `#+TAGS:` lines, in file order.
    pub declared: Vec<TagSpec>,
    /// Mutually exclusive groups `{ a b }`.
    pub exclusive: Vec<Vec<String>>,
    /// Group tag then members: `[ GTD : Control Persp ]`.
    pub hierarchies: Vec<(String, Vec<String>)>,
    /// `#+SELECT_TAGS:`; Org's default is `export`.
    pub select_tags: Vec<String>,
    /// `#+EXCLUDE_TAGS:`; Org's default is `noexport`.
    pub exclude_tags: Vec<String>,
}

impl Default for TagSettings {
    fn default() -> Self {
        Self {
            filetags: Vec::new(),
            declared: Vec::new(),
            exclusive: Vec::new(),
            hierarchies: Vec::new(),
            select_tags: vec!["export".into()],
            exclude_tags: vec!["noexport".into()],
        }
    }
}

impl TagSettings {
    /// Own tags plus inherited FILETAGS, which is Org's ALLTAGS for a
    /// level-one heading (manual 6.1). FILETAGS are not copied onto the
    /// heading on write.
    pub fn all_tags(&self, own: &[String]) -> Vec<String> {
        let mut tags = own.to_vec();
        for tag in &self.filetags {
            if !tags.iter().any(|seen| seen == tag) {
                tags.push(tag.clone());
            }
        }
        tags
    }

    /// Whether `needle` matches an own tag, an inherited FILETAGS tag, or
    /// a group tag whose members the heading carries (manual 6.3).
    pub fn matches_query(&self, own: &[String], needle: &str) -> bool {
        let needle_l = needle.to_lowercase();
        if needle_l.is_empty() {
            return false;
        }
        let all = self.all_tags(own);
        if all.iter().any(|tag| tag.to_lowercase().contains(&needle_l)) {
            return true;
        }
        for (group, members) in &self.hierarchies {
            if !group.to_lowercase().contains(&needle_l) {
                continue;
            }
            if members
                .iter()
                .any(|m| all.iter().any(|tag| tag.eq_ignore_ascii_case(m)))
            {
                return true;
            }
        }
        false
    }

    /// Whether this heading's own tags exclude it from Org export.
    ///
    /// FILETAGS `:noexport:` is a file-level publish signal and does not
    /// hide the heading from a vissue mirror. A heading tagged `noexport`
    /// (or another `#+EXCLUDE_TAGS:` token) is dropped. `noexport` wins
    /// over `export` (manual 13.2).
    pub fn heading_exportable(&self, own: &[String]) -> bool {
        !own.iter().any(|tag| {
            self.exclude_tags
                .iter()
                .any(|ex| ex.eq_ignore_ascii_case(tag))
        })
    }
}

/// Parse `#+FILETAGS:`, `#+TAGS:`, `#+SELECT_TAGS:`, `#+EXCLUDE_TAGS:`.
pub fn tag_settings_from_preamble(preamble: &str) -> TagSettings {
    let mut settings = TagSettings::default();
    let mut saw_select = false;
    let mut saw_exclude = false;
    for line in preamble.lines() {
        let trimmed = line.trim();
        if let Some(rest) = strip_file_keyword(trimmed, "FILETAGS") {
            for tag in rest.trim().trim_matches(':').split(':') {
                let tag = tag.trim();
                if !tag.is_empty()
                    && tag.chars().all(is_org_tag_char)
                    && !settings.filetags.iter().any(|t| t == tag)
                {
                    settings.filetags.push(tag.to_string());
                }
            }
            continue;
        }
        if let Some(rest) = strip_file_keyword(trimmed, "TAGS") {
            apply_tags_line(&mut settings, rest);
            continue;
        }
        if let Some(rest) = strip_file_keyword(trimmed, "SELECT_TAGS") {
            settings.select_tags = split_keyword_tags(rest);
            saw_select = true;
            continue;
        }
        if let Some(rest) = strip_file_keyword(trimmed, "EXCLUDE_TAGS") {
            settings.exclude_tags = split_keyword_tags(rest);
            saw_exclude = true;
        }
    }
    if !saw_select {
        settings.select_tags = vec!["export".into()];
    }
    if !saw_exclude {
        settings.exclude_tags = vec!["noexport".into()];
    }
    settings
}

fn split_keyword_tags(rest: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for tag in rest
        .split(|c: char| c.is_whitespace() || c == ':' || c == ',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        if tag.chars().all(is_org_tag_char) && !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
    }
    tags
}

fn apply_tags_line(settings: &mut TagSettings, rest: &str) {
    let tokens = tokenize_tags_line(rest);
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "{" | "[" => {
                let exclusive = tokens[i] == "{";
                let closer = if exclusive { "}" } else { "]" };
                i += 1;
                let mut names: Vec<TagSpec> = Vec::new();
                let mut hierarchy_at = None;
                while i < tokens.len() && tokens[i] != closer {
                    if tokens[i] == ":" {
                        hierarchy_at = Some(names.len());
                        i += 1;
                        continue;
                    }
                    if let Some(spec) = parse_tag_token(&tokens[i]) {
                        names.push(spec);
                    }
                    i += 1;
                }
                if i < tokens.len() && tokens[i] == closer {
                    i += 1;
                }
                let names_only: Vec<String> = names.iter().map(|s| s.name.clone()).collect();
                if let Some(split) = hierarchy_at {
                    if split >= 1 {
                        let group = names[0].name.clone();
                        let members: Vec<String> = names_only.into_iter().skip(split).collect();
                        settings.hierarchies.push((group, members));
                    }
                } else if exclusive && names_only.len() >= 2 {
                    settings.exclusive.push(names_only);
                }
                for spec in names {
                    if !settings.declared.iter().any(|d| d.name == spec.name) {
                        settings.declared.push(spec);
                    }
                }
            }
            "\\n" => i += 1,
            other => {
                if let Some(spec) = parse_tag_token(other)
                    && !settings.declared.iter().any(|d| d.name == spec.name)
                {
                    settings.declared.push(spec);
                }
                i += 1;
            }
        }
    }
}

fn tokenize_tags_line(rest: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if matches!(c, '{' | '}' | '[' | ']' | ':') {
            tokens.push(c.to_string());
            i += 1;
            continue;
        }
        if c == '\\' && chars.get(i + 1) == Some(&'n') {
            tokens.push("\\n".into());
            i += 2;
            continue;
        }
        let start = i;
        while i < chars.len()
            && !chars[i].is_whitespace()
            && !matches!(chars[i], '{' | '}' | '[' | ']' | ':')
        {
            i += 1;
        }
        tokens.push(chars[start..i].iter().collect());
    }
    tokens
}

fn parse_tag_token(token: &str) -> Option<TagSpec> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if let Some(name) = token.strip_suffix(')')
        && let Some((name, key)) = name.rsplit_once('(')
    {
        let name = name.trim();
        let key = key.trim();
        if !name.is_empty() && name.chars().all(is_org_tag_char) && key.chars().count() == 1 {
            return Some(TagSpec {
                name: name.to_string(),
                key: key.chars().next(),
            });
        }
    }
    if token.chars().all(is_org_tag_char) {
        return Some(TagSpec {
            name: token.to_string(),
            key: None,
        });
    }
    None
}

/// House `#+TAGS:` lines: types are mutually exclusive; the rest are loose.
pub const HOUSE_TAGS_LINES: &[&str] = &[
    "#+TAGS: { bug(b) feature(f) task(t) chore(c) plan(p) }",
    "#+TAGS: docs(d) perf ignore ARCHIVE",
];

/// House `#+PRIORITIES:`: highest `A`, lowest `C`, default `C`.
///
/// Org's own default cookie is `B`. The tracker defaults to `C` so an
/// unprioritised heading is the lowest cookie, not the middle one.
pub const HOUSE_PRIORITIES_LINE: &str = "#+PRIORITIES: A C C";

/// On-disk `issues.org` contract. Independent of the crate version and of
/// the control-socket protocol.
///
/// 1 is the house Org shape: `#+CATEGORY:`, `#+FILETAGS:` with `noexport`,
/// the type `#+TAGS:` group, `#+PRIORITIES: A C C`, `#+SELECT_TAGS:` /
/// `#+EXCLUDE_TAGS:`, type as a heading tag, `:BLOCKED_BY:` as the graph,
/// and `:BLOCKER:` as org-edna (read, never minted).
pub const PROTOCOL_VERSION: u32 = 1;

/// In-buffer keyword that carries [`PROTOCOL_VERSION`].
pub const PROTOCOL_KEYWORD: &str = "VISSUE";

/// Protocol integer from `#+VISSUE:`, when the line parses.
pub fn protocol_from_preamble(preamble: &str) -> Option<u32> {
    for line in preamble.lines() {
        if let Some(n) = protocol_from_keyword_line(line) {
            return Some(n);
        }
    }
    None
}

fn protocol_from_keyword_line(line: &str) -> Option<u32> {
    let rest = strip_file_keyword(line.trim(), PROTOCOL_KEYWORD)?;
    let mut parts = rest.split_whitespace();
    let first = parts.next()?;
    if first.eq_ignore_ascii_case("protocol") {
        parts.next()?.parse().ok()
    } else {
        first
            .strip_prefix("protocol=")
            .unwrap_or(first)
            .parse()
            .ok()
    }
}

fn protocol_stamp_line() -> String {
    format!("#+{PROTOCOL_KEYWORD}: {PROTOCOL_VERSION}")
}

/// File-local priority cookie range (`#+PRIORITIES: highest lowest default`).
///
/// Org requires the highest cookie to have a lower ASCII value than the
/// lowest (`A` before `C`). The third token is the default when a heading
/// has no `[#X]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrioritySpec {
    /// Highest priority cookie (`A` in the house file).
    pub highest: char,
    /// Lowest priority cookie (`C` in the house file).
    pub lowest: char,
    /// Cookie written when the heading has none.
    pub default: char,
}

impl Default for PrioritySpec {
    fn default() -> Self {
        Self {
            highest: 'A',
            lowest: 'C',
            default: 'C',
        }
    }
}

impl PrioritySpec {
    /// Whether `cookie` sits in this file's range, inclusive.
    pub fn contains(self, cookie: char) -> bool {
        let (lo, hi) = if self.highest <= self.lowest {
            (self.highest, self.lowest)
        } else {
            (self.lowest, self.highest)
        };
        cookie >= lo && cookie <= hi
    }
}

/// `#+PRIORITIES:` from the preamble, or `A C C`.
pub fn priorities_from_preamble(preamble: &str) -> PrioritySpec {
    for line in preamble.lines() {
        let Some(rest) = strip_file_keyword(line.trim(), "PRIORITIES") else {
            continue;
        };
        let mut toks = rest.split_whitespace();
        let (Some(h), Some(l), Some(d)) = (toks.next(), toks.next(), toks.next()) else {
            continue;
        };
        let (Some(highest), Some(lowest), Some(default)) =
            (h.chars().next(), l.chars().next(), d.chars().next())
        else {
            continue;
        };
        return PrioritySpec {
            highest,
            lowest,
            default,
        };
    }
    PrioritySpec::default()
}

/// A vissue `:ID:`: `{project}-{suffix}` with a `[0-9a-z]+` suffix.
///
/// `project` may contain `/` when the tracker lives in a nested directory
/// (`acme/cli-1a2b`). That slash is not an org-gcal event/calendar split.
pub fn is_vissue_id(id: &str) -> bool {
    let id = id.trim();
    let Some((project, suffix)) = id.rsplit_once('-') else {
        return false;
    };
    !project.is_empty()
        && !suffix.is_empty()
        && suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
}

/// An org-gcal event id (`<event>/<calendar>`), not an org-id / vissue id.
///
/// A nested-project vissue id contains `/` because the project name does
/// (`acme/cli-1a2b`). That is still `{project}-{suffix}`, not an event
/// and a calendar.
pub fn is_gcal_event_id(id: &str) -> bool {
    let id = id.trim();
    if is_vissue_id(id) {
        return false;
    }
    let Some((left, right)) = id.split_once('/') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && !left.contains(char::is_whitespace)
        && !right.contains(char::is_whitespace)
}

/// Org treats a non-empty property other than `nil` / `0` as true.
pub fn org_property_is_set(
    properties: &std::collections::BTreeMap<String, String>,
    key: &str,
) -> bool {
    properties.get(key).is_some_and(|raw| {
        let v = raw.trim();
        !v.is_empty() && !v.eq_ignore_ascii_case("nil") && v != "0"
    })
}

/// In-buffer settings from `#+SETUPFILE:` plus the file's own preamble.
///
/// Local files only. A URL is left unread. Cycles and a missing file are
/// skipped so a tracker still parses.
pub fn merge_setupfile_settings(preamble: &str, base_dir: Option<&std::path::Path>) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut out = String::new();
    collect_setupfile_settings(preamble, base_dir, &mut seen, 0, &mut out);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(preamble);
    out
}

fn collect_setupfile_settings(
    text: &str,
    base_dir: Option<&std::path::Path>,
    seen: &mut std::collections::HashSet<std::path::PathBuf>,
    depth: u8,
    out: &mut String,
) {
    if depth > 16 {
        return;
    }
    for line in text.lines() {
        let Some(rest) = strip_file_keyword(line.trim(), "SETUPFILE") else {
            continue;
        };
        let spec = rest.trim().trim_matches('"').trim_matches('\'').trim();
        if spec.is_empty()
            || spec.contains("://")
            || spec.starts_with("http:")
            || spec.starts_with("https:")
        {
            continue;
        }
        let path = match base_dir {
            Some(dir) => dir.join(spec),
            None => std::path::PathBuf::from(spec),
        };
        let key = path.canonicalize().unwrap_or(path.clone());
        if !seen.insert(key) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        let keywords: String = body
            .lines()
            .filter(|l| l.trim_start().starts_with("#+"))
            .collect::<Vec<_>>()
            .join("\n");
        if !keywords.is_empty() {
            out.push_str(&keywords);
            out.push('\n');
        }
        collect_setupfile_settings(&keywords, path.parent(), seen, depth + 1, out);
    }
}

/// Whether the preamble already carries `#+NAME:`.
pub fn preamble_has_keyword(preamble: &str, name: &str) -> bool {
    preamble
        .lines()
        .any(|line| strip_file_keyword(line.trim(), name).is_some())
}

/// Insert the house in-buffer settings a hand-started file never grew.
///
/// Org takes the category from the file name otherwise, and every
/// project's file is `issues.org`. A missing `#+TAGS:` means Emacs
/// fast-tag selection has no type group. `#+FILETAGS:` includes
/// `noexport` so a vault publish project skips the tracker (manual 13.2,
/// 14). A FILETAGS line that already exists but has no `noexport` gets
/// that tag appended.
pub fn ensure_org_preamble(preamble: &str, project: &str) -> String {
    if preamble.trim().is_empty() {
        return preamble.to_string();
    }
    let mut lines: Vec<String> = preamble.lines().map(str::to_string).collect();
    let insert_at = lines
        .iter()
        .position(|line| strip_file_keyword(line.trim(), "TITLE").is_some())
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut extra = Vec::new();
    if !preamble_has_keyword(preamble, "CATEGORY") {
        extra.push(format!("#+CATEGORY: {project}"));
    }
    if !preamble_has_keyword(preamble, "FILETAGS") {
        extra.push(format!("#+FILETAGS: :issues:{project}:noexport:"));
    }
    if !preamble_has_keyword(preamble, "TAGS") {
        extra.extend(HOUSE_TAGS_LINES.iter().map(|s| (*s).to_string()));
    }
    if !preamble_has_keyword(preamble, "EXCLUDE_TAGS") {
        extra.push("#+EXCLUDE_TAGS: noexport".to_string());
    }
    if !preamble_has_keyword(preamble, "SELECT_TAGS") {
        extra.push("#+SELECT_TAGS: export".to_string());
    }
    if !preamble_has_keyword(preamble, "PRIORITIES") {
        extra.push(HOUSE_PRIORITIES_LINE.to_string());
    }
    for (offset, line) in extra.into_iter().enumerate() {
        lines.insert(insert_at + offset, line);
    }
    ensure_filetags_has_noexport(&mut lines);
    ensure_protocol_stamp(&mut lines);
    let out = lines.join("\n");
    if out == preamble {
        preamble.to_string()
    } else {
        out
    }
}

fn ensure_protocol_stamp(lines: &mut Vec<String>) {
    let stamp = protocol_stamp_line();
    for line in lines.iter_mut() {
        if strip_file_keyword(line.trim(), PROTOCOL_KEYWORD).is_none() {
            continue;
        }
        match protocol_from_keyword_line(line) {
            Some(n) if n >= PROTOCOL_VERSION => {}
            _ => *line = stamp,
        }
        return;
    }
    let insert_at = lines
        .iter()
        .position(|line| strip_file_keyword(line.trim(), "TITLE").is_some())
        .map(|i| i + 1)
        .unwrap_or(0);
    lines.insert(insert_at, stamp);
}

fn ensure_filetags_has_noexport(lines: &mut [String]) {
    for line in lines.iter_mut() {
        let Some(rest) = strip_file_keyword(line.trim(), "FILETAGS") else {
            continue;
        };
        let tags: Vec<&str> = rest
            .trim()
            .trim_matches(':')
            .split(':')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect();
        if tags.iter().any(|t| t.eq_ignore_ascii_case("noexport")) {
            return;
        }
        let mut all = tags;
        all.push("noexport");
        *line = format!("#+FILETAGS: :{}:", all.join(":"));
        return;
    }
}

/// Move classifiers Org can hold onto the heading: a legal `:TYPE:` and
/// any legal token in `:VISSUE_TAGS:`. Hyphenated leftovers stay in the
/// property. `:TYPE:` itself is kept so export and `--type` filters still
/// read it.
pub fn settle_heading_classifiers(
    org_tags: &mut Vec<String>,
    properties: &mut std::collections::BTreeMap<String, String>,
) {
    fn push_tag(org_tags: &mut Vec<String>, tag: &str) {
        if !tag.is_empty()
            && tag.chars().all(is_org_tag_char)
            && !org_tags.iter().any(|seen| seen == tag)
        {
            org_tags.push(tag.to_string());
        }
    }
    for key in ["VISSUE_TYPE", "TYPE"] {
        if let Some(kind) = properties.get(key) {
            push_tag(org_tags, kind.trim());
        }
    }
    if let Some(raw) = properties.get(crate::model::TAGS_PROPERTY).cloned() {
        let mut kept = Vec::new();
        for tag in raw
            .split([',', ':'])
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            if tag.chars().all(is_org_tag_char) {
                push_tag(org_tags, tag);
            } else {
                kept.push(tag.to_string());
            }
        }
        if kept.is_empty() {
            properties.remove(crate::model::TAGS_PROPERTY);
        } else {
            properties.insert(crate::model::TAGS_PROPERTY.to_string(), kept.join(","));
        }
    }
}

/// Org specials that are computed. Writing them in a drawer does not
/// set them; Org reads the headline, the planning line, or the clock.
pub const COMPUTED_SPECIALS: &[&str] = &[
    "ALLTAGS",
    "BLOCKED",
    "CLOCKSUM",
    "CLOCKSUM_T",
    "FILE",
    "ITEM",
    "PRIORITY",
    "TAGS",
    "TIMESTAMP",
    "TIMESTAMP_IA",
    "TODO",
];

/// Org specials a heading may set. `CATEGORY` and `ARCHIVE` are the
/// ones the agenda actually honours from the drawer.
pub const SETTABLE_SPECIALS: &[&str] = &[
    "ARCHIVE",
    "CATEGORY",
    "COLUMNS",
    "COOKIE_DATA",
    "LOGGING",
    "ORDERED",
    "STYLE",
];

/// Words org-edna and org-depend put in `:BLOCKER:` / `:TRIGGER:`.
const EDNA_ATOMS: &[&str] = &[
    "ancestors",
    "chain-siblings",
    "children",
    "descendants",
    "file-progress",
    "first-child",
    "has-property",
    "heading",
    "headings",
    "id",
    "ids",
    "last-child",
    "match",
    "next-sibling",
    "olp",
    "parent",
    "prev-sibling",
    "previous-sibling",
    "relatives",
    "rest-of-siblings",
    "siblings",
    "todo-state",
    "todo-state!",
];

/// Split a BLOCKED_BY-style id list: commas and whitespace both separate.
pub fn split_id_list(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether a `:BLOCKER:` value is org-edna / org-depend syntax, not a
/// list of issue ids. GNU ELPA org-edna is the maintained package;
/// org-depend in org-contrib is the older one. Both own this name.
pub fn is_edna_blocker(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.contains('(') {
        return true;
    }
    trimmed.split_whitespace().any(|tok| {
        let atom = tok.trim_end_matches('!');
        EDNA_ATOMS
            .iter()
            .any(|known| atom.eq_ignore_ascii_case(known))
    })
}

/// Issue ids mentioned in an org-edna `ids(...)` / `id(...)` form.
pub fn edna_blocker_id_refs(raw: &str) -> Vec<&str> {
    let mut ids = Vec::new();
    let mut rest = raw;
    while let Some(start) = rest.find('(') {
        let Some(end) = rest[start + 1..].find(')') else {
            break;
        };
        let inner = &rest[start + 1..start + 1 + end];
        for id in inner.split(|c: char| c == ',' || c.is_whitespace()) {
            let id = id.trim();
            if !id.is_empty() && !id.contains('"') && !ids.contains(&id) {
                ids.push(id);
            }
        }
        rest = &rest[start + 1 + end + 1..];
    }
    ids
}

/// Issue ids mentioned in an org-edna `ids(...)` / `id(...)` form.
pub fn edna_blocker_ids(raw: &str) -> Vec<String> {
    edna_blocker_id_refs(raw)
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Every blocker id a heading declares: `:BLOCKED_BY:`, a typo
/// `:BLOCKEDBY:`, a `:BLOCKER:` that is just ids, and `ids(...)` inside
/// an org-edna form.
pub fn blocker_ids_from_properties(
    properties: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let mut ids = Vec::new();
    for key in ["VISSUE_BLOCKED_BY", "BLOCKED_BY", "BLOCKEDBY"] {
        if let Some(raw) = properties.get(key) {
            for id in split_id_list(raw) {
                if !ids.iter().any(|seen| seen == &id) {
                    ids.push(id);
                }
            }
        }
    }
    if let Some(raw) = properties.get("BLOCKER") {
        let extra = if is_edna_blocker(raw) {
            edna_blocker_ids(raw)
        } else {
            split_id_list(raw)
        };
        for id in extra {
            if !ids.iter().any(|seen| seen == &id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Effort value Org's column view and agenda effort filter read.
/// `org-effort-property` defaults to `Effort`.
pub fn effort_from_properties(
    properties: &std::collections::BTreeMap<String, String>,
) -> Option<&str> {
    properties
        .get("Effort")
        .or_else(|| properties.get("EFFORT"))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

/// A duration Org accepts for Effort: `1:30`, `2h`, `3d`, `0:10`.
pub fn is_org_effort(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() {
        return false;
    }
    if let Some((h, m)) = s.split_once(':') {
        return !h.is_empty()
            && h.chars().all(|c| c.is_ascii_digit())
            && !m.is_empty()
            && m.chars().all(|c| c.is_ascii_digit());
    }
    let (num, unit) = s.split_at(
        s.find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(s.len()),
    );
    !num.is_empty() && matches!(unit, "h" | "d" | "m" | "w" | "min" | "")
}

fn strip_file_keyword<'a>(trimmed: &'a str, name: &str) -> Option<&'a str> {
    let rest = trimmed.strip_prefix("#+")?;
    let (key, value) = rest.split_once(':')?;
    if key.eq_ignore_ascii_case(name) {
        Some(value)
    } else {
        None
    }
}

/// The pieces Org puts on a headline after the stars (manual 2.1, 5.1, 5.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadlineBits<'a> {
    /// TODO keyword, when the first word is one of `keywords`.
    pub keyword: Option<&'a str>,
    /// Priority cookie character, when `[#X]` follows the keyword.
    pub priority: Option<char>,
    /// Whether the heading carries the `COMMENT` keyword (manual 13.6).
    pub commented: bool,
    /// Title text, including a trailing tag run and statistics cookies.
    pub rest: &'a str,
}

/// Split the text after `* ` into keyword, priority, COMMENT, and title.
pub fn parse_headline_bits<'a>(after_stars: &'a str, keywords: &[String]) -> HeadlineBits<'a> {
    let trimmed = after_stars.trim();
    let mut rest = trimmed;
    let mut keyword = None;
    if let Some((word, after)) = first_word(rest)
        && is_listed_keyword(word, keywords)
    {
        keyword = Some(word);
        rest = after.trim_start();
    }
    let mut priority = None;
    if let Some((p, after)) = parse_priority_cookie(rest) {
        priority = Some(p);
        rest = after.trim_start();
    }
    let mut commented = false;
    if let Some((word, after)) = first_word(rest)
        && word == "COMMENT"
    {
        commented = true;
        rest = after.trim_start();
    }
    HeadlineBits {
        keyword,
        priority,
        commented,
        rest,
    }
}

fn first_word(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    match s.find(char::is_whitespace) {
        Some(i) => Some((&s[..i], &s[i..])),
        None => Some((s, "")),
    }
}

fn is_listed_keyword(word: &str, keywords: &[String]) -> bool {
    keywords.iter().any(|k| k == word)
}

/// A top-level heading that is an issue: recognised TODO keyword, not COMMENT.
pub fn is_issue_headline(line: &str, keywords: &[String]) -> bool {
    let Some(after) = line.strip_prefix("* ") else {
        return false;
    };
    let bits = parse_headline_bits(after, keywords);
    bits.keyword.is_some() && !bits.commented
}

/// Split a leading `[#A]` cookie off a heading. Any other shape yields `None`.
pub fn parse_priority_cookie(after: &str) -> Option<(char, &str)> {
    let rest = after.strip_prefix("[#")?;
    let mut chars = rest.char_indices();
    let (_, priority) = chars.next()?;
    let (close, bracket) = chars.next()?;
    if bracket != ']' {
        return None;
    }
    Some((priority, &rest[close + 1..]))
}

/// Split trailing statistics cookies (`[2/5]`, `[33%]`) off a title.
///
/// Manual 5.5. Cookies sit after the title and before the tag run.
pub fn split_statistics_cookies(text: &str) -> (String, Option<String>) {
    let mut trimmed = text.trim_end().to_string();
    let mut cookies = Vec::new();
    while let Some(open) = trimmed.rfind('[') {
        if !trimmed.ends_with(']') {
            break;
        }
        let cookie = &trimmed[open..];
        if !is_statistics_cookie(cookie) {
            break;
        }
        let prefix = trimmed[..open].trim_end();
        if open > 0 && !trimmed[..open].ends_with(char::is_whitespace) {
            break;
        }
        cookies.push(cookie.to_string());
        trimmed = prefix.to_string();
    }
    cookies.reverse();
    if cookies.is_empty() {
        (text.trim_end().to_string(), None)
    } else {
        (trimmed, Some(cookies.join(" ")))
    }
}

fn is_statistics_cookie(cookie: &str) -> bool {
    let Some(inner) = cookie.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return false;
    };
    // `[/]` and `[%]` are the empty cookies Org fills in (manual 5.5).
    if let Some((a, b)) = inner.split_once('/') {
        return a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit());
    }
    inner
        .strip_suffix('%')
        .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
}

/// Consume one Org timestamp or timestamp range at the start of `s`.
///
/// Manual 8.1: active `<>`, inactive `[]`, diary sexps `<%%(...)>`,
/// time ranges on one stamp, and ranges of two stamps joined by `--`.
/// Repeaters (`+1w`, `++1w`, `.+1w`) and warnings (`-2d`, `--2d`) live
/// inside the brackets, so the first closing delimiter is enough.
pub fn take_timestamp(s: &str) -> Option<(&str, &str)> {
    let start = s.trim_start();
    let leading = s.len() - start.len();
    if start.starts_with("<%%") {
        let end = start.find('>')?;
        let consumed = leading + end + 1;
        return Some((s[leading..consumed].trim_end(), s[consumed..].trim_start()));
    }
    let close = match start.chars().next()? {
        '<' => '>',
        '[' => ']',
        _ => return None,
    };
    let end = start.find(close)?;
    let mut consumed = leading + end + 1;
    let rest = &s[consumed..];
    if let Some(after) = rest.strip_prefix("--") {
        let after = after.trim_start();
        if after.starts_with('<') || after.starts_with('[') || after.starts_with("<%%") {
            let close2 = if after.starts_with('[') { ']' } else { '>' };
            let end2 = after.find(close2)?;
            consumed = s.len() - after.len() + end2 + 1;
        }
    }
    Some((s[leading..consumed].trim_end(), s[consumed..].trim_start()))
}

/// Read an Org planning line into `KEY -> timestamp` pairs.
///
/// Org packs several onto one line. A line holding anything else is not a
/// planning line at all, so a body sentence that opens on `DEADLINE:` stays
/// body.
pub fn parse_planning_line(line: &str) -> Vec<(String, String)> {
    let mut rest = line.trim();
    let mut found = Vec::new();
    while !rest.is_empty() {
        let Some(key) = PLANNING_KEYS
            .iter()
            .find(|key| rest.starts_with(&format!("{key}:")))
        else {
            return Vec::new();
        };
        let after = rest[key.len() + 1..].trim_start();
        let Some((ts, next)) = take_timestamp(after) else {
            return Vec::new();
        };
        found.push(((*key).to_string(), ts.to_string()));
        rest = next;
    }
    found
}

/// Whether `trimmed` opens a planning line (any planning key and a colon).
pub fn is_planning_line(trimmed: &str) -> bool {
    PLANNING_KEYS
        .iter()
        .any(|key| trimmed.starts_with(key) && trimmed[key.len()..].starts_with(':'))
}

/// Ids named by Org links in `body` that also sit in `known_ids`.
///
/// Manual 4.1 / 4.4: `[[id:foo]]`, `[[id:foo][desc]]`, `<id:foo>`, and a
/// bare `id:foo`. `[[foo]]` and `[[file:x.org::foo]]` still resolve when
/// the target or the search fragment is a known id.
pub fn org_link_targets(body: &str, known_ids: &std::collections::HashSet<&str>) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("]]") else {
            break;
        };
        let raw = &after_start[..end];
        let target = raw.split_once("][").map_or(raw, |(target, _)| target);
        push_link_target(&mut targets, target, known_ids);
        rest = &after_start[end + 2..];
    }
    rest = body;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            break;
        };
        push_link_target(&mut targets, &after[..end], known_ids);
        rest = &after[end + 1..];
    }
    rest = body;
    while let Some(start) = rest.find("id:") {
        let after = &rest[start + 3..];
        let len = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(after.len());
        let id = &after[..len];
        if !id.is_empty() && known_ids.contains(id) && !targets.iter().any(|t| t == id) {
            targets.push(id.to_string());
        }
        rest = &after[len.max(1)..];
    }
    targets
}

fn push_link_target(
    targets: &mut Vec<String>,
    raw: &str,
    known_ids: &std::collections::HashSet<&str>,
) {
    let target = raw.trim();
    let target = target.strip_prefix("id:").unwrap_or(target);
    let target = target.rsplit_once("::").map_or(target, |(_, fragment)| {
        fragment.strip_prefix('#').unwrap_or(fragment)
    });
    let target = target.strip_prefix('#').unwrap_or(target);
    if known_ids.contains(target) && !targets.iter().any(|t| t == target) {
        targets.push(target.to_string());
    }
}

/// Property key as written, with a trailing `+` (append) stripped.
///
/// Manual 7.1: `:var+: value` appends to `:var:`.
pub fn property_key_and_append(key: &str) -> (&str, bool) {
    match key.strip_suffix('+') {
        Some(bare) if !bare.is_empty() => (bare, true),
        _ => (key, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_todo_sequence_splits_at_the_bar_or_at_the_last_keyword() {
        let seq = todo_sequence_from_lines(&["#+TODO: TODO WAITING(w) | DONE WONTFIX"]);
        assert_eq!(seq.open, ["TODO", "STARTED", "BLOCKED", "WAITING"]);
        assert_eq!(seq.done, ["DONE", "CANCELLED", "WONTFIX"]);
        let bare = todo_sequence_from_lines(&["#+SEQ_TODO: NEXT FINISHED"]);
        assert!(bare.open.iter().any(|k| k == "NEXT"));
        assert!(bare.is_done("FINISHED"));
        assert!(TodoSequence::house().knows("CANCELLED"));
        assert!(!TodoSequence::house().knows("WAITING"));
    }

    #[test]
    fn a_repeater_shifts_as_org_shifts_it() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        assert_eq!(
            shift_repeating_timestamp("<2026-09-22 Tue +1w>", today).as_deref(),
            Some("<2026-09-29 Tue +1w>")
        );
        assert_eq!(
            shift_repeating_timestamp("<2026-09-01 Tue 10:00 ++1w -2d>", today).as_deref(),
            Some("<2026-09-22 Tue 10:00 ++1w -2d>")
        );
        assert_eq!(
            shift_repeating_timestamp("<2026-01-31 Sat .+1m>", today).as_deref(),
            Some("<2026-10-20 Tue .+1m>")
        );
        assert_eq!(
            shift_repeating_timestamp("<2026-01-31 Sat +1m>", today).as_deref(),
            Some("<2026-02-28 Sat +1m>")
        );
        assert!(shift_repeating_timestamp("<2026-09-22 Tue>", today).is_none());
        assert!(
            shift_repeating_timestamp("<2026-09-22 Tue>--<2026-09-23 Wed +1w>", today).is_none()
        );
        assert!(has_repeater("<2026-09-22 Tue +1w>"));
        assert!(!has_repeater("<2026-09-22 Tue -2d>"));
    }

    #[test]
    fn empty_statistics_cookies_are_cookies() {
        assert_eq!(
            split_statistics_cookies("Parent of two [/]"),
            ("Parent of two".to_string(), Some("[/]".to_string()))
        );
        assert_eq!(
            split_statistics_cookies("Half [%]").1.as_deref(),
            Some("[%]")
        );
        assert_eq!(split_statistics_cookies("Not a cookie [a/b]").1, None);
    }
    use std::collections::HashSet;

    fn house() -> Vec<String> {
        TODO_KEYWORDS.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_headline_needs_stars_then_a_space_at_column_zero() {
        assert!(is_headline("* TODO a title"));
        assert!(is_headline("*** deeper"));
        assert!(is_top_level_headline("* TODO a title"));
        assert!(!is_top_level_headline("** child"));
        assert!(!is_headline("**bold** at the start of a line"));
        assert!(!is_headline(" * indented is not a headline"));
        assert!(!is_headline("not a headline"));
    }

    #[test]
    fn greater_and_dynamic_blocks_hide_their_contents() {
        let mut nest = BlockNest::new();
        assert!(!nest.inside());
        assert!(nest.observe("#+BEGIN_SRC org"));
        assert!(nest.inside());
        assert!(nest.observe("* TODO quoted"));
        assert!(nest.observe("#+begin_example"));
        assert!(nest.observe("* still quoted"));
        assert!(nest.observe("#+end_example"));
        assert!(nest.inside());
        assert!(nest.observe("#+END_SRC"));
        assert!(!nest.inside());
        assert!(nest.observe("  #+BEGIN: clocktable :scope file"));
        assert!(nest.observe("* TODO inside clocktable"));
        assert!(nest.observe("  #+END:"));
        assert!(!nest.inside());
    }

    #[test]
    fn file_local_todo_keywords_accumulate_and_keep_the_house_set() {
        let keys = todo_keywords_from_preamble(
            "#+TITLE: x\n#+TODO: TODO(t) WAIT(w@) | DONE(d!)\n#+TODO: HOLD | CANCELLED\n",
        );
        for expected in [
            "TODO",
            "STARTED",
            "BLOCKED",
            "DONE",
            "CANCELLED",
            "WAIT",
            "HOLD",
        ] {
            assert!(
                keys.iter().any(|k| k == expected),
                "{keys:?} missing {expected}"
            );
        }
    }

    #[test]
    fn comment_and_section_headlines_are_not_issues() {
        let keys = house();
        assert!(is_issue_headline("* TODO Ship it", &keys));
        assert!(is_issue_headline("* DONE [#A] Ship it", &keys));
        assert!(!is_issue_headline("* COMMENT Archive", &keys));
        assert!(!is_issue_headline("* TODO COMMENT hidden", &keys));
        // Org's keyword is the upper-case word alone; a title that opens
        // with the word "comment" is a title.
        assert!(is_issue_headline("* TODO [#A] Comment :bug:", &keys));
        assert!(is_issue_headline("* TODO comment on the draft", &keys));
        assert!(!is_issue_headline("* Notes", &keys));
        assert!(!is_issue_headline("** TODO child", &keys));
    }

    #[test]
    fn a_file_local_keyword_is_an_issue() {
        let keys = todo_keywords_from_preamble("#+TODO: TODO WAIT | DONE\n");
        assert!(is_issue_headline("* WAIT Parked", &keys));
        assert!(!is_issue_headline("* HOLD Parked", &keys));
    }

    #[test]
    fn statistics_cookies_split_off_the_title() {
        assert_eq!(
            split_statistics_cookies("Break it down [2/5]"),
            ("Break it down".into(), Some("[2/5]".into()))
        );
        assert_eq!(
            split_statistics_cookies("Break it down [2/5] [40%]"),
            ("Break it down".into(), Some("[2/5] [40%]".into()))
        );
        assert_eq!(
            split_statistics_cookies("Array [2/3] leftover"),
            ("Array [2/3] leftover".into(), None)
        );
        assert_eq!(
            split_statistics_cookies("Not a cookie [n/a]"),
            ("Not a cookie [n/a]".into(), None)
        );
    }

    #[test]
    fn timestamps_include_ranges_repeaters_and_diary_sexps() {
        let (ts, rest) = take_timestamp("<2026-09-01 Tue +1w -2d> leftover").unwrap();
        assert_eq!(ts, "<2026-09-01 Tue +1w -2d>");
        assert_eq!(rest, "leftover");
        let (ts, rest) = take_timestamp("<2026-09-01 Tue>--<2026-09-08 Tue>").unwrap();
        assert_eq!(ts, "<2026-09-01 Tue>--<2026-09-08 Tue>");
        assert!(rest.is_empty());
        let (ts, _) = take_timestamp("[2026-09-01 Tue 09:00-17:00]").unwrap();
        assert_eq!(ts, "[2026-09-01 Tue 09:00-17:00]");
        let (ts, rest) = take_timestamp("<%%(diary-float t 4 2)> next").unwrap();
        assert_eq!(ts, "<%%(diary-float t 4 2)>");
        assert_eq!(rest, "next");
    }

    #[test]
    fn a_planning_line_keeps_a_range_and_rejects_prose() {
        let found = parse_planning_line(
            "CLOSED: [2026-08-14 Fri 03:33] SCHEDULED: <2026-09-01 Tue>--<2026-09-08 Tue> DEADLINE: <2026-09-15 Mon +1w>",
        );
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[1].1, "<2026-09-01 Tue>--<2026-09-08 Tue>");
        assert_eq!(found[2].1, "<2026-09-15 Mon +1w>");
        assert!(parse_planning_line("DEADLINE: is discussed in the design note.").is_empty());
    }

    #[test]
    fn org_links_include_brackets_angles_and_bare_ids() {
        let known: HashSet<&str> = ["atlas-1a2b", "beacon-5j6k"].into_iter().collect();
        let body =
            "See [[id:atlas-1a2b][the parser]] and <id:beacon-5j6k> plus id:atlas-1a2b again.";
        let found = org_link_targets(body, &known);
        assert_eq!(found, vec!["atlas-1a2b", "beacon-5j6k"]);
    }

    #[test]
    fn filetags_parse_the_in_buffer_keyword() {
        assert_eq!(
            filetags_from_preamble("#+FILETAGS: :issues:parser:\n"),
            vec!["issues", "parser"]
        );
    }

    #[test]
    fn tag_settings_parse_groups_and_keys() {
        let settings = tag_settings_from_preamble(
            "#+FILETAGS: :issues:demo:noexport:\n\
             #+TAGS: { bug(b) feature(f) task(t) }\n\
             #+TAGS: [ area : core cli ]\n\
             #+TAGS: docs(d) perf\n\
             #+EXCLUDE_TAGS: noexport\n\
             #+SELECT_TAGS: export\n",
        );
        assert_eq!(settings.filetags, vec!["issues", "demo", "noexport"]);
        assert_eq!(
            settings
                .declared
                .iter()
                .map(|s| (s.name.as_str(), s.key))
                .collect::<Vec<_>>(),
            vec![
                ("bug", Some('b')),
                ("feature", Some('f')),
                ("task", Some('t')),
                ("area", None),
                ("core", None),
                ("cli", None),
                ("docs", Some('d')),
                ("perf", None),
            ]
        );
        assert_eq!(
            settings.exclusive,
            vec![vec![
                "bug".to_string(),
                "feature".to_string(),
                "task".to_string()
            ]]
        );
        assert_eq!(
            settings.hierarchies,
            vec![("area".to_string(), vec!["core".into(), "cli".into()])]
        );
        let own = vec!["core".to_string(), "bug".to_string()];
        assert!(settings.matches_query(&own, "area"));
        assert!(settings.matches_query(&own, "issues"));
        assert!(settings.heading_exportable(&own));
        assert!(!settings.heading_exportable(&["noexport".into()]));
        assert_eq!(
            settings.all_tags(&own),
            vec!["core", "bug", "issues", "demo", "noexport"]
        );
    }

    #[test]
    fn ensure_org_preamble_inserts_category_and_filetags() {
        let raw = "#+TITLE: demo issues\n#+TODO: TODO | DONE";
        let out = ensure_org_preamble(raw, "demo");
        assert!(out.contains("#+VISSUE: 1"), "{out}");
        assert!(out.contains("#+CATEGORY: demo"), "{out}");
        assert!(out.contains("#+FILETAGS: :issues:demo:noexport:"), "{out}");
        assert!(
            out.contains("#+TAGS: { bug(b) feature(f) task(t) chore(c) plan(p) }"),
            "{out}"
        );
        assert!(out.contains("#+EXCLUDE_TAGS: noexport"), "{out}");
        assert!(out.contains("#+SELECT_TAGS: export"), "{out}");
        assert!(out.contains("#+PRIORITIES: A C C"), "{out}");
        assert!(out.find("#+TITLE:").unwrap() < out.find("#+CATEGORY:").unwrap());
        assert_eq!(ensure_org_preamble(&out, "demo"), out);
        let kept = "#+TITLE: demo issues\n#+FILETAGS: :issues:demo:\n#+TODO: TODO | DONE";
        let healed = ensure_org_preamble(kept, "demo");
        assert!(
            healed.contains("#+FILETAGS: :issues:demo:noexport:"),
            "existing FILETAGS gain noexport: {healed}"
        );
        let old = "#+TITLE: demo issues\n#+VISSUE: 0\n#+CATEGORY: demo\n";
        let bumped = ensure_org_preamble(old, "demo");
        assert!(bumped.contains("#+VISSUE: 1"), "{bumped}");
        assert!(!bumped.contains("#+VISSUE: 0"), "{bumped}");
        let future = "#+TITLE: demo issues\n#+VISSUE: 99\n#+CATEGORY: demo\n";
        let left = ensure_org_preamble(future, "demo");
        assert!(left.contains("#+VISSUE: 99"), "{left}");
    }

    #[test]
    fn protocol_from_preamble_reads_the_vissue_keyword() {
        assert_eq!(protocol_from_preamble("#+VISSUE: 1\n"), Some(1));
        assert_eq!(protocol_from_preamble("#+VISSUE: protocol 2\n"), Some(2));
        assert_eq!(protocol_from_preamble("#+VISSUE: protocol=3\n"), Some(3));
        assert_eq!(protocol_from_preamble("#+TITLE: x\n"), None);
    }

    #[test]
    fn priorities_from_preamble_reads_highest_lowest_default() {
        let spec = priorities_from_preamble("#+PRIORITIES: A D B\n");
        assert_eq!(spec.highest, 'A');
        assert_eq!(spec.lowest, 'D');
        assert_eq!(spec.default, 'B');
        assert!(spec.contains('C'));
        assert!(!spec.contains('E'));
        assert_eq!(priorities_from_preamble("").default, 'C');
    }

    #[test]
    fn gcal_event_ids_are_not_org_ids() {
        assert!(is_gcal_event_id("abc123/primary@group.calendar.google.com"));
        assert!(is_gcal_event_id("evt/cal"));
        assert!(is_gcal_event_id("abc/def/ghi"));
        assert!(!is_gcal_event_id("atlas-1a2b"));
        assert!(!is_gcal_event_id("no-slash"));
        assert!(!is_gcal_event_id("acme/cli-1a2b"));
        assert!(!is_gcal_event_id("acme/cli-consensus"));
        assert!(!is_gcal_event_id("acme/python-parseplot-yq4v"));
        assert!(is_vissue_id("acme/cli-1a2b"));
        assert!(is_vissue_id("atlas-1a2b"));
        assert!(!is_vissue_id("abc123/primary@group.calendar.google.com"));
        assert!(!is_vissue_id("evt/cal"));
    }

    #[test]
    fn setupfile_merges_local_inbuffer_settings() {
        let dir = tempfile::tempdir().unwrap();
        let setup = dir.path().join("house.org");
        std::fs::write(
            &setup,
            "#+TODO: TODO HOLD | DONE\n#+PRIORITIES: A D C\n* not a keyword\n",
        )
        .unwrap();
        let preamble = format!(
            "#+TITLE: x\n#+SETUPFILE: {}\n#+CATEGORY: x\n",
            setup.display()
        );
        let merged = merge_setupfile_settings(&preamble, Some(dir.path()));
        assert!(merged.contains("#+TODO: TODO HOLD | DONE"), "{merged}");
        assert!(merged.contains("#+PRIORITIES: A D C"), "{merged}");
        assert!(merged.contains("#+CATEGORY: x"), "{merged}");
        assert!(!merged.contains("* not a keyword"), "{merged}");
        assert_eq!(priorities_from_preamble(&merged).lowest, 'D');
    }

    #[test]
    fn edna_blocker_is_not_an_id_list() {
        assert!(is_edna_blocker("prev-sibling"));
        assert!(is_edna_blocker("ids(atlas-1a2b atlas-3e4f)"));
        assert!(is_edna_blocker("headings(\"Ship it\")"));
        assert!(!is_edna_blocker("atlas-1a2b"));
        assert!(!is_edna_blocker("atlas-1a2b beacon-5j6k"));
        assert_eq!(
            edna_blocker_ids("ids(atlas-1a2b atlas-3e4f) next-sibling"),
            vec!["atlas-1a2b", "atlas-3e4f"]
        );
        let mut props = std::collections::BTreeMap::new();
        props.insert("BLOCKER".into(), "atlas-1a2b atlas-3e4f".into());
        assert_eq!(
            blocker_ids_from_properties(&props),
            vec!["atlas-1a2b", "atlas-3e4f"]
        );
        let mut edna = std::collections::BTreeMap::new();
        edna.insert("BLOCKER".into(), "prev-sibling".into());
        assert!(blocker_ids_from_properties(&edna).is_empty());
    }

    #[test]
    fn effort_accepts_org_durations() {
        assert!(is_org_effort("1:30"));
        assert!(is_org_effort("2h"));
        assert!(is_org_effort("20d"));
        assert!(!is_org_effort("soon"));
        let mut props = std::collections::BTreeMap::new();
        props.insert("Effort".into(), "2h".into());
        assert_eq!(effort_from_properties(&props), Some("2h"));
    }

    #[test]
    fn settle_moves_legal_type_and_tags_onto_the_heading() {
        let mut tags = Vec::new();
        let mut props = std::collections::BTreeMap::new();
        props.insert("TYPE".into(), "bug".into());
        props.insert("VISSUE_TAGS".into(), "perf,needs-review".into());
        settle_heading_classifiers(&mut tags, &mut props);
        assert_eq!(tags, vec!["bug", "perf"]);
        assert_eq!(
            props.get("VISSUE_TAGS").map(String::as_str),
            Some("needs-review")
        );
        assert_eq!(props.get("TYPE").map(String::as_str), Some("bug"));
    }

    #[test]
    fn property_plus_appends() {
        assert_eq!(property_key_and_append("BLOCKED_BY+"), ("BLOCKED_BY", true));
        assert_eq!(property_key_and_append("ID"), ("ID", false));
    }

    #[test]
    fn results_keywords_match_what_babel_writes() {
        assert!(is_results_keyword("#+RESULTS:"));
        assert!(is_results_keyword("  #+results:"));
        assert!(is_results_keyword("#+RESULTS[deadbeef]:"));
        assert!(is_results_keyword(
            "#+RESULTS[(2026-08-18 17:50) abcdef]: named"
        ));
        assert!(is_results_keyword("#+RESULTS: named"));
        assert!(!is_results_keyword("#+RESULTANT:"));
        assert!(!is_results_keyword("#+TODO: TODO"));
    }

    #[test]
    fn babel_call_and_affiliated_keywords() {
        assert!(is_babel_call("#+CALL: plot(x=1) :results output"));
        assert!(is_babel_call("#+call: fn[:session]()"));
        assert!(!is_babel_call("#+CALLING:"));
        assert!(is_affiliated_keyword("#+NAME: plot"));
        assert!(is_affiliated_keyword("#+HEADER: :var x=1"));
        assert!(is_affiliated_keyword("#+ATTR_HTML: :width 40"));
        assert!(is_affiliated_keyword("#+TBLNAME: old"));
        assert!(!is_affiliated_keyword("#+TODO: TODO"));
    }

    #[test]
    fn src_begin_splits_lang_switches_and_headers() {
        let head = parse_src_begin("  #+BEGIN_SRC python -n -r :results output :var x=1").unwrap();
        assert_eq!(head.lang, "python");
        assert_eq!(head.switches, "-n -r");
        assert_eq!(head.headers, ":results output :var x=1");
        assert_eq!(
            parse_header_args(head.headers),
            vec![
                ("results".into(), "output".into()),
                ("var".into(), "x=1".into())
            ]
        );
    }

    #[test]
    fn noweb_and_inline_src_and_calls() {
        assert_eq!(
            noweb_refs("use <<setup>> and <<setup(n=1)>>"),
            vec!["setup", "setup(n=1)"]
        );
        assert_eq!(
            inline_src_spans("see src_python[:results raw]{print(1)} and src_elisp{(+ 1 2)}"),
            vec![
                ("python", ":results raw", "print(1)"),
                ("elisp", "", "(+ 1 2)")
            ]
        );
        assert_eq!(
            inline_call_names("then call_plot[:session](x=1) here"),
            vec!["plot"]
        );
    }

    #[test]
    fn babel_results_hide_headlines_and_drawers() {
        let mut scan = OrgScan::new();
        assert!(!scan.observe("#+NAME: dump"));
        assert!(!scan.observe("prologue"));
        assert!(scan.observe("#+BEGIN_SRC python :results raw"));
        assert!(scan.observe("print('* TODO dumped')"));
        assert!(scan.observe("#+END_SRC"));
        assert!(!scan.inside());
        assert!(scan.observe("#+RESULTS:"));
        assert!(scan.observe("* TODO dumped"));
        assert!(scan.observe(":PROPERTIES:"));
        assert!(scan.observe(":ID:         ghost-9999"));
        assert!(scan.observe(":END:"));
        assert!(scan.inside());
        assert!(!scan.observe("* TODO real"));
        assert!(!scan.inside());
    }

    #[test]
    fn babel_results_table_and_fixed_width_and_drawer() {
        let mut scan = OrgScan::new();
        assert!(scan.observe("#+RESULTS:"));
        assert!(scan.observe("| a | b |"));
        assert!(scan.observe("|---+---|"));
        assert!(scan.observe("| 1 | 2 |"));
        assert!(!scan.observe("after the table"));

        let mut scan = OrgScan::new();
        assert!(scan.observe("#+RESULTS:"));
        assert!(scan.observe(": 42"));
        assert!(!scan.observe("not fixed width"));

        let mut scan = OrgScan::new();
        assert!(scan.observe("#+RESULTS:"));
        assert!(scan.observe(":RESULTS:"));
        assert!(scan.observe("* looks like a headline"));
        assert!(scan.observe(":END:"));
        assert!(!scan.observe("* TODO real"));
    }
}
