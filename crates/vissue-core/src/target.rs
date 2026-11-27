//! A target is where a change lands: one canonical repo and one branch.
//!
//! The id is `sha256` of those two, so a rename is a new record and the old
//! one stays addressable. The record is an org heading in `targets.org` under
//! the tracker prefix. A change points at one or more of them, and its land
//! fields (`TARGET`, `QUORUM`, `LANDERS`) are ordinary properties: when two
//! writers set different values, the org merge keeps both and reports the clash.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use sha2::{Digest, Sha256};

use crate::config::Layout;
use crate::error::{Error, Result};
use crate::model::{IssueHeading, LogEntry};
use crate::props::{self, LAND_TARGET, LANDERS, QUORUM, TARGETS};
use crate::store::{self, IssueDoc};

const TITLE: &str = "#+TITLE: targets\n\n";
const COLUMN: usize = 13;

/// How a land moves the branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The tip must fast-forward the branch.
    FfOnly,
    /// Rebuild the series on the current tip and fast-forward.
    RebuildFf,
    /// A merge commit.
    Merge,
    /// A forge squash at an expected head.
    SquashForge,
    /// Open or update a pull request. Not a land.
    Pr,
    /// Prepare a mail series. Sending stays a human act.
    Mail,
}

impl Mode {
    /// The org spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FfOnly => "ff-only",
            Self::RebuildFf => "rebuild-ff",
            Self::Merge => "merge",
            Self::SquashForge => "squash-forge",
            Self::Pr => "pr",
            Self::Mail => "mail",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "ff-only" => Ok(Self::FfOnly),
            "rebuild-ff" => Ok(Self::RebuildFf),
            "merge" => Ok(Self::Merge),
            "squash-forge" => Ok(Self::SquashForge),
            "pr" => Ok(Self::Pr),
            "mail" => Ok(Self::Mail),
            other => Err(anyhow!("unknown mode: {other}").into()),
        }
    }
}

/// Whether a land may move the branch other than by fast-forward.
///
/// `strict-base` is the default and the only rule a main ref may carry.
/// `lease-rewrite` allows a force-with-lease to an expected old sha, and only
/// when the target says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseRule {
    /// The remote branch must equal the kit's base.
    StrictBase,
    /// A non-fast-forward is allowed with an explicit expected old sha.
    LeaseRewrite,
}

impl LeaseRule {
    /// The org spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StrictBase => "strict-base",
            Self::LeaseRewrite => "lease-rewrite",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "strict-base" => Ok(Self::StrictBase),
            "lease-rewrite" => Ok(Self::LeaseRewrite),
            other => Err(anyhow!("unknown lease rule: {other}").into()),
        }
    }
}

/// Whether this tracker owns the repo the target names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owned {
    /// The tracker owns the repo.
    Yes,
    /// The repo is upstream. A land does not push it.
    Upstream,
}

impl Owned {
    /// The org spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Upstream => "upstream",
        }
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "yes" => Ok(Self::Yes),
            "upstream" => Ok(Self::Upstream),
            other => Err(anyhow!("unknown owned: {other}").into()),
        }
    }
}

/// One `(repo, branch)` a change can land on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `sha256:` of the canonical repo id and the branch.
    pub id: String,
    /// Canonical repo id. A fetch URL is not stored here.
    pub repo: String,
    /// Branch name, or a full ref such as `refs/heads/develop`.
    pub branch: String,
    /// How a land moves the branch.
    pub mode: Mode,
    /// Whether a non-fast-forward land is allowed.
    pub lease: LeaseRule,
    /// Quorum text. Empty when the record does not name one.
    pub quorum: String,
    /// Who may land. Empty when the record does not name them.
    pub landers: String,
    /// Checks that gate the land, in the order they were written.
    pub checks: Vec<String>,
    /// Whether this tracker owns the repo.
    pub owned: Owned,
    /// Fork the pull-request head is pushed to. Empty when there is none.
    pub fork: String,
    /// Mirrors updated after the land, in the order they were written.
    pub mirrors: Vec<String>,
}

