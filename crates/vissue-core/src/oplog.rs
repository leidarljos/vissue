//! Content-addressed operations and a pure fold into today's in-memory issues.
//!
//! The bytes are the canonical Cap'n Proto encoding of `schema/op.capnp`.
//! Decode accepts a segment only when re-encoding it yields those same bytes,
//! so an op id, the sha256 of that segment, is the hash of the bytes that were
//! accepted. The segment table is redundant on a single segment and is not
//! part of the hash. Wall time is stored and is not part of the order.
//! Concurrent body edits always clash; fold does not merge them.
//! Org files stay the source of truth; nothing here writes one.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::anyhow;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::model::{IssueHeading, LogEntry};

/// 128-bit issue identity. Derived from the project and the short id, so the
/// same heading always names the same key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IssueKey([u8; 16]);

impl IssueKey {
    /// Key for `short_id` inside `project`.
    #[must_use]
    pub fn derive(project: &str, short_id: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"vissue-issue-key\0");
        hasher.update(project.as_bytes());
        hasher.update([0]);
        hasher.update(short_id.as_bytes());
        let dig = hasher.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&dig[..16]);
        Self(bytes)
    }

    /// Raw key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// sha256 of the canonical segment of one op.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpId([u8; 32]);

impl OpId {
    /// Raw digest bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// What an op does to one issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpKind {
    /// First record of an issue.
    Create {
        /// Short id, the `:ID:` value.
        short_id: String,
        /// Heading title.
        title: String,
        /// TODO keyword.
        state: String,
        /// One priority character.
        priority: String,
    },
    /// Replace one scalar field.
    SetField {
        /// `title`, `state`, `priority`, `body`, `tags`, `statistics`, or `property:<KEY>`.
        name: String,
        /// New value. An empty `statistics` clears the cookie.
        value: String,
    },
    /// Add one logbook line. Log lines are a set, so repeats do not accumulate.
    AppendLog {
        /// The line to add.
        entry: LogEntry,
    },
    /// One voter's ballot. A later causal ballot replaces it.
    Ballot {
        /// Identity that voted.
        voter: String,
        /// Revision the ballot is about. Empty when the vote is on the issue.
        rev: String,
        /// The choice, verbatim.
        choice: String,
        /// Inactive stamp, brackets included.
        stamp: String,
    },
    /// One revision, keyed by its id.
    AddRevision {
        /// Revision id.
        rev_id: String,
        /// Short text kept with it.
        summary: String,
    },
    /// Fields of one target.
    SetTarget {
        /// Target id.
        target_id: String,
        /// Field name and value, in the order they were named.
        fields: Vec<(String, String)>,
    },
    /// Point the issue at a target.
    Retarget {
        /// Target id.
        target_id: String,
    },
    /// One deed accession. Deeds are a set.
    AddDeed {
        /// Accession token.
        accession: String,
    },
    /// One edge. `blocked-by` unions into `:BLOCKED_BY:`.
    Link {
        /// Edge kind, such as `blocked-by`.
        kind: String,
        /// The other issue's short id.
        other: String,
    },
    /// Replace the body. Concurrent body edits always clash.
    SetBody {
        /// Body text.
        body: String,
    },
    /// Move the issue to `CANCELLED` and record why.
    Retire {
        /// Note filed with the retirement. Empty files none.
        reason: String,
    },
    /// A chosen value whose parents are the ops it settles.
    Resolve {
        /// Field name, same vocabulary as [`OpKind::SetField`].
        field: String,
        /// The value that stands.
        value: String,
    },
    /// One parsed issue, carried whole so a fold can reproduce it.
    GenesisImport {
        /// The heading as the org parser built it.
        heading: Box<IssueHeading>,
        /// Bytes the heading does not already carry.
        residue: String,
    },
}

/// One immutable operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    /// Heads the writer had seen. Empty only for a genesis op.
    pub parents: Vec<OpId>,
    /// `max(parents) + 1`, or 1 when there are no parents.
    pub lamport: u64,
    /// Seat that wrote the op.
    pub seat: String,
    /// Project the issue lives in.
    pub project: String,
    /// Stable issue key.
    pub issue: IssueKey,
    /// Payload.
    pub kind: OpKind,
    /// Wall clock, unix milliseconds. Never used for order.
    pub wall: i64,
}

/// Two ops that wrote one field and neither happened before the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// Project of the issue.
    pub project: String,
    /// Short id of the issue.
    pub id: String,
    /// Field name.
    pub field: String,
    /// The ops still in force, ordered by `(lamport, op id)`.
    pub ops: Vec<OpId>,
}

/// Issues after folding a set of ops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folded {
    /// Headings keyed by `(project, short id)`.
    pub issues: BTreeMap<(String, String), IssueHeading>,
    /// Scalar fields with more than one live value.
    pub conflicts: Vec<Conflict>,
}

impl Op {
    /// A parentless create. State is `TODO` and priority is `B`.
    ///
    /// # Errors
    ///
    /// Returns an error when `project` or `short_id` is empty.
    pub fn create(project: &str, short_id: &str, title: &str, seat: &str) -> Result<Self> {
        if project.is_empty() || short_id.is_empty() {
            return Err(bad("an op needs a project and an issue id"));
        }
        Self::born(
            &[],
            project,
            short_id,
            seat,
            OpKind::Create {
                short_id: short_id.to_string(),
                title: title.to_string(),
                state: "TODO".to_string(),
                priority: "B".to_string(),
            },
        )
    }

    /// A scalar write that has seen `parents`.
    ///
    /// # Errors
    ///
    /// Returns an error when a parent cannot be encoded, or the names are empty.
    pub fn set_field(
        parents: &[&Self],
        project: &str,
        short_id: &str,
        name: &str,
        value: &str,
        seat: &str,
    ) -> Result<Self> {
        Self::born(
            parents,
            project,
            short_id,
            seat,
            OpKind::SetField {
                name: name.to_string(),
                value: value.to_string(),
            },
        )
    }

    /// An added log line that has seen `parents`.
    ///
    /// # Errors
    ///
    /// Returns an error when a parent cannot be encoded, or the names are empty.
    pub fn append_log(
        parents: &[&Self],
        project: &str,
        short_id: &str,
        entry: LogEntry,
        seat: &str,
    ) -> Result<Self> {
        Self::born(
            parents,
            project,
            short_id,
            seat,
            OpKind::AppendLog { entry },
        )
    }

    fn born(
        parents: &[&Self],
        project: &str,
        short_id: &str,
        seat: &str,
        kind: OpKind,
    ) -> Result<Self> {
        if project.is_empty() || short_id.is_empty() {
            return Err(bad("an op needs a project and an issue id"));
        }
        let mut ids = Vec::with_capacity(parents.len());
        let mut max_lamport = 0u64;
        for parent in parents {
            ids.push(parent.id()?);
            max_lamport = max_lamport.max(parent.lamport);
        }
        let op = Self {
            parents: ids,
            lamport: max_lamport + 1,
            seat: seat.to_string(),
            project: project.to_string(),
            issue: IssueKey::derive(project, short_id),
            kind,
            wall: 0,
        };
        check_op(&op)?;
        Ok(op)
    }

    /// Id of this op, the sha256 of its canonical segment.
    ///
    /// # Errors
    ///
    /// Returns an error when the op cannot be encoded.
    pub fn id(&self) -> Result<OpId> {
        let segment = self.canonical_segment()?;
        let dig = Sha256::digest(segment);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&dig);
        Ok(OpId(bytes))
    }

    /// Canonical segment. This is the byte string [`Self::id`] hashes.
    ///
    /// # Errors
    ///
    /// Returns an error when a text field contains a zero byte or the op is
    /// too large for one segment.
    pub fn canonical_segment(&self) -> Result<Vec<u8>> {
        encode_op(self)
    }

    /// A single-segment Cap'n Proto message: the segment table, then the segment.
    ///
    /// # Errors
    ///
    /// Returns an error when the op cannot be encoded.
    pub fn message_bytes(&self) -> Result<Vec<u8>> {
        let segment = self.canonical_segment()?;
        let words = u32::try_from(segment.len() / 8).map_err(|_| bad("op is too large"))?;
        let mut out = Vec::with_capacity(8 + segment.len());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&words.to_le_bytes());
        out.extend_from_slice(&segment);
        Ok(out)
    }

    /// Decode a message produced by [`Self::message_bytes`].
    ///
    /// # Errors
    ///
    /// Returns an error when the bytes are not the canonical encoding of one
    /// op, the issue key is not [`IssueKey::derive`] of its project and id, or
    /// a field name is not one the fold can write.
    pub fn from_message_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 || !bytes.len().is_multiple_of(8) {
            return Err(bad("op message is not a whole number of words"));
        }
        let nseg = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize + 1;
        let words = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        if nseg != 1 || 8 + words * 8 != bytes.len() {
            return Err(bad("an op message is one segment"));
        }
        decode_segment(&bytes[8..])
    }
}

/// One genesis op per heading. Folding the result reproduces the headings.
///
/// # Errors
///
/// Returns an error when a heading has no id.
pub fn describe(project: &str, headings: &[IssueHeading]) -> Result<Vec<Op>> {
    headings
        .iter()
        .map(|heading| {
            if heading.id.is_empty() {
                return Err(bad("a heading has no id"));
            }
            Op::born(
                &[],
                project,
                &heading.id,
                "import",
                OpKind::GenesisImport {
                    heading: Box::new(heading.clone()),
                    residue: String::new(),
                },
            )
        })
        .collect()
}

/// Fold `ops` in topological order, ties broken by `(lamport, op id)`.
///
/// The same set folds to the same issues whichever order it is passed in.
/// A property written by two ops that are not ordered by the parent relation
/// stays in [`Folded::conflicts`], and the heading shows the value from the
/// later `(lamport, op id)`. A scalar write concurrent with union adds on a
/// set field shows the union of those writes and still clashes. The
/// replacement drops elements it observed; an add it did not see stays. A
/// field write, log line, or retire note that sorts before the first birth
/// is merged when that birth is applied. Concurrent body edits always clash.
/// Two issues that share a short id both stay; the later one by
/// `(lamport, op id)` is shown with the smallest `~N` that no real
/// or already-assigned id uses.
///
/// # Errors
///
/// Returns an error when a parent is missing, the lamport clock does not
/// match the parents, the graph has a cycle, or an op names an issue that
/// has not been created.
pub fn fold(ops: &[Op]) -> Result<Folded> {
    for op in ops {
        check_op(op)?;
    }
    let indexed = index(ops)?;
    let order = topo(&indexed)?;
    let mut mem = Mem::default();
    for id in order {
        let op = &indexed[&id];
        mem.ancestors.insert(id, ancestor_set(op, &mem));
        apply(&mut mem, id, op)?;
    }
    require_created(&mem)?;
    Ok(mem.finish())
}