/// What one `target` call changes. Absent strings and empty lists are a read
/// of that part, except a record write, which fills mode, lease, and owned
/// with their defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TargetEdit {
    /// Change (issue) id, when the call is about a change.
    pub issue: Option<String>,
    /// Canonical repo id.
    pub repo: Option<String>,
    /// Branch or ref.
    pub branch: Option<String>,
    /// Mode spelling. Default `ff-only` on a record write.
    pub mode: Option<String>,
    /// Lease spelling. Default `strict-base` on a record write.
    pub lease: Option<String>,
    /// Quorum text, on the record or on the change.
    pub quorum: Option<String>,
    /// Landers text, on the record or on the change.
    pub landers: Option<String>,
    /// Checks to store on the record.
    pub checks: Vec<String>,
    /// `yes` or `upstream`. Default `yes` on a record write.
    pub owned: Option<String>,
    /// Fork repo id.
    pub fork: Option<String>,
    /// Mirror repo ids.
    pub mirrors: Vec<String>,
    /// Target ids to add to the change's list.
    pub add: Vec<String>,
    /// Target ids to drop from the change's list.
    pub remove: Vec<String>,
    /// Target id the change leaves.
    pub from: Option<String>,
    /// Target id the change moves to.
    pub to: Option<String>,
    /// Land-field target id on the change.
    pub land_target: Option<String>,
    /// Print the org bytes and write nothing.
    pub org: bool,
}

/// `sha256:` of the canonical repo id and the branch name.
///
/// The two are length-prefixed, so `a` + `bc` and `ab` + `c` do not collide.
#[must_use]
pub fn target_id(repo: &str, branch: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update((repo.len() as u64).to_be_bytes());
    hasher.update(repo.as_bytes());
    hasher.update((branch.len() as u64).to_be_bytes());
    hasher.update(branch.as_bytes());
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256:{hex}")
}

/// Whether `branch` is a main ref.
///
/// One leading `refs/heads/` or `heads/` is stripped, ignoring case, and the
/// remainder is `main` or `master`, ignoring case. `HEAD`, `@`, and any
/// `HEAD` component name no concrete branch: they may resolve to main, so
/// they count as a main ref too. The check uses this form; the stored branch
/// stays the text that was written.
#[must_use]
pub fn is_main_ref(branch: &str) -> bool {
    let branch = branch.trim();
    if is_symbolic_ref(branch) {
        return true;
    }
    let short = strip_one_prefix(branch);
    if short != branch && is_symbolic_ref(short) {
        return true;
    }
    short.eq_ignore_ascii_case("main") || short.eq_ignore_ascii_case("master")
}

fn is_symbolic_ref(branch: &str) -> bool {
    if branch == "@" || branch.eq_ignore_ascii_case("refs/@") {
        return true;
    }
    branch
        .split('/')
        .any(|part| part.eq_ignore_ascii_case("HEAD"))
}

fn strip_one_prefix(branch: &str) -> &str {
    for prefix in ["refs/heads/", "heads/"] {
        if branch.len() >= prefix.len() && branch[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return &branch[prefix.len()..];
        }
    }
    branch
}

/// `git check-ref-format --branch` rules, without calling git.
///
/// A branch name is `refs/heads/` plus the name, and it must not start with
/// `-` or be `HEAD`. `@{` is refused rather than expanded.
fn is_valid_branch_name(name: &str) -> bool {
    if name.is_empty() || name.as_bytes()[0] == b'-' || name == "HEAD" {
        return false;
    }
    refname_ok(&format!("refs/heads/{name}"))
}

fn refname_ok(name: &str) -> bool {
    if name == "@" {
        return false;
    }
    let bytes = name.as_bytes();
    let mut index = 0;
    let mut components = 0usize;
    while index < bytes.len() {
        let Some(len) = component_len(&bytes[index..]) else {
            return false;
        };
        if len == 0 {
            return false;
        }
        let component = &bytes[index..index + len];
        if component[0] == b'.' || component.ends_with(b".lock") {
            return false;
        }
        components += 1;
        index += len;
        if index == bytes.len() {
            break;
        }
        if bytes[index] != b'/' {
            return false;
        }
        index += 1;
        if index == bytes.len() {
            return false;
        }
    }
    components >= 2 && bytes.last() != Some(&b'.')
}

fn component_len(bytes: &[u8]) -> Option<usize> {
    let mut last = 0u8;
    for (index, &byte) in bytes.iter().enumerate() {
        match byte_class(byte) {
            ByteClass::End => return (index > 0).then_some(index),
            ByteClass::Dot if last == b'.' => return None,
            ByteClass::Brace if last == b'@' => return None,
            ByteClass::Bad => return None,
            ByteClass::Ok | ByteClass::Dot | ByteClass::Brace => {}
        }
        last = byte;
    }
    if bytes.is_empty() {
        None
    } else {
        Some(bytes.len())
    }
}

enum ByteClass {
    Ok,
    End,
    Dot,
    Brace,
    Bad,
}

fn byte_class(byte: u8) -> ByteClass {
    // git's `refname_disposition` for bytes 0..=127. Higher bytes are allowed.
    const CLASS: [u8; 128] = [
        1, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, //
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, //
        4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 2, 1, //
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 4, //
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 0, 4, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 4, 4,
    ];
    let class = if usize::from(byte) < CLASS.len() {
        CLASS[usize::from(byte)]
    } else {
        0
    };
    match class {
        1 => ByteClass::End,
        2 => ByteClass::Dot,
        3 => ByteClass::Brace,
        4 | 5 => ByteClass::Bad,
        _ => ByteClass::Ok,
    }
}

impl Target {
    /// Build a record. `lease-rewrite` on a main ref is refused.
    ///
    /// # Errors
    ///
    /// Returns an error if a field is empty or not in its closed set, the
    /// branch is not a valid branch name, or the lease is `lease-rewrite` on
    /// a main ref.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        repo: &str,
        branch: &str,
        mode: &str,
        lease: &str,
        quorum: Option<&str>,
        landers: Option<&str>,
        checks: &[String],
        owned: &str,
        fork: Option<&str>,
        mirrors: &[String],
    ) -> Result<Self> {
        let repo = token("repo", repo)?;
        let branch = token("branch", branch)?;
        let lease = LeaseRule::parse(lease)?;
        if lease == LeaseRule::LeaseRewrite && is_main_ref(&branch) {
            return Err(Error::LeaseRewriteOnMain { branch });
        }
        if !is_valid_branch_name(&branch) {
            return Err(Error::InvalidBranch { branch });
        }
        let checks = tokens("check", checks)?;
        let mirrors = tokens("mirror", mirrors)?;
        let fork = match fork {
            Some(raw) => token("fork", raw)?,
            None => String::new(),
        };
        Ok(Self {
            id: target_id(&repo, &branch),
            repo,
            branch,
            mode: Mode::parse(mode)?,
            lease,
            quorum: optional_line("quorum", quorum)?,
            landers: optional_line("landers", landers)?,
            checks,
            owned: Owned::parse(owned)?,
            fork,
            mirrors,
        })
    }

    fn heading(&self) -> String {
        let mut out = format!("* {}\n:PROPERTIES:\n", self.id);
        out.push_str(&prop_line("REPO", &self.repo));
        out.push_str(&prop_line("BRANCH", &self.branch));
        out.push_str(&prop_line("MODE", self.mode.as_str()));
        out.push_str(&prop_line("LEASE", self.lease.as_str()));
        push_text(&mut out, "QUORUM", &self.quorum);
        push_text(&mut out, "LANDERS", &self.landers);
        if !self.checks.is_empty() {
            out.push_str(&prop_line("CHECKS", &self.checks.join(" ")));
        }
        out.push_str(&prop_line("OWNED", self.owned.as_str()));
        push_text(&mut out, "FORK", &self.fork);
        if !self.mirrors.is_empty() {
            out.push_str(&prop_line("MIRRORS", &self.mirrors.join(" ")));
        }
        out.push_str(":END:\n");
        out
    }
}