fn require_created(mem: &Mem) -> Result<()> {
    let mut keys: Vec<IssueKey> = mem
        .pending
        .keys()
        .filter(|(key, _)| !mem.issues.contains_key(key))
        .map(|(key, _)| *key)
        .collect();
    keys.extend(
        mem.pending_logs
            .keys()
            .filter(|key| !mem.issues.contains_key(key))
            .copied(),
    );
    keys.sort();
    keys.dedup();
    if let Some(key) = keys.first() {
        return Err(Error::IssueNotFound {
            id: hex16(key.as_bytes()),
        });
    }
    Ok(())
}

fn bad(msg: impl Into<String>) -> Error {
    anyhow!(msg.into()).into()
}

fn index(ops: &[Op]) -> Result<BTreeMap<OpId, Op>> {
    let mut map = BTreeMap::new();
    for op in ops {
        let id = op.id()?;
        map.entry(id).or_insert_with(|| op.clone());
    }
    for op in map.values() {
        let max_parent = op
            .parents
            .iter()
            .map(|parent| map.get(parent).map(|p| p.lamport))
            .try_fold(0u64, |max, lamport| {
                lamport
                    .map(|value| max.max(value))
                    .ok_or_else(|| bad("an op names a parent that is not in the set"))
            })?;
        let Some(expect) = max_parent.checked_add(1) else {
            return Err(bad("an op's lamport clock does not follow its parents"));
        };
        if op.lamport != expect {
            return Err(bad("an op's lamport clock does not follow its parents"));
        }
        if op.parents.is_empty()
            && !matches!(
                op.kind,
                OpKind::Create { .. } | OpKind::GenesisImport { .. }
            )
        {
            return Err(bad("an op with no parents is a create or a genesis"));
        }
    }
    Ok(map)
}

fn topo(map: &BTreeMap<OpId, Op>) -> Result<Vec<OpId>> {
    let mut indeg: HashMap<OpId, usize> = HashMap::new();
    let mut children: HashMap<OpId, Vec<OpId>> = HashMap::new();
    for (id, op) in map {
        indeg.entry(*id).or_insert(0);
        for parent in &op.parents {
            *indeg.entry(*id).or_insert(0) += 1;
            children.entry(*parent).or_default().push(*id);
        }
    }
    let mut ready: BTreeSet<(u64, OpId)> = indeg
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| (map[id].lamport, *id))
        .collect();
    let mut order = Vec::with_capacity(map.len());
    while let Some((_, id)) = ready.pop_first() {
        order.push(id);
        for child in children.get(&id).into_iter().flatten() {
            let Some(degree) = indeg.get_mut(child) else {
                return Err(bad("the op set has a cycle"));
            };
            *degree -= 1;
            if *degree == 0 {
                ready.insert((map[child].lamport, *child));
            }
        }
    }
    if order.len() != map.len() {
        return Err(bad("the op set has a cycle"));
    }
    Ok(order)
}

fn ancestor_set(op: &Op, mem: &Mem) -> HashSet<OpId> {
    let mut set = HashSet::new();
    for parent in &op.parents {
        set.insert(*parent);
        if let Some(older) = mem.ancestors.get(parent) {
            set.extend(older.iter().copied());
        }
    }
    set
}

#[derive(Default)]
struct Mem {
    issues: HashMap<IssueKey, Vec<Live>>,
    ancestors: HashMap<OpId, HashSet<OpId>>,
    /// Field writers for one birth. Keyed by the birth op.
    frontiers: HashMap<(OpId, String), Frontier>,
    /// Field writes that have not seen a birth of their issue yet.
    pending: HashMap<(IssueKey, String), Frontier>,
    /// Births of one short id. Two writers here are the short-id clash.
    issue_frontier: HashMap<IssueKey, Frontier>,
    pending_logs: HashMap<IssueKey, Vec<LogEntry>>,
}

struct Live {
    project: String,
    short_id: String,
    heading: IssueHeading,
    links: BTreeSet<(String, String)>,
    ballots: BTreeMap<String, String>,
    revisions: BTreeMap<String, String>,
    targets: BTreeMap<(String, String), String>,
    retarget: Option<String>,
    /// Drawer lines the base reader does not take. A rewrite copies them verbatim.
    raw: BTreeMap<String, BTreeSet<String>>,
    born_lamport: u64,
    born_id: OpId,
    /// Length of the `property_order` prefix fixed by create or genesis.
    order_fixed: usize,
}

#[derive(Clone, Default)]
struct Frontier {
    writers: Vec<Writer>,
}

/// Add-tags for one set field. The tag's op is the add that introduced the
/// element. A replacement drops tags it observed. A later union does not
/// split an earlier element on whitespace.
#[derive(Clone, Default)]
struct UnionSet {
    tags: BTreeSet<(OpId, String)>,
}

impl UnionSet {
    fn from_scalar(source: OpId, value: &str) -> Self {
        let mut set = Self::default();
        for element in list_elements(value) {
            set.insert(source, &element);
        }
        set
    }

    fn insert(&mut self, source: OpId, element: &str) {
        if !element.is_empty() {
            self.tags.insert((source, element.to_string()));
        }
    }

    fn extend_set(&mut self, other: &Self) {
        self.tags.extend(other.tags.iter().cloned());
    }

    /// Tags whose adding op is in `sources` were observed and are gone.
    fn without(&self, sources: &HashSet<OpId>) -> Self {
        let mut out = Self::default();
        for (source, element) in &self.tags {
            if !sources.contains(source) {
                out.tags.insert((*source, element.clone()));
            }
        }
        out
    }

    fn canonical(&self) -> String {
        let mut elements = BTreeSet::new();
        for (_, element) in &self.tags {
            elements.insert(element.as_str());
        }
        let mut out = String::new();
        for (index, element) in elements.iter().enumerate() {
            if index > 0 {
                out.push(' ');
            }
            out.push_str(element);
        }
        out
    }
}

#[derive(Clone)]
struct Writer {
    id: OpId,
    lamport: u64,
    value: String,
    /// Pure unions are not a clash. Elements live in `union`.
    unite: bool,
    /// When set, `union` is this writer's own elements and `value` is the merged display.
    owned: bool,
    union: UnionSet,
    /// Add op ids whose elements this writer has removed.
    removed: HashSet<OpId>,
}

fn stored_union(writer: &Writer) -> UnionSet {
    if writer.unite || writer.owned {
        writer.union.clone()
    } else {
        UnionSet::from_scalar(writer.id, &writer.value)
    }
}

fn carried(writer: &Writer, drop_sources: &HashSet<OpId>) -> UnionSet {
    stored_union(writer).without(drop_sources)
}

fn removed_by(writers: &[Writer]) -> HashSet<OpId> {
    let mut removed = HashSet::new();
    for writer in writers {
        removed.extend(&writer.removed);
    }
    removed
}

fn push_conflict(
    conflicts: &mut Vec<Conflict>,
    frontier: &Frontier,
    live: Option<&Live>,
    field: &str,
) {
    if frontier.writers.len() < 2 || frontier.writers.iter().all(|writer| writer.unite) {
        return;
    }
    let Some(live) = live else {
        return;
    };
    let mut writers = frontier.writers.iter().collect::<Vec<_>>();
    writers.sort_by_key(|writer| (writer.lamport, writer.id));
    conflicts.push(Conflict {
        project: live.project.clone(),
        id: live.short_id.clone(),
        field: field.to_string(),
        ops: writers.into_iter().map(|writer| writer.id).collect(),
    });
}

impl Mem {
    fn finish(self) -> Folded {
        let mut conflicts = Vec::new();
        for (key, frontier) in &self.issue_frontier {
            push_conflict(
                &mut conflicts,
                frontier,
                self.issues.get(key).and_then(|lives| lives.first()),
                "issue",
            );
        }
        for ((birth, field), frontier) in &self.frontiers {
            let live = self
                .issues
                .values()
                .flatten()
                .find(|live| live.born_id == *birth);
            push_conflict(&mut conflicts, frontier, live, field);
        }
        conflicts.sort_by(|a, b| {
            (&a.project, &a.id, &a.field, &a.ops).cmp(&(&b.project, &b.id, &b.field, &b.ops))
        });
        let mut lives: Vec<(IssueKey, Live)> = Vec::new();
        for (key, group) in self.issues {
            for live in group {
                lives.push((key, live));
            }
        }
        lives.sort_by(|a, b| {
            (
                &a.1.project,
                &a.1.short_id,
                a.1.born_lamport,
                a.1.born_id,
                a.0,
            )
                .cmp(&(
                    &b.1.project,
                    &b.1.short_id,
                    b.1.born_lamport,
                    b.1.born_id,
                    b.0,
                ))
        });
        let real: BTreeSet<(String, String)> = lives
            .iter()
            .map(|(_, live)| (live.project.clone(), live.short_id.clone()))
            .collect();
        let mut issues = BTreeMap::new();
        let mut index = 0;
        while index < lives.len() {
            let mut end = index + 1;
            while end < lives.len()
                && lives[end].1.project == lives[index].1.project
                && lives[end].1.short_id == lives[index].1.short_id
            {
                end += 1;
            }
            for (rank, (_, live)) in lives[index..end].iter().enumerate() {
                let mut shown = if rank == 0 {
                    live.short_id.clone()
                } else {
                    free_suffix(&live.project, &live.short_id, &real, &issues)
                };
                if issues.contains_key(&(live.project.clone(), shown.clone())) {
                    shown = free_suffix(&live.project, &live.short_id, &real, &issues);
                }
                let mut heading = live.heading.clone();
                if shown != live.short_id {
                    if heading.id == live.short_id {
                        heading.id = shown.clone();
                    }
                    if heading.properties.get("ID").map(String::as_str) == Some(&live.short_id) {
                        heading.properties.insert("ID".to_string(), shown.clone());
                    }
                }
                issues.insert((live.project.clone(), shown), heading);
            }
            index = end;
        }
        Folded { issues, conflicts }
    }
}