/// Read or write a target, a change's target list, a retarget, or land fields.
///
/// # Errors
///
/// Returns an error if the tracker is missing, a field is not in its closed
/// set, the branch is not a valid branch name, `lease-rewrite` is asked for
/// on a main ref, or the named issue or target is not in the corpus.
pub fn apply(layout: &Layout, edit: &TargetEdit) -> Result<String> {
    layout.require_tracker()?;
    validate_given(edit)?;
    if edit.org {
        return read(layout, edit);
    }
    if record_fields(edit) {
        if issue_fields(edit) {
            return Err(anyhow!("pass a target or an issue, not both").into());
        }
        return write_record(layout, edit);
    }
    let Some(id) = edit.issue.as_deref() else {
        return Err(anyhow!("name a repo and a branch, or an issue").into());
    };
    write_issue(layout, id, edit)
}

fn record_fields(edit: &TargetEdit) -> bool {
    edit.repo.is_some() || edit.branch.is_some()
}

fn issue_fields(edit: &TargetEdit) -> bool {
    edit.issue.is_some()
        || edit.from.is_some()
        || edit.to.is_some()
        || edit.land_target.is_some()
        || !edit.add.is_empty()
        || !edit.remove.is_empty()
}

fn read(layout: &Layout, edit: &TargetEdit) -> Result<String> {
    if let (Some(repo), Some(branch)) = (edit.repo.as_deref(), edit.branch.as_deref()) {
        let repo = token("repo", repo)?;
        let branch = token("branch", branch)?;
        if !is_valid_branch_name(&branch) {
            return Err(Error::InvalidBranch { branch });
        }
        let id = target_id(&repo, &branch);
        let text = read_file(&targets_path(layout))?;
        let found = parse_file(&text)?;
        if !found.iter().any(|target| target.id == id) {
            return Err(anyhow!("target {id} not found").into());
        }
        return Ok(text);
    }
    if let Some(id) = edit.issue.as_deref() {
        return issue_text(layout, id);
    }
    let text = read_file(&targets_path(layout))?;
    parse_file(&text)?;
    Ok(text)
}

fn validate_given(edit: &TargetEdit) -> Result<()> {
    if let Some(mode) = edit.mode.as_deref() {
        Mode::parse(mode)?;
    }
    if let Some(lease) = edit.lease.as_deref() {
        LeaseRule::parse(lease)?;
    }
    if let Some(owned) = edit.owned.as_deref() {
        Owned::parse(owned)?;
    }
    if let Some(fork) = edit.fork.as_deref() {
        token("fork", fork)?;
    }
    tokens("check", &edit.checks)?;
    tokens("mirror", &edit.mirrors)?;
    if let Some(quorum) = edit.quorum.as_deref() {
        optional_line("quorum", Some(quorum))?;
    }
    if let Some(landers) = edit.landers.as_deref() {
        optional_line("landers", Some(landers))?;
    }
    Ok(())
}

/// Mode, lease, and owned are always written (defaults when the edit omits
/// them). Every other field the edit does not name stays as stored.
fn keep_unnamed(existing: &Target, edit: &TargetEdit, mut written: Target) -> Target {
    if edit.quorum.is_none() {
        written.quorum = existing.quorum.clone();
    }
    if edit.landers.is_none() {
        written.landers = existing.landers.clone();
    }
    if edit.checks.is_empty() {
        written.checks = existing.checks.clone();
    }
    if edit.fork.is_none() {
        written.fork = existing.fork.clone();
    }
    if edit.mirrors.is_empty() {
        written.mirrors = existing.mirrors.clone();
    }
    written
}

fn write_record(layout: &Layout, edit: &TargetEdit) -> Result<String> {
    let repo = edit
        .repo
        .as_deref()
        .ok_or_else(|| anyhow!("a target needs a repo"))?;
    let branch = edit
        .branch
        .as_deref()
        .ok_or_else(|| anyhow!("a target needs a branch"))?;
    let target = Target::try_new(
        repo,
        branch,
        edit.mode.as_deref().unwrap_or("ff-only"),
        edit.lease.as_deref().unwrap_or("strict-base"),
        edit.quorum.as_deref(),
        edit.landers.as_deref(),
        &edit.checks,
        edit.owned.as_deref().unwrap_or("yes"),
        edit.fork.as_deref(),
        &edit.mirrors,
    )?;
    let path = targets_path(layout);
    store::with_issues_lock(&path, || {
        let mut targets = match read_file(&path) {
            Ok(text) => parse_file(&text)?,
            Err(_) => Vec::new(),
        };
        if let Some(slot) = targets.iter_mut().find(|have| have.id == target.id) {
            *slot = keep_unnamed(slot, edit, target.clone());
        } else {
            targets.push(target.clone());
        }
        write_file(&path, &render_file(&targets))?;
        Ok(format!("{}\n", target.id))
    })
}