/// Smallest `short_id~N`, from 2 up, that is not a real short id and not
/// already a key in `issues`.
fn free_suffix(
    project: &str,
    short_id: &str,
    real: &BTreeSet<(String, String)>,
    issues: &BTreeMap<(String, String), IssueHeading>,
) -> String {
    let mut n = 2u64;
    loop {
        let shown = format!("{short_id}~{n}");
        let key = (project.to_string(), shown.clone());
        if !real.contains(&key) && !issues.contains_key(&key) {
            return shown;
        }
        n += 1;
    }
}

fn apply(mem: &mut Mem, id: OpId, op: &Op) -> Result<()> {
    match &op.kind {
        OpKind::Create {
            short_id,
            title,
            state,
            priority,
        } => apply_create(mem, id, op, short_id, title, state, priority),
        OpKind::GenesisImport { heading, .. } => apply_genesis(mem, id, op, heading),
        OpKind::SetField { name, value } | OpKind::Resolve { field: name, value } => {
            write_named(mem, id, op, name, value)
        }
        OpKind::SetBody { body } => write_named(mem, id, op, "body", body),
        OpKind::AppendLog { entry } => append_log(mem, id, op, entry.clone()),
        OpKind::Ballot {
            voter,
            rev,
            choice,
            stamp,
        } => {
            let line = vote_line(rev, stamp, voter, choice);
            write_named(mem, id, op, &keyed("ballot:", voter, rev), &line)
        }
        OpKind::AddRevision { rev_id, summary } => {
            write_named(mem, id, op, &format!("revision:{rev_id}"), summary)
        }
        OpKind::SetTarget { target_id, fields } => {
            if fields.is_empty() {
                return write_named(mem, id, op, &format!("target:{target_id}:id"), target_id);
            }
            for (name, value) in fields {
                write_named(mem, id, op, &format!("target:{target_id}:{name}"), value)?;
            }
            Ok(())
        }
        OpKind::Retarget { target_id } => write_named(mem, id, op, "retarget", target_id),
        OpKind::AddDeed { accession } => add_deed(mem, id, op, accession),
        OpKind::Link { kind, other } => add_link(mem, id, op, kind, other),
        OpKind::Retire { reason } => {
            write_named(mem, id, op, "state", "CANCELLED")?;
            if !reason.is_empty() {
                append_log(
                    mem,
                    id,
                    op,
                    LogEntry {
                        timestamp: String::new(),
                        from_state: None,
                        to_state: None,
                        note: Some(reason.clone()),
                        raw: None,
                    },
                )?;
            }
            Ok(())
        }
    }
}

fn apply_create(
    mem: &mut Mem,
    id: OpId,
    op: &Op,
    short_id: &str,
    title: &str,
    state: &str,
    priority: &str,
) -> Result<()> {
    let priority = one_char(priority)?;
    let heading = fresh_heading(short_id, title, state, priority);
    install_issue(mem, id, op, short_id, &heading)
}

fn apply_genesis(mem: &mut Mem, id: OpId, op: &Op, heading: &IssueHeading) -> Result<()> {
    install_issue(mem, id, op, &heading.id, heading)
}

/// Concurrent births of one short id are two issues. A causal second create,
/// one that saw the earlier birth and no concurrent birth, is still an error.
/// Each birth keeps the heading it carried. Writes that had not seen a birth
/// are merged in when that birth is applied.
fn install_issue(
    mem: &mut Mem,
    id: OpId,
    op: &Op,
    short_id: &str,
    heading: &IssueHeading,
) -> Result<()> {
    require_derived(&op.project, short_id, op.issue)?;
    let key = op.issue;
    let ancestors = mem.ancestors.get(&id).cloned().unwrap_or_default();
    let existing: Vec<OpId> = mem
        .issues
        .get(&key)
        .map(|lives| lives.iter().map(|live| live.born_id).collect())
        .unwrap_or_default();
    let causal = existing.iter().any(|birth| ancestors.contains(birth));
    let concurrent = existing.iter().any(|birth| !ancestors.contains(birth));
    if !existing.is_empty() && causal && !concurrent {
        return Err(bad("an issue is created twice"));
    }
    {
        let slot = mem.issue_frontier.entry(key).or_default();
        converge(slot, &ancestors, id, op.lamport, &heading.title, false);
    }
    let mut live = Live::from_op(op, short_id, heading.clone(), id);
    let fields = heading_writes(heading);
    let written: HashSet<String> = fields.iter().map(|(name, _)| name.clone()).collect();
    for (field, value) in &fields {
        let mut slot = mem
            .pending
            .get(&(key, field.clone()))
            .cloned()
            .unwrap_or_default();
        let display = converge(&mut slot, &ancestors, id, op.lamport, value, false);
        if slot.writers.iter().any(|writer| writer.id != id) {
            project(&mut live, field, &display);
        }
        mem.frontiers.insert((id, field.clone()), slot);
    }
    absorb_pending(mem, key, id, &mut live, &written);
    absorb_logs(mem, key, &mut live);
    mem.issues.entry(key).or_default().push(live);
    Ok(())
}

fn absorb_pending(
    mem: &mut Mem,
    key: IssueKey,
    id: OpId,
    live: &mut Live,
    written: &HashSet<String>,
) {
    let names: Vec<String> = mem
        .pending
        .keys()
        .filter(|(issue, name)| *issue == key && !written.contains(name))
        .map(|(_, name)| name.clone())
        .collect();
    for name in names {
        let Some(slot) = mem.pending.get(&(key, name.clone())).cloned() else {
            continue;
        };
        project(live, &name, &frontier_display(&slot));
        mem.frontiers.insert((id, name), slot);
    }
}

fn absorb_logs(mem: &Mem, key: IssueKey, live: &mut Live) {
    let Some(entries) = mem.pending_logs.get(&key) else {
        return;
    };
    for entry in entries {
        if !live.heading.logbook.iter().any(|have| have == entry) {
            live.heading.logbook.push(entry.clone());
        }
    }
    if !entries.is_empty() {
        live.heading.logbook.sort_by(log_order);
    }
}

fn frontier_display(frontier: &Frontier) -> String {
    frontier
        .writers
        .iter()
        .max_by_key(|writer| (writer.lamport, writer.id))
        .map(|writer| writer.value.clone())
        .unwrap_or_default()
}

fn fresh_heading(id: &str, title: &str, state: &str, priority: char) -> IssueHeading {
    let mut properties = BTreeMap::new();
    properties.insert("ID".to_string(), id.to_string());
    IssueHeading {
        id: id.to_string(),
        title: title.to_string(),
        state: state.to_string(),
        priority,
        properties,
        org_tags: Vec::new(),
        statistics: None,
        property_order: vec!["ID".to_string()],
        extra_drawers: Vec::new(),
        body: String::new(),
        logbook: Vec::new(),
        line_start: 0,
        line_end: 0,
    }
}

fn heading_writes(heading: &IssueHeading) -> Vec<(String, String)> {
    let mut fields = vec![
        ("title".to_string(), heading.title.clone()),
        ("state".to_string(), heading.state.clone()),
        ("priority".to_string(), heading.priority.to_string()),
        ("body".to_string(), heading.body.clone()),
        ("tags".to_string(), heading.org_tags.join(" ")),
        (
            "statistics".to_string(),
            heading.statistics.clone().unwrap_or_default(),
        ),
    ];
    for (name, value) in &heading.properties {
        fields.push((format!("property:{name}"), value.clone()));
    }
    let (ballots, _) = load_votes(heading);
    for (slot, line) in ballots {
        fields.push((slot, line));
    }
    let (revisions, _) = load_revisions(heading);
    for (rev, summary) in revisions {
        fields.push((format!("revision:{rev}"), summary));
    }
    let (targets, retarget, _) = load_targets(heading);
    for ((target, key), value) in targets {
        fields.push((format!("target:{target}:{key}"), value));
    }
    if let Some(target) = retarget {
        fields.push(("retarget".to_string(), target));
    }
    fields
}

fn write_named(mem: &mut Mem, id: OpId, op: &Op, name: &str, value: &str) -> Result<()> {
    write_field(mem, id, op, name, value, false)
}

fn write_union(mem: &mut Mem, id: OpId, op: &Op, name: &str, element: &str) -> Result<()> {
    write_field(mem, id, op, name, element, true)
}

/// One path for every property write, including births and revisions.
///
/// A writer whose op is not an ancestor is concurrent. The fold keeps both
/// and [`Mem::finish`] records a clash, except a pure set-union, which
/// commutes. A set replacement drops add-tags it observed and keeps an add
/// it did not see. The issue row may be a later birth; the write stays on
/// the frontier until that birth merges it.
fn write_field(
    mem: &mut Mem,
    id: OpId,
    op: &Op,
    name: &str,
    value: &str,
    unite: bool,
) -> Result<()> {
    let field = canonical_field(name)?;
    if field == "priority" {
        one_char(value)?;
    }
    let ancestors = mem.ancestors.get(&id).cloned().unwrap_or_default();
    let births: Vec<OpId> = mem
        .issues
        .get(&op.issue)
        .map(|lives| lives.iter().map(|live| live.born_id).collect())
        .unwrap_or_default();
    let causal: Vec<OpId> = births
        .iter()
        .copied()
        .filter(|birth| ancestors.contains(birth))
        .collect();
    if causal.is_empty() {
        let slot = mem.pending.entry((op.issue, field.clone())).or_default();
        converge(slot, &ancestors, id, op.lamport, value, unite);
    }
    let targets = if causal.is_empty() { births } else { causal };
    for birth in targets {
        let display = {
            let slot = mem.frontiers.entry((birth, field.clone())).or_default();
            converge(slot, &ancestors, id, op.lamport, value, unite)
        };
        if let Some(lives) = mem.issues.get_mut(&op.issue)
            && let Some(live) = lives.iter_mut().find(|live| live.born_id == birth)
        {
            project(live, &field, &display);
        }
    }
    Ok(())
}