fn write_issue(layout: &Layout, id: &str, edit: &TargetEdit) -> Result<String> {
    let moving = edit.from.is_some() || edit.to.is_some();
    let list = !edit.add.is_empty() || !edit.remove.is_empty();
    let land = edit.quorum.is_some() || edit.landers.is_some() || edit.land_target.is_some();
    if moving && (list || land) {
        return Err(anyhow!("retarget is its own write").into());
    }
    if !moving && !list && !land {
        return issue_text(layout, id);
    }
    if moving {
        return retarget(layout, id, edit);
    }
    edit_issue(layout, id, edit)
}

fn retarget(layout: &Layout, id: &str, edit: &TargetEdit) -> Result<String> {
    let (Some(from), Some(to)) = (edit.from.as_deref(), edit.to.as_deref()) else {
        return Err(Error::RetargetNeedsBoth);
    };
    let from = token("from", from)?;
    let to = token("to", to)?;
    require_target(layout, &to)?;
    let (_heading, path, project) = store::find_by_id(layout, id)?
        .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    store::with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let heading = heading_mut(&mut doc, id)?;
        let mut listed = target_list(heading);
        if !listed.iter().any(|have| have == &from) {
            return Err(anyhow!("target {from} is not on {id}").into());
        }
        listed.retain(|have| have != &from);
        if !listed.iter().any(|have| have == &to) {
            listed.push(to.clone());
        }
        write_list(heading, &listed);
        set_field(heading, LAND_TARGET, &to);
        heading.logbook.insert(
            0,
            LogEntry {
                timestamp: LogEntry::now(),
                from_state: None,
                to_state: None,
                note: Some(format!("retarget {from} -> {to}")),
                raw: None,
            },
        );
        doc.write()?;
        Ok(format!("{id}: retarget {from} -> {to}\n"))
    })
}

fn edit_issue(layout: &Layout, id: &str, edit: &TargetEdit) -> Result<String> {
    for target in edit.add.iter().chain(edit.land_target.iter()) {
        require_target(layout, target)?;
    }
    let (_heading, path, project) = store::find_by_id(layout, id)?
        .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    store::with_issues_lock(&path, || {
        let mut doc = IssueDoc::parse_file(&project, &path)?;
        let heading = heading_mut(&mut doc, id)?;
        let mut listed = target_list(heading);
        let before = listed.clone();
        for target in &edit.add {
            let target = token("add", target)?;
            if !listed.iter().any(|have| have == &target) {
                listed.push(target);
            }
        }
        for target in &edit.remove {
            let target = token("remove", target)?;
            listed.retain(|have| have != &target);
        }
        let list_changed = listed != before;
        if list_changed {
            write_list(heading, &listed);
        }
        let mut wrote = Vec::new();
        if let Some(quorum) = edit.quorum.as_deref() {
            set_field(heading, QUORUM, &optional_line("quorum", Some(quorum))?);
            wrote.push("quorum");
        }
        if let Some(landers) = edit.landers.as_deref() {
            set_field(heading, LANDERS, &optional_line("landers", Some(landers))?);
            wrote.push("landers");
        }
        if let Some(target) = edit.land_target.as_deref() {
            let target = token("land-target", target)?;
            set_field(heading, LAND_TARGET, &target);
            wrote.push("target");
        }
        if !list_changed && wrote.is_empty() {
            return Ok(issue_report(id, &listed));
        }
        doc.write()?;
        if wrote.is_empty() {
            Ok(issue_report(id, &listed))
        } else {
            Ok(format!("{id}: {}\n", wrote.join(", ")))
        }
    })
}