fn converge(
    slot: &mut Frontier,
    ancestors: &HashSet<OpId>,
    id: OpId,
    lamport: u64,
    value: &str,
    unite: bool,
) -> String {
    let mut causal = Vec::new();
    let mut concurrent = Vec::new();
    for writer in slot.writers.drain(..) {
        if ancestors.contains(&writer.id) {
            causal.push(writer);
        } else {
            concurrent.push(writer);
        }
    }
    let replacing = !unite
        && (causal.iter().any(|writer| writer.unite || writer.owned)
            || concurrent.iter().any(|writer| writer.unite || writer.owned));
    let (stored, union, owned, removed) = if unite {
        // Tags this op observed, minus every tombstone already in the
        // frontier. A remover this op did not see still drops the tag it
        // observed, so a descendant cannot copy that tag back. A concurrent
        // add keeps the tag it introduced.
        let mut tombstones = removed_by(&causal);
        tombstones.extend(removed_by(&concurrent));
        let mut union = UnionSet::default();
        for writer in &causal {
            union.extend_set(&carried(writer, &tombstones));
        }
        union.insert(id, value);
        let mut display = union.clone();
        for writer in &concurrent {
            display.extend_set(&carried(writer, &tombstones));
        }
        (display.canonical(), union, false, removed_by(&causal))
    } else if replacing {
        let mut removed = ancestors.clone();
        removed.extend(removed_by(&causal));
        removed.extend(removed_by(&concurrent));
        let mut display = UnionSet::default();
        for writer in &concurrent {
            display.extend_set(&carried(writer, &removed));
        }
        let own = UnionSet::from_scalar(id, value);
        display.extend_set(&own);
        (display.canonical(), own, true, removed)
    } else {
        (
            value.to_string(),
            UnionSet::default(),
            false,
            HashSet::new(),
        )
    };
    let mut keep = concurrent;
    keep.push(Writer {
        id,
        lamport,
        value: stored,
        unite,
        owned,
        union,
        removed,
    });
    let display = keep
        .iter()
        .max_by_key(|writer| (writer.lamport, writer.id))
        .map(|writer| writer.value.clone())
        .unwrap_or_default();
    slot.writers = keep;
    display
}

fn canonical_field(name: &str) -> Result<String> {
    if matches!(
        name,
        "title" | "state" | "priority" | "body" | "tags" | "statistics" | "retarget"
    ) {
        return Ok(name.to_string());
    }
    if let Some(key) = name.strip_prefix("property:") {
        spellable_colon("field", key)?;
        return Ok(name.to_string());
    }
    if let Some(rev) = name.strip_prefix("revision:") {
        spellable("revision", rev)?;
        return Ok(name.to_string());
    }
    if let Some(rest) = name.strip_prefix("target:") {
        let Some((id, field)) = rest.split_once(':') else {
            return Err(bad("unknown field"));
        };
        if !token_ok_colon(id) || !token_ok_colon(field) {
            return Err(bad(format!("field {name:?}")));
        }
        return Ok(name.to_string());
    }
    if name.starts_with("link:") {
        let Some((kind, other)) = unkey("link:", name) else {
            return Err(bad(format!("field {name:?}")));
        };
        spellable("link kind", &kind)?;
        spellable("id", &other)?;
        return Ok(name.to_string());
    }
    if name.starts_with("ballot:") {
        let Some((voter, rev)) = unkey("ballot:", name) else {
            return Err(bad(format!("field {name:?}")));
        };
        spellable("voter", &voter)?;
        if !rev.is_empty() {
            spellable("revision", &rev)?;
        }
        return Ok(name.to_string());
    }
    Err(bad("unknown field"))
}

fn project(live: &mut Live, field: &str, value: &str) {
    match field {
        "title" => live.heading.title = value.to_string(),
        "state" => live.heading.state = value.to_string(),
        "priority" => live.heading.priority = value.chars().next().unwrap_or('C'),
        "body" => live.heading.body = value.to_string(),
        "tags" => {
            live.heading.org_tags = value.split_whitespace().map(str::to_string).collect();
        }
        "statistics" => {
            live.heading.statistics = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        }
        "retarget" => {
            live.retarget = Some(value.to_string());
            rewrite_targets(live);
        }
        name if name.starts_with("property:") => {
            set_property(live, &name["property:".len()..], value)
        }
        name if name.starts_with("ballot:") => {
            live.ballots.insert(name.to_string(), value.to_string());
            rewrite_ballots(live);
        }
        name if name.starts_with("revision:") => {
            let rev = &name["revision:".len()..];
            rewrite_revisions(live, rev, value);
        }
        name if let Some((target, key)) = split_target(name) => {
            live.targets.insert((target, key), value.to_string());
            rewrite_targets(live);
        }
        name if let Some((kind, other)) = unkey("link:", name) => {
            if value.is_empty() {
                live.links.remove(&(kind.clone(), other.clone()));
            } else {
                live.links.insert((kind, other));
            }
            let mut lines: Vec<String> = live
                .links
                .iter()
                .map(|(kind, other)| link_line(kind, other))
                .collect();
            push_raw(&live.raw, "LINKS", &mut lines);
            lines.sort();
            lines.dedup();
            replace_drawer(&mut live.heading, "LINKS", &lines);
        }
        _ => {}
    }
}

fn set_property(live: &mut Live, key: &str, value: &str) {
    if value.is_empty() {
        live.heading.properties.remove(key);
        if let Some(index) = live
            .heading
            .property_order
            .iter()
            .position(|name| name == key)
        {
            live.heading.property_order.remove(index);
            if index < live.order_fixed {
                live.order_fixed -= 1;
            }
        }
        return;
    }
    live.heading
        .properties
        .insert(key.to_string(), value.to_string());
    if live.heading.property_order.iter().any(|name| name == key) {
        return;
    }
    live.heading.property_order.push(key.to_string());
    let start = live.order_fixed;
    if start < live.heading.property_order.len() {
        live.heading.property_order[start..].sort();
    }
}

fn rewrite_ballots(live: &mut Live) {
    let mut lines: Vec<String> = live.ballots.values().cloned().collect();
    push_raw(&live.raw, "VOTES", &mut lines);
    lines.sort();
    lines.dedup();
    replace_drawer(&mut live.heading, "VOTES", &lines);
}

fn rewrite_revisions(live: &mut Live, rev: &str, summary: &str) {
    live.revisions.insert(rev.to_string(), summary.to_string());
    let mut lines: Vec<String> = live
        .revisions
        .iter()
        .map(|(rev, summary)| revision_line(rev, summary))
        .collect();
    push_raw(&live.raw, "REVISIONS", &mut lines);
    lines.sort();
    lines.dedup();
    replace_drawer(&mut live.heading, "REVISIONS", &lines);
}

fn rewrite_targets(live: &mut Live) {
    let mut lines = Vec::new();
    for ((target, key), value) in &live.targets {
        lines.push(target_field_line(target, key, value));
    }
    if let Some(target) = &live.retarget {
        lines.push(retarget_line(target));
    }
    push_raw(&live.raw, "TARGETS", &mut lines);
    lines.sort();
    lines.dedup();
    replace_drawer(&mut live.heading, "TARGETS", &lines);
}

fn drawer_lines(heading: &IssueHeading, name: &str) -> Vec<String> {
    heading
        .extra_drawers
        .iter()
        .find(|drawer| drawer_named(drawer, name))
        .map(|drawer| {
            drawer
                .lines()
                .skip(1)
                .filter(|line| !line.eq_ignore_ascii_case(":END:"))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn replace_drawer(heading: &mut IssueHeading, name: &str, lines: &[String]) {
    heading
        .extra_drawers
        .retain(|drawer| !drawer_named(drawer, name));
    if lines.is_empty() {
        return;
    }
    let mut text = format!(":{name}:\n");
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str(":END:\n");
    heading.extra_drawers.push(text);
    heading.extra_drawers.sort();
}

fn drawer_named(drawer: &str, name: &str) -> bool {
    drawer
        .lines()
        .next()
        .is_some_and(|line| line.trim().eq_ignore_ascii_case(&format!(":{name}:")))
}

fn append_log(mem: &mut Mem, id: OpId, op: &Op, entry: LogEntry) -> Result<()> {
    let ancestors = mem.ancestors.get(&id).cloned().unwrap_or_default();
    let births: Vec<OpId> = mem
        .issues
        .get(&op.issue)
        .map(|lives| lives.iter().map(|live| live.born_id).collect())
        .unwrap_or_default();
    let causal: Vec<OpId> = births
        .iter()
        .copied()
        .filter(|birth| ancestors.contains(birth))
        .collect();
    if causal.is_empty() {
        mem.pending_logs
            .entry(op.issue)
            .or_default()
            .push(entry.clone());
    }
    let targets = if causal.is_empty() { births } else { causal };
    for birth in targets {
        if let Some(lives) = mem.issues.get_mut(&op.issue)
            && let Some(live) = lives.iter_mut().find(|live| live.born_id == birth)
        {
            if !live.heading.logbook.iter().any(|have| have == &entry) {
                live.heading.logbook.push(entry.clone());
            }
            live.heading.logbook.sort_by(log_order);
        }
    }
    Ok(())
}

fn log_order(a: &LogEntry, b: &LogEntry) -> Ordering {
    b.timestamp
        .cmp(&a.timestamp)
        .then_with(|| a.render().cmp(&b.render()))
}

fn add_deed(mem: &mut Mem, id: OpId, op: &Op, accession: &str) -> Result<()> {
    write_union(mem, id, op, "property:DEEDS", accession)
}

fn add_link(mem: &mut Mem, id: OpId, op: &Op, kind: &str, other: &str) -> Result<()> {
    if kind == "blocked-by" {
        return write_union(mem, id, op, "property:BLOCKED_BY", other);
    }
    write_union(mem, id, op, &keyed("link:", kind, other), other)
}

fn one_char(value: &str) -> Result<char> {
    let mut chars = value.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Ok(ch),
        _ => Err(bad("priority is one character")),
    }
}

fn load_votes(heading: &IssueHeading) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut votes = BTreeMap::new();
    let mut raw = BTreeSet::new();
    for line in drawer_lines(heading, "VOTES") {
        if let Some(ballot) = crate::ops::parse_ballot(&line) {
            let rev = ballot.rev.unwrap_or_default();
            if token_ok(&ballot.agent) && (rev.is_empty() || token_ok(&rev)) {
                votes.insert(keyed("ballot:", &ballot.agent, &rev), line);
                continue;
            }
        }
        raw.insert(line);
    }
    (votes, raw)
}

fn load_revisions(heading: &IssueHeading) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let mut revisions = BTreeMap::new();
    let mut raw = BTreeSet::new();
    for line in drawer_lines(heading, "REVISIONS") {
        if let Some((id, summary)) = structured_revision(&line) {
            revisions.insert(id, summary);
        } else {
            raw.insert(line);
        }
    }
    (revisions, raw)
}

fn structured_revision(line: &str) -> Option<(String, String)> {
    if let Some((id, summary)) = line.split_once(' ') {
        if token_ok(id)
            && !summary.is_empty()
            && !summary
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control())
        {
            return Some((id.to_string(), summary.to_string()));
        }
        return None;
    }
    if token_ok(line) {
        Some((line.to_string(), String::new()))
    } else {
        None
    }
}

fn load_links(heading: &IssueHeading) -> (BTreeSet<(String, String)>, BTreeSet<String>) {
    let mut links = BTreeSet::new();
    let mut raw = BTreeSet::new();
    for line in drawer_lines(heading, "LINKS") {
        if let Some(pair) = structured_link(&line) {
            links.insert(pair);
        } else {
            raw.insert(line);
        }
    }
    (links, raw)
}

fn structured_link(line: &str) -> Option<(String, String)> {
    let (kind, other) = line.split_once(' ')?;
    if token_ok(kind) && token_ok(other) {
        Some((kind.to_string(), other.to_string()))
    } else {
        None
    }
}

fn load_targets(
    heading: &IssueHeading,
) -> (
    BTreeMap<(String, String), String>,
    Option<String>,
    BTreeSet<String>,
) {
    let mut targets = BTreeMap::new();
    let mut retarget = None;
    let mut raw = BTreeSet::new();
    for line in drawer_lines(heading, "TARGETS") {
        if let Some(id) = line.strip_prefix("retarget ") {
            if token_ok(id) {
                retarget = Some(id.to_string());
            } else {
                raw.insert(line);
            }
            continue;
        }
        if let Some((target, key, value)) = structured_target(&line) {
            targets.insert((target, key), value);
            continue;
        }
        raw.insert(line);
    }
    (targets, retarget, raw)
}

fn structured_target(line: &str) -> Option<(String, String, String)> {
    let (target, rest) = line.split_once(' ')?;
    let (key, value) = rest.split_once(' ')?;
    if token_ok_colon(target) && token_ok_colon(key) && !value.chars().any(char::is_control) {
        Some((target.to_string(), key.to_string(), value.to_string()))
    } else {
        None
    }
}

fn push_raw(raw: &BTreeMap<String, BTreeSet<String>>, drawer: &str, lines: &mut Vec<String>) {
    let Some(extra) = raw.get(drawer) else {
        return;
    };
    lines.extend(extra.iter().cloned());
}

fn kept_lines(
    votes: BTreeSet<String>,
    revisions: BTreeSet<String>,
    links: BTreeSet<String>,
    targets: BTreeSet<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut raw = BTreeMap::new();
    if !votes.is_empty() {
        raw.insert("VOTES".to_string(), votes);
    }
    if !revisions.is_empty() {
        raw.insert("REVISIONS".to_string(), revisions);
    }
    if !links.is_empty() {
        raw.insert("LINKS".to_string(), links);
    }
    if !targets.is_empty() {
        raw.insert("TARGETS".to_string(), targets);
    }
    raw
}

fn token_ok(value: &str) -> bool {
    !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || ch == '"' || ch == '\\')
}

fn token_ok_colon(value: &str) -> bool {
    token_ok(value) && !value.contains(':')
}

fn spellable(field: &str, value: &str) -> Result<()> {
    if token_ok(value) {
        Ok(())
    } else {
        Err(bad(format!("{field} {value:?}")))
    }
}

fn spellable_colon(field: &str, value: &str) -> Result<()> {
    if token_ok_colon(value) {
        Ok(())
    } else {
        Err(bad(format!("{field} {value:?}")))
    }
}

fn spellable_choice(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(bad(format!("choice {value:?}")));
    }
    Ok(())
}

/// Name of one `(left, right)` pair.
///
/// A colon is legal in a voter, a revision, a link kind, and an id. The name
/// records how long the left side is, so two different pairs do not share a
/// name. Drawer lines never use this spelling.
fn keyed(prefix: &str, left: &str, right: &str) -> String {
    format!("{prefix}{}:{left}{right}", left.chars().count())
}

fn unkey(prefix: &str, name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix(prefix)?;
    let (len, tail) = rest.split_once(':')?;
    let n: usize = len.parse().ok()?;
    if tail.chars().count() < n {
        return None;
    }
    let left: String = tail.chars().take(n).collect();
    let right: String = tail.chars().skip(n).collect();
    Some((left, right))
}

fn split_target(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix("target:")?;
    let (target, key) = rest.split_once(':')?;
    if token_ok_colon(target) && token_ok_colon(key) {
        Some((target.to_string(), key.to_string()))
    } else {
        None
    }
}

/// Ids in a property list. A doubled space is not two ids: the base reader
/// would erase it, and a legacy line keeps the spelling it was given.
fn list_elements(value: &str) -> Vec<String> {
    if value.contains("  ") || value.chars().any(|ch| ch.is_whitespace() && ch != ' ') {
        return if value.is_empty() {
            Vec::new()
        } else {
            vec![value.to_string()]
        };
    }
    crate::org::split_id_list(value)
}

fn vote_line(rev: &str, stamp: &str, voter: &str, choice: &str) -> String {
    let mut line = String::new();
    if !rev.is_empty() {
        line.push_str("[rev=");
        line.push_str(rev);
        line.push_str("] ");
    }
    line.push_str(stamp);
    line.push(' ');
    line.push_str(voter);
    line.push_str(": ");
    line.push_str(choice);
    line
}

fn revision_line(rev: &str, summary: &str) -> String {
    if summary.is_empty() {
        rev.to_string()
    } else {
        format!("{rev} {summary}")
    }
}

fn link_line(kind: &str, other: &str) -> String {
    format!("{kind} {other}")
}

fn target_field_line(target: &str, key: &str, value: &str) -> String {
    format!("{target} {key} {value}")
}

fn retarget_line(target: &str) -> String {
    format!("retarget {target}")
}

fn require_derived(project: &str, short_id: &str, key: IssueKey) -> Result<()> {
    if key != IssueKey::derive(project, short_id) {
        return Err(bad("an issue key is not derived from its project and id"));
    }
    Ok(())
}

fn check_op(op: &Op) -> Result<()> {
    match &op.kind {
        OpKind::Create { short_id, .. } => {
            spellable("id", short_id)?;
            require_derived(&op.project, short_id, op.issue)
        }
        OpKind::GenesisImport { heading, .. } => {
            spellable("id", &heading.id)?;
            require_derived(&op.project, &heading.id, op.issue)
        }
        OpKind::SetField { name, value } | OpKind::Resolve { field: name, value } => {
            canonical_field(name)?;
            if name == "retarget" {
                spellable("retarget", value)?;
            }
            Ok(())
        }
        OpKind::Ballot {
            voter,
            rev,
            choice,
            stamp,
        } => {
            spellable("voter", voter)?;
            if !rev.is_empty() {
                spellable("revision", rev)?;
            }
            spellable_choice(choice)?;
            if stamp.is_empty() || stamp.chars().any(char::is_control) {
                return Err(bad(format!("stamp {stamp:?}")));
            }
            Ok(())
        }
        OpKind::AddRevision { rev_id, summary } => {
            spellable("revision", rev_id)?;
            if !summary.is_empty()
                && summary
                    .chars()
                    .any(|ch| ch.is_whitespace() || ch.is_control())
            {
                return Err(bad(format!("summary {summary:?}")));
            }
            Ok(())
        }
        OpKind::SetTarget { target_id, fields } => {
            spellable_colon("target id", target_id)?;
            for (name, value) in fields {
                spellable_colon("target field", name)?;
                if value.chars().any(char::is_control) {
                    return Err(bad(format!("target field {value:?}")));
                }
            }
            Ok(())
        }
        OpKind::Retarget { target_id } => spellable("retarget", target_id),
        OpKind::AddDeed { accession } => spellable("deed", accession),
        OpKind::Link { kind, other } => {
            spellable("link kind", kind)?;
            spellable("id", other)?;
            Ok(())
        }
        OpKind::AppendLog { .. } | OpKind::SetBody { .. } | OpKind::Retire { .. } => Ok(()),
    }
}

impl Live {
    fn from_op(op: &Op, short_id: &str, heading: IssueHeading, id: OpId) -> Self {
        let (ballots, raw_votes) = load_votes(&heading);
        let (revisions, raw_revisions) = load_revisions(&heading);
        let (links, raw_links) = load_links(&heading);
        let (targets, retarget, raw_targets) = load_targets(&heading);
        let raw = kept_lines(raw_votes, raw_revisions, raw_links, raw_targets);
        let order_fixed = heading.property_order.len();
        Self {
            project: op.project.clone(),
            short_id: short_id.to_string(),
            heading,
            links,
            ballots,
            revisions,
            targets,
            retarget,
            raw,
            born_lamport: op.lamport,
            born_id: id,
            order_fixed,
        }
    }
}