fn issue_text(layout: &Layout, id: &str) -> Result<String> {
    let (heading, _path, _project) = store::find_by_id(layout, id)?
        .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })?;
    Ok(issue_report(id, &target_list(&heading)))
}

fn issue_report(id: &str, listed: &[String]) -> String {
    if listed.is_empty() {
        return format!("{id}: no targets\n");
    }
    let mut out = format!(
        "{id}: {} target{}\n",
        listed.len(),
        if listed.len() == 1 { "" } else { "s" }
    );
    for target in listed {
        out.push_str("  ");
        out.push_str(target);
        out.push('\n');
    }
    out
}

fn require_target(layout: &Layout, id: &str) -> Result<()> {
    let id = token("target", id)?;
    let text = read_file(&targets_path(layout))?;
    let found = parse_file(&text)?;
    if found.iter().any(|target| target.id == id) {
        Ok(())
    } else {
        Err(anyhow!("target {id} not found").into())
    }
}

fn target_list(heading: &IssueHeading) -> Vec<String> {
    props::get(&heading.properties, TARGETS)
        .map(|raw| raw.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn write_list(heading: &mut IssueHeading, listed: &[String]) {
    if listed.is_empty() {
        props::remove(&mut heading.properties, TARGETS);
        heading.property_order.retain(|key| key != TARGETS);
        return;
    }
    set_field(heading, TARGETS, &listed.join(" "));
}

fn set_field(heading: &mut IssueHeading, key: &str, value: &str) {
    props::insert(&mut heading.properties, key, value.to_string());
    if !heading.property_order.iter().any(|have| have == key) {
        heading.property_order.push(key.to_string());
    }
}

fn heading_mut<'a>(doc: &'a mut IssueDoc, id: &str) -> Result<&'a mut IssueHeading> {
    doc.headings
        .iter_mut()
        .find(|heading| heading.id == id)
        .ok_or_else(|| Error::IssueNotFound { id: id.to_string() })
}

fn targets_path(layout: &Layout) -> PathBuf {
    layout.projects_dir().join("targets.org")
}

fn render_file(targets: &[Target]) -> String {
    let mut out = String::from(TITLE);
    for target in targets {
        out.push_str(&target.heading());
        out.push('\n');
    }
    out
}

fn parse_file(text: &str) -> Result<Vec<Target>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut targets = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some(title) = lines[index].strip_prefix("* ") else {
            index += 1;
            continue;
        };
        index += 1;
        let mut props: Vec<(String, String)> = Vec::new();
        while index < lines.len() && !lines[index].starts_with("* ") {
            if lines[index].trim().eq_ignore_ascii_case(":PROPERTIES:") {
                index += 1;
                while index < lines.len()
                    && !lines[index].trim().eq_ignore_ascii_case(":END:")
                    && !lines[index].starts_with("* ")
                {
                    if let Some(pair) = prop_of(lines[index]) {
                        props.push(pair);
                    }
                    index += 1;
                }
            }
            if index < lines.len() && !lines[index].starts_with("* ") {
                index += 1;
            }
        }
        targets.push(from_props(title.trim(), &props)?);
    }
    Ok(targets)
}

fn from_props(id: &str, props: &[(String, String)]) -> Result<Target> {
    let mut repo = None;
    let mut branch = None;
    let mut mode = None;
    let mut lease = None;
    let mut quorum = None;
    let mut landers = None;
    let mut checks = None;
    let mut owned = None;
    let mut fork = None;
    let mut mirrors = None;
    for (key, value) in props {
        match key.as_str() {
            "REPO" => repo = Some(value.clone()),
            "BRANCH" => branch = Some(value.clone()),
            "MODE" => mode = Some(value.clone()),
            "LEASE" => lease = Some(value.clone()),
            "QUORUM" => quorum = Some(value.clone()),
            "LANDERS" => landers = Some(value.clone()),
            "CHECKS" => checks = Some(value.clone()),
            "OWNED" => owned = Some(value.clone()),
            "FORK" => fork = Some(value.clone()),
            "MIRRORS" => mirrors = Some(value.clone()),
            _ => {}
        }
    }
    let repo = repo.ok_or_else(|| anyhow!("target {id} is missing REPO"))?;
    let branch = branch.ok_or_else(|| anyhow!("target {id} is missing BRANCH"))?;
    let mode = mode.ok_or_else(|| anyhow!("target {id} is missing MODE"))?;
    let lease = lease.ok_or_else(|| anyhow!("target {id} is missing LEASE"))?;
    let owned = owned.ok_or_else(|| anyhow!("target {id} is missing OWNED"))?;
    let checks = split_list(checks.as_deref());
    let mirrors = split_list(mirrors.as_deref());
    let target = Target::try_new(
        &repo,
        &branch,
        &mode,
        &lease,
        quorum.as_deref(),
        landers.as_deref(),
        &checks,
        &owned,
        fork.as_deref(),
        &mirrors,
    )?;
    if target.id != id {
        return Err(anyhow!("target id {id} does not match {repo} and {branch}").into());
    }
    Ok(target)
}