fn hex16(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(32);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

// --- canonical Cap'n Proto segment -----------------------------------------
//
// `schema/op.capnp` laid out by Cap'n Proto 1.0.1:
// Op data is lamport at byte 0 (u64), kind at byte 8 (u16), wall at byte 16 (i64).
// Op pointers are parents, seat, project, issue, payload.
// Payload pointers follow the field order in the schema. Trailing default
// words are omitted, which is the canonical form.

fn encode_op(op: &Op) -> Result<Vec<u8>> {
    let node = op_node(op)?;
    let mut enc = Enc { words: vec![0] };
    let place = enc.emit(&node)?;
    let (data, ptrs) = match place.meta {
        Meta::Struct { data, ptrs } => (data, ptrs),
        _ => return Err(bad("the root of an op is a struct")),
    };
    enc.words[0] = struct_ptr(0, place.at, data, ptrs)?;
    Ok(words_to_bytes(&enc.words))
}

fn op_node(op: &Op) -> Result<Node> {
    let mut data = vec![0u64; 3];
    data[0] = op.lamport;
    data[1] = u64::from(kind_code(&op.kind));
    data[2] = op.wall as u64;
    let ptrs = vec![
        parents_node(&op.parents),
        Some(text_node(&op.seat)?),
        Some(text_node(&op.project)?),
        Some(Node::Bytes(op.issue.0.to_vec())),
        Some(payload_node(&op.kind)?),
    ];
    Ok(struct_node(data, ptrs))
}

fn kind_code(kind: &OpKind) -> u16 {
    match kind {
        OpKind::Create { .. } => 0,
        OpKind::SetField { .. } => 1,
        OpKind::AppendLog { .. } => 2,
        OpKind::Ballot { .. } => 3,
        OpKind::AddRevision { .. } => 4,
        OpKind::SetTarget { .. } => 5,
        OpKind::Retarget { .. } => 6,
        OpKind::AddDeed { .. } => 7,
        OpKind::Link { .. } => 8,
        OpKind::SetBody { .. } => 9,
        OpKind::Retire { .. } => 10,
        OpKind::Resolve { .. } => 11,
        OpKind::GenesisImport { .. } => 12,
    }
}

fn payload_node(kind: &OpKind) -> Result<Node> {
    let mut ptrs = Vec::with_capacity(13);
    for _ in 0..13 {
        ptrs.push(None);
    }
    let (index, node) = match kind {
        OpKind::Create {
            short_id,
            title,
            state,
            priority,
        } => (0, texts(&[short_id, title, state, priority])?),
        OpKind::SetField { name, value } => (1, texts(&[name, value])?),
        OpKind::AppendLog { entry } => (2, log_node(entry)?),
        OpKind::Ballot {
            voter,
            rev,
            choice,
            stamp,
        } => (3, texts(&[voter, rev, choice, stamp])?),
        OpKind::AddRevision { rev_id, summary } => (4, texts(&[rev_id, summary])?),
        OpKind::SetTarget { target_id, fields } => (5, target_node(target_id, fields)?),
        OpKind::Retarget { target_id } => (6, texts(&[target_id.as_str()])?),
        OpKind::AddDeed { accession } => (7, texts(&[accession.as_str()])?),
        OpKind::Link { kind, other } => (8, texts(&[kind, other])?),
        OpKind::SetBody { body } => (9, text_node(body)?),
        OpKind::Retire { reason } => (10, text_node(reason)?),
        OpKind::Resolve { field, value } => (11, texts(&[field, value])?),
        OpKind::GenesisImport { heading, residue } => (12, genesis_node(heading, residue)?),
    };
    ptrs[index] = Some(node);
    Ok(struct_node(Vec::new(), ptrs))
}

fn texts(values: &[&str]) -> Result<Node> {
    let mut ptrs = Vec::with_capacity(values.len());
    for value in values {
        ptrs.push(Some(text_node(value)?));
    }
    Ok(struct_node(Vec::new(), ptrs))
}

fn log_node(entry: &LogEntry) -> Result<Node> {
    Ok(struct_node(
        Vec::new(),
        vec![
            Some(text_node(&entry.timestamp)?),
            maybe_text(entry.from_state.as_deref())?,
            maybe_text(entry.to_state.as_deref())?,
            maybe_text(entry.note.as_deref())?,
            maybe_text(entry.raw.as_deref())?,
        ],
    ))
}

fn target_node(target_id: &str, fields: &[(String, String)]) -> Result<Node> {
    Ok(struct_node(
        Vec::new(),
        vec![Some(text_node(target_id)?), field_list(fields)?],
    ))
}

fn field_list(fields: &[(String, String)]) -> Result<Option<Node>> {
    if fields.is_empty() {
        return Ok(None);
    }
    let mut elems = Vec::with_capacity(fields.len());
    for (key, value) in fields {
        elems.push(vec![Some(text_node(key)?), Some(text_node(value)?)]);
    }
    Ok(Some(Node::StructList {
        ptr_words: 2,
        elems,
    }))
}

fn genesis_node(heading: &IssueHeading, residue: &str) -> Result<Node> {
    if heading.line_start > u32::MAX as usize || heading.line_end > u32::MAX as usize {
        return Err(bad("a heading line number does not fit"));
    }
    let mut data = vec![0u64; 1];
    data[0] = heading.line_start as u64 | ((heading.line_end as u64) << 32);
    let mut props: Vec<(String, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    for key in &heading.property_order {
        if let Some(value) = heading.properties.get(key) {
            props.push((key.clone(), value.clone()));
            seen.insert(key.clone());
        }
    }
    for (key, value) in &heading.properties {
        if seen.insert(key.clone()) {
            props.push((key.clone(), value.clone()));
        }
    }
    Ok(struct_node(
        data,
        vec![
            Some(text_node(&heading.id)?),
            Some(text_node(&heading.title)?),
            Some(text_node(&heading.state)?),
            Some(text_node(&heading.priority.to_string())?),
            field_list(&props)?,
            text_list(&heading.org_tags)?,
            maybe_text(heading.statistics.as_deref())?,
            text_list(&heading.property_order)?,
            text_list(&heading.extra_drawers)?,
            Some(text_node(&heading.body)?),
            log_list(&heading.logbook)?,
            maybe_text(if residue.is_empty() {
                None
            } else {
                Some(residue)
            })?,
        ],
    ))
}

fn log_list(entries: &[LogEntry]) -> Result<Option<Node>> {
    if entries.is_empty() {
        return Ok(None);
    }
    let mut built = Vec::with_capacity(entries.len());
    let mut width = 0usize;
    for entry in entries {
        let ptrs = vec![
            Some(text_node(&entry.timestamp)?),
            maybe_text(entry.from_state.as_deref())?,
            maybe_text(entry.to_state.as_deref())?,
            maybe_text(entry.note.as_deref())?,
            maybe_text(entry.raw.as_deref())?,
        ];
        let need = ptrs.iter().rposition(Option::is_some).map_or(0, |i| i + 1);
        width = width.max(need);
        built.push(ptrs);
    }
    let elems = built
        .into_iter()
        .map(|mut ptrs| {
            ptrs.resize_with(width, || None);
            ptrs
        })
        .collect();
    Ok(Some(Node::StructList {
        ptr_words: width,
        elems,
    }))
}

fn text_list(items: &[String]) -> Result<Option<Node>> {
    if items.is_empty() {
        return Ok(None);
    }
    let mut nodes = Vec::with_capacity(items.len());
    for item in items {
        nodes.push(text_node(item)?);
    }
    Ok(Some(Node::PtrList(nodes)))
}

fn parents_node(ids: &[OpId]) -> Option<Node> {
    if ids.is_empty() {
        return None;
    }
    Some(Node::PtrList(
        ids.iter().map(|id| Node::Bytes(id.0.to_vec())).collect(),
    ))
}

fn text_node(text: &str) -> Result<Node> {
    if text.as_bytes().contains(&0) {
        return Err(bad("a text field contains a zero byte"));
    }
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    Ok(Node::Bytes(bytes))
}

fn maybe_text(text: Option<&str>) -> Result<Option<Node>> {
    text.map(text_node).transpose()
}

fn struct_node(mut data: Vec<u64>, mut ptrs: Vec<Option<Node>>) -> Node {
    while data.last() == Some(&0) {
        data.pop();
    }
    while ptrs.last().is_some_and(Option::is_none) {
        ptrs.pop();
    }
    Node::Struct {
        data,
        ptrs: ptrs.into_iter().map(|slot| slot.map(Box::new)).collect(),
    }
}

enum Node {
    Struct {
        data: Vec<u64>,
        ptrs: Vec<Option<Box<Node>>>,
    },
    Bytes(Vec<u8>),
    PtrList(Vec<Node>),
    StructList {
        ptr_words: usize,
        elems: Vec<Vec<Option<Node>>>,
    },
}

struct Enc {
    words: Vec<u64>,
}

struct Place {
    at: usize,
    meta: Meta,
}

enum Meta {
    Struct { data: u16, ptrs: u16 },
    Bytes { len: u32 },
    PtrList { count: u32 },
    Composite { words: u32 },
}

impl Enc {
    fn emit(&mut self, node: &Node) -> Result<Place> {
        match node {
            Node::Struct { data, ptrs } => self.emit_struct(data, ptrs),
            Node::Bytes(bytes) => Ok(self.emit_bytes(bytes)),
            Node::PtrList(items) => self.emit_ptr_list(items),
            Node::StructList { ptr_words, elems } => self.emit_struct_list(*ptr_words, elems),
        }
    }

    fn emit_struct(&mut self, data: &[u64], ptrs: &[Option<Box<Node>>]) -> Result<Place> {
        let start = self.words.len();
        self.words.extend_from_slice(data);
        let ptr_at = self.words.len();
        self.words.resize(ptr_at + ptrs.len(), 0);
        for (index, child) in ptrs.iter().enumerate() {
            if let Some(child) = child {
                let place = self.emit(child)?;
                self.words[ptr_at + index] = point_at(ptr_at + index, &place)?;
            }
        }
        Ok(Place {
            at: start,
            meta: Meta::Struct {
                data: u16::try_from(data.len()).map_err(|_| bad("struct data is too wide"))?,
                ptrs: u16::try_from(ptrs.len()).map_err(|_| bad("struct pointers are too wide"))?,
            },
        })
    }

    fn emit_bytes(&mut self, bytes: &[u8]) -> Place {
        let start = self.words.len();
        if !bytes.is_empty() {
            let mut padded = bytes.to_vec();
            while !padded.len().is_multiple_of(8) {
                padded.push(0);
            }
            for chunk in padded.chunks(8) {
                let mut word = [0u8; 8];
                word.copy_from_slice(chunk);
                self.words.push(u64::from_le_bytes(word));
            }
        }
        Place {
            at: start,
            meta: Meta::Bytes {
                len: bytes.len() as u32,
            },
        }
    }

    fn emit_ptr_list(&mut self, items: &[Node]) -> Result<Place> {
        let start = self.words.len();
        self.words.resize(start + items.len(), 0);
        for (index, item) in items.iter().enumerate() {
            let place = self.emit(item)?;
            self.words[start + index] = point_at(start + index, &place)?;
        }
        Ok(Place {
            at: start,
            meta: Meta::PtrList {
                count: items.len() as u32,
            },
        })
    }

    fn emit_struct_list(&mut self, ptr_words: usize, elems: &[Vec<Option<Node>>]) -> Result<Place> {
        let start = self.words.len();
        let stride = ptr_words;
        self.words.resize(start + 1 + elems.len() * stride, 0);
        self.words[start] = list_tag(elems.len() as u32, 0, ptr_words as u16)?;
        for (index, elem) in elems.iter().enumerate() {
            let base = start + 1 + index * stride;
            for (slot, child) in elem.iter().enumerate() {
                if let Some(child) = child {
                    let place = self.emit(child)?;
                    self.words[base + slot] = point_at(base + slot, &place)?;
                }
            }
        }
        Ok(Place {
            at: start,
            meta: Meta::Composite {
                words: (elems.len() * stride) as u32,
            },
        })
    }
}

fn point_at(from: usize, place: &Place) -> Result<u64> {
    match place.meta {
        Meta::Struct { data, ptrs } => {
            if data == 0 && ptrs == 0 {
                return Ok(zero_struct_ptr());
            }
            struct_ptr(from, place.at, data, ptrs)
        }
        Meta::Bytes { len } => list_ptr(from, place.at, 2, len),
        Meta::PtrList { count } => list_ptr(from, place.at, 6, count),
        Meta::Composite { words } => list_ptr(from, place.at, 7, words),
    }
}

fn struct_ptr(from: usize, to: usize, data: u16, ptrs: u16) -> Result<u64> {
    let offset = word_offset(from, to)?;
    Ok(((offset as u32 as u64) & 0x3fff_ffff) << 2
        | (u64::from(data) << 32)
        | (u64::from(ptrs) << 48))
}

fn list_ptr(from: usize, to: usize, esize: u8, count: u32) -> Result<u64> {
    if count >= 1 << 29 {
        return Err(bad("a list in an op is too long"));
    }
    let offset = word_offset(from, to)?;
    Ok(1 | ((offset as u32 as u64) & 0x3fff_ffff) << 2
        | (u64::from(esize) << 32)
        | (u64::from(count) << 35))
}

fn list_tag(count: u32, data: u16, ptrs: u16) -> Result<u64> {
    if count >= 1 << 29 {
        return Err(bad("a list in an op is too long"));
    }
    Ok(((u64::from(count)) & 0x3fff_ffff) << 2 | (u64::from(data) << 32) | (u64::from(ptrs) << 48))
}

fn zero_struct_ptr() -> u64 {
    let offset: i32 = -1;
    (offset as u32 as u64 & 0x3fff_ffff) << 2
}

fn word_offset(from: usize, to: usize) -> Result<i32> {
    let offset = to as i64 - from as i64 - 1;
    if !(-(1 << 29)..1 << 29).contains(&offset) {
        return Err(bad("op is too large to encode"));
    }
    Ok(offset as i32)
}

fn words_to_bytes(words: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 8);
    for word in words {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out
}

fn decode_segment(bytes: &[u8]) -> Result<Op> {
    if !bytes.len().is_multiple_of(8) {
        return Err(bad("op segment is not a whole number of words"));
    }
    let mut words = Vec::with_capacity(bytes.len() / 8);
    for chunk in bytes.chunks(8) {
        let mut word = [0u8; 8];
        word.copy_from_slice(chunk);
        words.push(u64::from_le_bytes(word));
    }
    let seg = Seg {
        words: &words,
        budget: Cell::new(words.len().saturating_mul(TRAVERSAL_MULTIPLE)),
    };
    let root = seg.read_ptr(0)?;
    let Ptr::Struct(st) = root else {
        return Err(bad("an op root is not a struct"));
    };
    let data = seg.data_of(&st);
    let lamport = data.first().copied().unwrap_or(0);
    let kind = data.get(1).copied().unwrap_or(0) as u16;
    let wall = data.get(2).copied().unwrap_or(0) as i64;
    let parents = seg.parents(&st)?;
    let seat = seg.req_text(&st, 1)?;
    let project = seg.req_text(&st, 2)?;
    let issue = seg.issue_key(&st)?;
    let payload = match seg.ptr_of(&st, 4)? {
        Ptr::Struct(payload) => payload,
        Ptr::Null => return Err(bad("an op has no payload")),
        _ => return Err(bad("an op payload is not a struct")),
    };
    let kind = seg.kind(kind, &payload)?;
    let op = Op {
        parents,
        lamport,
        seat,
        project,
        issue,
        kind,
        wall,
    };
    let again = encode_op(&op)?;
    if again.as_slice() != bytes {
        return Err(bad("op segment is not canonical"));
    }
    check_op(&op)?;
    Ok(op)
}

// Cap'n Proto counts every pointer follow against one traversal limit so
// overlapping pointers cannot amplify a message. The encoding spec allows
// that limit to be a small multiple of the message; eight covers one tree
// walk, since a canonical op reads each word at most once.
const TRAVERSAL_MULTIPLE: usize = 8;

struct Seg<'a> {
    words: &'a [u64],
    budget: Cell<usize>,
}

struct St {
    at: usize,
    data: usize,
    ptrs: usize,
}

enum Ptr {
    Null,
    Struct(St),
    Bytes(Vec<u8>),
    Words { at: usize, count: usize },
    Composite { at: usize, words: usize },
}

impl<'a> Seg<'a> {
    fn read_ptr(&self, idx: usize) -> Result<Ptr> {
        let word = *self
            .words
            .get(idx)
            .ok_or_else(|| bad("pointer past the end of the op"))?;
        if word == 0 {
            return Ok(Ptr::Null);
        }
        let tag = word & 3;
        let offset = sext30(word >> 2);
        let at = target(idx, offset)?;
        match tag {
            0 => {
                let data = ((word >> 32) & 0xffff) as usize;
                let ptrs = ((word >> 48) & 0xffff) as usize;
                if data == 0 && ptrs == 0 {
                    if offset == -1 {
                        return Ok(Ptr::Struct(St {
                            at: idx,
                            data: 0,
                            ptrs: 0,
                        }));
                    }
                    return Err(bad("zero-size struct is not canonical"));
                }
                self.need(at, data + ptrs)?;
                self.charge(data + ptrs)?;
                Ok(Ptr::Struct(St { at, data, ptrs }))
            }
            1 => self.read_list(at, ((word >> 32) & 7) as u8, (word >> 35) as u32),
            _ => Err(bad("an op pointer is not canonical")),
        }
    }

    fn read_list(&self, at: usize, esize: u8, count: u32) -> Result<Ptr> {
        match esize {
            2 => {
                let nbytes = count as usize;
                let nwords = nbytes.div_ceil(8);
                self.need(at, nwords)?;
                // Charge before the copy. A second pointer at these words counts
                // again, so aliases stop here instead of multiplying the blob.
                self.charge(nwords)?;
                let mut buf = Vec::with_capacity(nwords * 8);
                for word in &self.words[at..at + nwords] {
                    buf.extend_from_slice(&word.to_le_bytes());
                }
                buf.truncate(nbytes);
                Ok(Ptr::Bytes(buf))
            }
            6 => {
                let n = count as usize;
                self.need(at, n)?;
                self.charge(n)?;
                Ok(Ptr::Words { at, count: n })
            }
            7 => {
                let n = count as usize;
                self.need(at, 1 + n)?;
                self.charge(1 + n)?;
                Ok(Ptr::Composite { at, words: n })
            }
            _ => Err(bad("unsupported list element in an op")),
        }
    }

    fn charge(&self, words: usize) -> Result<()> {
        let left = self.budget.get();
        if words > left {
            return Err(bad("op segment is not canonical"));
        }
        self.budget.set(left - words);
        Ok(())
    }

    fn need(&self, at: usize, words: usize) -> Result<()> {
        if at
            .checked_add(words)
            .is_some_and(|end| end <= self.words.len())
        {
            Ok(())
        } else {
            Err(bad("an op pointer lands outside the segment"))
        }
    }

    fn ptr_of(&self, st: &St, index: usize) -> Result<Ptr> {
        if index >= st.ptrs {
            return Ok(Ptr::Null);
        }
        self.read_ptr(st.at + st.data + index)
    }

    fn data_of(&self, st: &St) -> &'a [u64] {
        &self.words[st.at..st.at + st.data]
    }

    fn req_text(&self, st: &St, index: usize) -> Result<String> {
        self.opt_text(st, index)?
            .ok_or_else(|| bad("a required text field is missing"))
    }

    fn opt_text(&self, st: &St, index: usize) -> Result<Option<String>> {
        match self.ptr_of(st, index)? {
            Ptr::Null => Ok(None),
            Ptr::Bytes(data) => Ok(Some(as_text(data)?)),
            _ => Err(bad("a text field is not text")),
        }
    }

    fn parents(&self, st: &St) -> Result<Vec<OpId>> {
        match self.ptr_of(st, 0)? {
            Ptr::Null => Ok(Vec::new()),
            Ptr::Words { at, count } => {
                let mut ids = Vec::with_capacity(count);
                for index in 0..count {
                    match self.read_ptr(at + index)? {
                        Ptr::Bytes(data) if data.len() == 32 => {
                            let mut bytes = [0u8; 32];
                            bytes.copy_from_slice(&data);
                            ids.push(OpId(bytes));
                        }
                        _ => return Err(bad("a parent id is not 32 bytes")),
                    }
                }
                Ok(ids)
            }
            _ => Err(bad("parents are not a list of ids")),
        }
    }

    fn issue_key(&self, st: &St) -> Result<IssueKey> {
        match self.ptr_of(st, 3)? {
            Ptr::Bytes(data) if data.len() == 16 => {
                let mut bytes = [0u8; 16];
                bytes.copy_from_slice(&data);
                Ok(IssueKey(bytes))
            }
            _ => Err(bad("an issue key is not 16 bytes")),
        }
    }

    fn kind(&self, code: u16, payload: &St) -> Result<OpKind> {
        match code {
            0 => Ok(OpKind::Create {
                short_id: self.nested_text(payload, 0, 0)?,
                title: self.nested_text(payload, 0, 1)?,
                state: self.nested_text(payload, 0, 2)?,
                priority: self.nested_text(payload, 0, 3)?,
            }),
            1 => {
                let (name, value) = self.field_at(payload, 1)?;
                Ok(OpKind::SetField { name, value })
            }
            2 => Ok(OpKind::AppendLog {
                entry: self.log_at(payload, 2)?,
            }),
            3 => {
                let st = self.nested(payload, 3)?;
                Ok(OpKind::Ballot {
                    voter: self.req_text(&st, 0)?,
                    rev: self.req_text(&st, 1)?,
                    choice: self.req_text(&st, 2)?,
                    stamp: self.req_text(&st, 3)?,
                })
            }
            4 => {
                let st = self.nested(payload, 4)?;
                Ok(OpKind::AddRevision {
                    rev_id: self.req_text(&st, 0)?,
                    summary: self.req_text(&st, 1)?,
                })
            }
            5 => self.target_at(payload),
            6 => Ok(OpKind::Retarget {
                target_id: self.nested_text(payload, 6, 0)?,
            }),
            7 => Ok(OpKind::AddDeed {
                accession: self.nested_text(payload, 7, 0)?,
            }),
            8 => {
                let st = self.nested(payload, 8)?;
                Ok(OpKind::Link {
                    kind: self.req_text(&st, 0)?,
                    other: self.req_text(&st, 1)?,
                })
            }
            9 => Ok(OpKind::SetBody {
                body: self.req_text(payload, 9)?,
            }),
            10 => Ok(OpKind::Retire {
                reason: self.req_text(payload, 10)?,
            }),
            11 => {
                let (field, value) = self.field_at(payload, 11)?;
                Ok(OpKind::Resolve { field, value })
            }
            12 => Ok(OpKind::GenesisImport {
                heading: Box::new(self.genesis(payload)?),
                residue: self
                    .nested(payload, 12)
                    .and_then(|st| self.opt_text(&st, 11))?
                    .unwrap_or_default(),
            }),
            _ => Err(bad("unknown op kind")),
        }
    }

    fn nested(&self, payload: &St, index: usize) -> Result<St> {
        match self.ptr_of(payload, index)? {
            Ptr::Struct(st) => Ok(st),
            _ => Err(bad("an op payload field is missing")),
        }
    }

    fn nested_text(&self, payload: &St, index: usize, field: usize) -> Result<String> {
        let st = self.nested(payload, index)?;
        self.req_text(&st, field)
    }

    fn field_at(&self, payload: &St, index: usize) -> Result<(String, String)> {
        let st = self.nested(payload, index)?;
        Ok((self.req_text(&st, 0)?, self.req_text(&st, 1)?))
    }

    fn log_at(&self, payload: &St, index: usize) -> Result<LogEntry> {
        let st = self.nested(payload, index)?;
        Ok(LogEntry {
            timestamp: self.req_text(&st, 0)?,
            from_state: self.opt_text(&st, 1)?,
            to_state: self.opt_text(&st, 2)?,
            note: self.opt_text(&st, 3)?,
            raw: self.opt_text(&st, 4)?,
        })
    }

    fn target_at(&self, payload: &St) -> Result<OpKind> {
        let st = self.nested(payload, 5)?;
        let target_id = self.req_text(&st, 0)?;
        let fields = match self.ptr_of(&st, 1)? {
            Ptr::Null => Vec::new(),
            Ptr::Composite { at, words } => self
                .elements(at, words)?
                .into_iter()
                .map(|elem| Ok((self.req_text(&elem, 0)?, self.req_text(&elem, 1)?)))
                .collect::<Result<Vec<_>>>()?,
            _ => return Err(bad("target fields are not a list")),
        };
        Ok(OpKind::SetTarget { target_id, fields })
    }

    fn genesis(&self, payload: &St) -> Result<IssueHeading> {
        let st = self.nested(payload, 12)?;
        let word = self.data_of(&st).first().copied().unwrap_or(0);
        let priority = self.req_text(&st, 3)?;
        let properties_list = self.pairs(&st, 4)?;
        let mut properties = BTreeMap::new();
        for (key, value) in properties_list {
            properties.insert(key, value);
        }
        Ok(IssueHeading {
            id: self.req_text(&st, 0)?,
            title: self.req_text(&st, 1)?,
            state: self.req_text(&st, 2)?,
            priority: one_char(&priority)?,
            properties,
            org_tags: self.text_list(&st, 5)?,
            statistics: self.opt_text(&st, 6)?,
            property_order: self.text_list(&st, 7)?,
            extra_drawers: self.text_list(&st, 8)?,
            body: self.req_text(&st, 9)?,
            logbook: self.logs(&st)?,
            line_start: (word & 0xffff_ffff) as usize,
            line_end: (word >> 32) as usize,
        })
    }

    fn pairs(&self, st: &St, index: usize) -> Result<Vec<(String, String)>> {
        match self.ptr_of(st, index)? {
            Ptr::Null => Ok(Vec::new()),
            Ptr::Composite { at, words } => self
                .elements(at, words)?
                .into_iter()
                .map(|elem| Ok((self.req_text(&elem, 0)?, self.req_text(&elem, 1)?)))
                .collect(),
            _ => Err(bad("a field list is not a list of structs")),
        }
    }

    fn logs(&self, st: &St) -> Result<Vec<LogEntry>> {
        match self.ptr_of(st, 10)? {
            Ptr::Null => Ok(Vec::new()),
            Ptr::Composite { at, words } => self
                .elements(at, words)?
                .into_iter()
                .map(|elem| {
                    Ok(LogEntry {
                        timestamp: self.opt_text(&elem, 0)?.unwrap_or_default(),
                        from_state: self.opt_text(&elem, 1)?,
                        to_state: self.opt_text(&elem, 2)?,
                        note: self.opt_text(&elem, 3)?,
                        raw: self.opt_text(&elem, 4)?,
                    })
                })
                .collect(),
            _ => Err(bad("a logbook is not a list of lines")),
        }
    }

    fn text_list(&self, st: &St, index: usize) -> Result<Vec<String>> {
        match self.ptr_of(st, index)? {
            Ptr::Null => Ok(Vec::new()),
            Ptr::Words { at, count } => {
                let mut out = Vec::with_capacity(count);
                for index in 0..count {
                    match self.read_ptr(at + index)? {
                        Ptr::Bytes(data) => out.push(as_text(data)?),
                        _ => return Err(bad("a text list holds a non-text")),
                    }
                }
                Ok(out)
            }
            _ => Err(bad("a text list is not a list")),
        }
    }

    fn elements(&self, at: usize, words: usize) -> Result<Vec<St>> {
        let tag = self.words[at];
        if tag & 3 != 0 {
            return Err(bad("a struct list tag is not a struct description"));
        }
        let count = ((tag >> 2) & 0x3fff_ffff) as usize;
        let data = ((tag >> 32) & 0xffff) as usize;
        let ptrs = ((tag >> 48) & 0xffff) as usize;
        let stride = data + ptrs;
        // A zero stride makes the product zero for every count, so a short
        // segment can claim 2^28 elements and the reserve aborts. Charge one
        // word per element against the words still in the segment, on the
        // same counter as every other follow.
        let spanned = count
            .checked_mul(stride)
            .ok_or_else(|| bad("a struct list does not match its tag"))?;
        if spanned != words {
            return Err(bad("a struct list does not match its tag"));
        }
        if stride == 0 {
            let room = self.words.len().saturating_sub(at.saturating_add(1));
            if count > room || count > self.budget.get() {
                return Err(bad("a struct list does not match its tag"));
            }
            self.charge(count)?;
        }
        let mut out = Vec::new();
        for index in 0..count {
            out.push(St {
                at: at + 1 + index * stride,
                data,
                ptrs,
            });
        }
        Ok(out)
    }
}