fn split_list(raw: Option<&str>) -> Vec<String> {
    raw.map(|text| text.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|_| anyhow!("no targets recorded").into())
}

fn write_file(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(".targets.org.tmp.{}", std::process::id()));
    if let Err(err) = fs::write(&tmp, text) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err.into());
    }
    Ok(())
}

fn prop_line(key: &str, value: &str) -> String {
    let key_part = format!(":{key}:");
    let pad = if key_part.len() < COLUMN {
        " ".repeat(COLUMN - key_part.len())
    } else {
        " ".to_string()
    };
    format!("{key_part}{pad}{value}\n")
}

fn push_text(out: &mut String, key: &str, value: &str) {
    if !value.is_empty() {
        out.push_str(&prop_line(key, value));
    }
}

fn prop_of(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let rest = line.strip_prefix(':')?;
    let end = rest.find(':')?;
    let key = rest[..end].trim();
    if key.is_empty() {
        return None;
    }
    Some((key.to_string(), rest[end + 1..].trim().to_string()))
}

fn token(label: &str, raw: &str) -> Result<String> {
    let value = raw.trim();
    if value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(anyhow!("{label} must be one token").into());
    }
    Ok(value.to_string())
}

fn tokens(label: &str, raw: &[String]) -> Result<Vec<String>> {
    raw.iter().map(|item| token(label, item)).collect()
}