fn as_text(bytes: Vec<u8>) -> Result<String> {
    let Some((&0, body)) = bytes.split_last() else {
        return Err(bad("text is missing its trailing zero"));
    };
    if body.contains(&0) {
        return Err(bad("text contains an interior zero"));
    }
    String::from_utf8(body.to_vec()).map_err(|_| bad("text is not utf-8"))
}

fn sext30(raw: u64) -> i32 {
    let value = (raw & 0x3fff_ffff) as u32;
    if value & (1 << 29) != 0 {
        (value | 0xc000_0000) as i32
    } else {
        value as i32
    }
}

fn target(idx: usize, offset: i32) -> Result<usize> {
    let at = idx as i64 + 1 + i64::from(offset);
    usize::try_from(at).map_err(|_| bad("an op pointer has a negative target"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(title: &str, lamport: u64) -> Live {
        Live {
            project: "proj".to_string(),
            short_id: "shared".to_string(),
            heading: fresh_heading("shared", title, "TODO", 'B'),
            links: BTreeSet::new(),
            ballots: BTreeMap::new(),
            revisions: BTreeMap::new(),
            targets: BTreeMap::new(),
            retarget: None,
            raw: BTreeMap::new(),
            born_lamport: lamport,
            born_id: OpId([0; 32]),
            order_fixed: 1,
        }
    }

    #[test]
    fn two_derived_keys_that_share_a_short_id_keep_both_headings() {
        let alpha = IssueKey::derive("proj", "alpha");
        let beta = IssueKey::derive("proj", "beta");
        let mut mem = Mem::default();
        mem.issues.entry(beta).or_default().push(live("Beta", 2));
        mem.issues.entry(alpha).or_default().push(live("Alpha", 1));
        let folded = mem.finish();
        assert_eq!(folded.issues.len(), 2);
        let plain = &folded.issues[&("proj".to_string(), "shared".to_string())];
        let later = &folded.issues[&("proj".to_string(), "shared~2".to_string())];
        assert_eq!(plain.title, "Alpha");
        assert_eq!(plain.id, "shared");
        assert_eq!(later.title, "Beta");
        assert_eq!(later.id, "shared~2");
    }
}