fn optional_line(label: &str, raw: Option<&str>) -> Result<String> {
    let Some(raw) = raw else {
        return Ok(String::new());
    };
    let value = raw.trim();
    if value.is_empty()
        || value
            .chars()
            .any(|c| c == '\n' || c == '\r' || c.is_control())
    {
        return Err(anyhow!("{label} must be one line").into());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_layout() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("tempdir");
        let layout = Layout::new(dir.path(), "Software");
        fs::create_dir_all(layout.projects_dir()).expect("projects");
        (dir, layout)
    }

    fn record(repo: &str, branch: &str, lease: &str) -> TargetEdit {
        TargetEdit {
            repo: Some(repo.to_string()),
            branch: Some(branch.to_string()),
            mode: Some("merge".to_string()),
            lease: Some(lease.to_string()),
            quorum: Some("8 seats".to_string()),
            landers: Some("seat:reviewer".to_string()),
            checks: vec!["ci:required".to_string()],
            owned: Some("upstream".to_string()),
            fork: Some("github:acme/fork".to_string()),
            mirrors: vec!["gitlab:acme/widgets".to_string()],
            ..TargetEdit::default()
        }
    }

    #[test]
    fn a_target_rewrites_to_the_same_bytes() {
        let (_dir, layout) = temp_layout();
        let edit = record("github:acme/widgets", "develop", "strict-base");
        apply(&layout, &edit).expect("write");
        let path = targets_path(&layout);
        let first = fs::read_to_string(&path).expect("read");
        apply(&layout, &edit).expect("rewrite");
        let second = fs::read_to_string(&path).expect("reread");
        assert_eq!(first, second);
        let org = apply(
            &layout,
            &TargetEdit {
                org: true,
                repo: edit.repo.clone(),
                branch: edit.branch.clone(),
                ..TargetEdit::default()
            },
        )
        .expect("org");
        assert_eq!(org, first);
        assert!(first.contains("8 seats"), "{first}");
        assert!(first.contains(":LEASE:"), "{first}");
    }

    #[test]
    fn lease_rewrite_on_a_main_ref_is_refused() {
        for branch in [
            "main",
            "refs/heads/main",
            "MAIN",
            "Main",
            "master",
            "refs/heads/MAIN",
            "Refs/Heads/main",
            "HEAD",
            "head",
            "@",
            "refs/HEAD",
            "refs/heads/HEAD",
            "heads/main",
        ] {
            assert!(is_main_ref(branch), "{branch} is a main ref");
            let (_dir, layout) = temp_layout();
            let err = apply(
                &layout,
                &record("github:acme/widgets", branch, "lease-rewrite"),
            )
            .expect_err(branch);
            assert!(
                err.to_string()
                    .contains(&format!("lease-rewrite is refused on a main ref: {branch}")),
                "{branch}: {err}"
            );
            assert!(!targets_path(&layout).exists(), "{branch}");
        }
        for branch in ["main/", "refs/heads//main", "foo.lock", "-topic", "a//b"] {
            assert!(!is_main_ref(branch), "{branch}");
            let (_dir, layout) = temp_layout();
            let err = apply(
                &layout,
                &record("github:acme/widgets", branch, "lease-rewrite"),
            )
            .expect_err(branch);
            assert!(
                err.to_string()
                    .contains(&format!("invalid branch: {branch}")),
                "{branch}: {err}"
            );
            assert!(!targets_path(&layout).exists(), "{branch}");
        }

        let (_dir, layout) = temp_layout();
        for branch in ["develop", "mainline", "refs/heads/topic"] {
            assert!(!is_main_ref(branch), "{branch}");
            apply(
                &layout,
                &record("github:acme/widgets", branch, "lease-rewrite"),
            )
            .expect(branch);
        }
        let text = fs::read_to_string(targets_path(&layout)).expect("read");
        assert!(text.contains(":BRANCH:     develop\n"), "{text}");
        assert!(text.contains(":BRANCH:     mainline\n"), "{text}");
        assert!(text.contains(":BRANCH:     refs/heads/topic\n"), "{text}");
        assert!(text.contains("lease-rewrite"), "{text}");

        let main = apply(
            &layout,
            &record("github:acme/widgets", "main", "strict-base"),
        )
        .expect("strict-base on main");
        assert!(main.starts_with("sha256:"), "{main}");

        let repo = "github:acme/widgets";
        let branch = "MAIN";
        let id = target_id(repo, branch);
        let hand = format!(
            "#+TITLE: targets\n\n* {id}\n:PROPERTIES:\n:REPO:       {repo}\n:BRANCH:     {branch}\n:MODE:       merge\n:LEASE:      lease-rewrite\n:OWNED:      yes\n:END:\n"
        );
        let (_hand, hand_layout) = temp_layout();
        fs::write(targets_path(&hand_layout), &hand).expect("hand edit");
        let err = apply(
            &hand_layout,
            &TargetEdit {
                org: true,
                ..TargetEdit::default()
            },
        )
        .expect_err("hand-edited main spelling");
        assert!(
            err.to_string()
                .contains("lease-rewrite is refused on a main ref: MAIN"),
            "{err}"
        );
        assert_eq!(
            fs::read_to_string(targets_path(&hand_layout)).expect("unchanged"),
            hand,
            "a refused read rewrote the file"
        );
    }

    #[test]
    fn a_partial_record_write_keeps_quorum_and_landers() {
        let (_dir, layout) = temp_layout();
        let edit = record("github:acme/widgets", "develop", "strict-base");
        apply(&layout, &edit).expect("write");
        apply(
            &layout,
            &TargetEdit {
                repo: edit.repo.clone(),
                branch: edit.branch.clone(),
                mode: Some("merge".to_string()),
                lease: Some("strict-base".to_string()),
                ..TargetEdit::default()
            },
        )
        .expect("restate");
        let text = fs::read_to_string(targets_path(&layout)).expect("read");
        assert!(
            text.contains("8 seats") && text.contains("seat:reviewer"),
            "{text}"
        );
    }
}
