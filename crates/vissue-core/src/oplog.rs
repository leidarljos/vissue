//! Content-addressed operations and a pure fold into today's in-memory issues.
//!
//! The bytes are the canonical Cap'n Proto encoding of `schema/op.capnp`.
//! Decode accepts a segment only when re-encoding it yields those same bytes,
//! so an op id, the sha256 of that segment, is the hash of the bytes that were
//! accepted. The segment table is redundant on a single segment and is not
//! part of the hash. Wall time is stored and is not part of the order.
//! Concurrent body edits always clash; fold does not merge them.
//! Org files stay the source of truth; nothing here writes one.

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
        Ok(Self {
            parents: ids,
            lamport: max_lamport + 1,
            seat: seat.to_string(),
            project: project.to_string(),
            issue: IssueKey::derive(project, short_id),
            kind,
            wall: 0,
        })
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
/// later `(lamport, op id)`. Concurrent body edits always clash. Two issues
/// that share a short id both stay; the later one by `(lamport, op id)` is
/// shown with a `~N` suffix.
///
/// # Errors
///
/// Returns an error when a parent is missing, the lamport clock does not
/// match the parents, the graph has a cycle, or an op names an issue that
/// has not been created.
pub fn fold(ops: &[Op]) -> Result<Folded> {
    let indexed = index(ops)?;
    let order = topo(&indexed)?;
    let mut mem = Mem::default();
    for id in order {
        let op = &indexed[&id];
        mem.ancestors.insert(id, ancestor_set(op, &mem));
        apply(&mut mem, id, op)?;
    }
    Ok(mem.finish())
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
        let expect = op
            .parents
            .iter()
            .map(|parent| map.get(parent).map(|p| p.lamport))
            .try_fold(0u64, |max, lamport| {
                lamport
                    .map(|value| max.max(value))
                    .ok_or_else(|| bad("an op names a parent that is not in the set"))
            })?
            + 1;
        if op.lamport != expect {
            return Err(bad("an op's lamport clock does not follow its parents"));
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
    issues: HashMap<IssueKey, Live>,
    ancestors: HashMap<OpId, HashSet<OpId>>,
    frontiers: HashMap<(IssueKey, String), Frontier>,
}

struct Live {
    project: String,
    short_id: String,
    heading: IssueHeading,
    links: BTreeSet<(String, String)>,
    ballots: BTreeMap<String, String>,
    targets: BTreeMap<(String, String), String>,
    retarget: Option<String>,
    born_lamport: u64,
    born_id: OpId,
}

#[derive(Default)]
struct Frontier {
    writers: Vec<Writer>,
}

struct Writer {
    id: OpId,
    lamport: u64,
    value: String,
    /// Set-union element (a deed or a link). Pure unions are not a clash.
    unite: bool,
}

impl Mem {
    fn finish(self) -> Folded {
        let mut conflicts = Vec::new();
        for ((key, field), frontier) in &self.frontiers {
            if frontier.writers.len() < 2 || frontier.writers.iter().all(|writer| writer.unite) {
                continue;
            }
            let Some(live) = self.issues.get(key) else {
                continue;
            };
            let mut writers = frontier.writers.iter().collect::<Vec<_>>();
            writers.sort_by_key(|writer| (writer.lamport, writer.id));
            conflicts.push(Conflict {
                project: live.project.clone(),
                id: live.short_id.clone(),
                field: field.clone(),
                ops: writers.into_iter().map(|writer| writer.id).collect(),
            });
        }
        conflicts.sort_by(|a, b| (&a.project, &a.id, &a.field).cmp(&(&b.project, &b.id, &b.field)));
        let mut lives: Vec<(IssueKey, Live)> = self.issues.into_iter().collect();
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
                let shown = if rank == 0 {
                    live.short_id.clone()
                } else {
                    format!("{}~{}", live.short_id, rank + 1)
                };
                let mut heading = live.heading.clone();
                if rank > 0 {
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
        OpKind::AppendLog { entry } => append_log(mem, op, entry.clone()),
        OpKind::Ballot {
            voter,
            rev,
            choice,
            stamp,
        } => {
            let line = format!("{stamp} {voter}: {choice}");
            let slot = ballot_slot(voter, rev);
            write_named(mem, id, op, &format!("ballot:{slot}"), &line)
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
    require_derived(&op.project, short_id, op.issue)?;
    if mem.issues.contains_key(&op.issue) {
        return Err(bad("an issue is created twice"));
    }
    let priority = one_char(priority)?;
    let heading = fresh_heading(short_id, title, state, priority);
    reseed(mem, op.issue, id, op.lamport, &heading);
    mem.issues
        .insert(op.issue, Live::from_op(op, short_id, heading, id));
    Ok(())
}

fn apply_genesis(mem: &mut Mem, id: OpId, op: &Op, heading: &IssueHeading) -> Result<()> {
    require_derived(&op.project, &heading.id, op.issue)?;
    let key = op.issue;
    let ancestors = mem.ancestors.get(&id).cloned().unwrap_or_default();
    let concurrent = mem
        .frontiers
        .get(&(key, "genesis".to_string()))
        .map(|frontier| {
            frontier
                .writers
                .iter()
                .any(|writer| !ancestors.contains(&writer.id))
        })
        .unwrap_or(false);
    let slot = mem
        .frontiers
        .entry((key, "genesis".to_string()))
        .or_default();
    slot.writers
        .retain(|writer| !ancestors.contains(&writer.id));
    slot.writers.push(Writer {
        id,
        lamport: op.lamport,
        value: String::new(),
        unite: false,
    });
    let winner = slot
        .writers
        .iter()
        .max_by_key(|writer| (writer.lamport, writer.id))
        .map(|writer| writer.id);
    if concurrent && winner != Some(id) {
        return Ok(());
    }
    let keys: Vec<String> = mem
        .frontiers
        .keys()
        .filter(|(issue, field)| *issue == key && field != "genesis")
        .map(|(_, field)| field.clone())
        .collect();
    for field in keys {
        mem.frontiers.remove(&(key, field));
    }
    reseed(mem, key, id, op.lamport, heading);
    mem.issues
        .insert(key, Live::from_op(op, &heading.id, heading.clone(), id));
    Ok(())
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

fn reseed(mem: &mut Mem, key: IssueKey, id: OpId, lamport: u64, heading: &IssueHeading) {
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
    for (field, value) in fields {
        mem.frontiers.insert(
            (key, field),
            Frontier {
                writers: vec![Writer {
                    id,
                    lamport,
                    value,
                    unite: false,
                }],
            },
        );
    }
    for (slot, line) in load_votes(heading) {
        mem.frontiers.insert(
            (key, format!("ballot:{slot}")),
            Frontier {
                writers: vec![Writer {
                    id,
                    lamport,
                    value: line,
                    unite: false,
                }],
            },
        );
    }
}

fn write_named(mem: &mut Mem, id: OpId, op: &Op, name: &str, value: &str) -> Result<()> {
    write_field(mem, id, op, name, value, false)
}

fn write_union(mem: &mut Mem, id: OpId, op: &Op, name: &str, element: &str) -> Result<()> {
    write_field(mem, id, op, name, element, true)
}

/// One path for every property write. A writer whose op is not an ancestor is
/// concurrent: the fold keeps both and [`Mem::finish`] records a clash, except
/// a pure set-union (deeds, links), which commutes.
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
    if !mem.issues.contains_key(&op.issue) {
        return Err(missing(op));
    }
    let ancestors = mem.ancestors.get(&id).cloned().unwrap_or_default();
    let slot = mem.frontiers.entry((op.issue, field.clone())).or_default();
    let mut base = String::new();
    let mut base_key = None;
    let mut concurrent = Vec::new();
    for writer in slot.writers.drain(..) {
        if ancestors.contains(&writer.id) {
            let key = (writer.lamport, writer.id);
            if base_key.is_none_or(|prev| key > prev) {
                base_key = Some(key);
                base = writer.value;
            }
        } else {
            concurrent.push(writer);
        }
    }
    let absorb = unite && concurrent.iter().all(|writer| writer.unite);
    let stored = if unite {
        let mut parts = tokens(Some(base.as_str()));
        if absorb {
            for writer in &concurrent {
                parts.extend(tokens(Some(writer.value.as_str())));
            }
        }
        if !value.is_empty() {
            parts.insert(value.to_string());
        }
        parts.into_iter().collect::<Vec<_>>().join(" ")
    } else {
        value.to_string()
    };
    let mut keep = concurrent;
    keep.push(Writer {
        id,
        lamport: op.lamport,
        value: stored,
        unite,
    });
    let Some(display) = keep
        .iter()
        .max_by_key(|writer| (writer.lamport, writer.id))
        .map(|writer| writer.value.clone())
    else {
        return Err(bad("a field write recorded no value"));
    };
    slot.writers = keep;
    let live = mem.issues.get_mut(&op.issue).ok_or_else(|| missing(op))?;
    project(live, &field, &display);
    Ok(())
}

fn canonical_field(name: &str) -> Result<String> {
    if matches!(
        name,
        "title" | "state" | "priority" | "body" | "tags" | "statistics" | "retarget"
    ) || name.starts_with("property:")
        || name.starts_with("ballot:")
        || name.starts_with("revision:")
        || colon_name(name, "target:")
        || colon_name(name, "link:")
    {
        Ok(name.to_string())
    } else {
        Err(bad("unknown field"))
    }
}

fn colon_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .is_some_and(|rest| rest.contains(':'))
}

fn missing(op: &Op) -> Error {
    Error::IssueNotFound {
        id: hex16(&op.issue.0),
    }
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
            live.ballots
                .insert(name["ballot:".len()..].to_string(), value.to_string());
            rewrite_ballots(live);
        }
        name if name.starts_with("revision:") => {
            let rev = &name["revision:".len()..];
            rewrite_revisions(live, rev, value);
        }
        name if name.starts_with("target:") => {
            let rest = &name["target:".len()..];
            if let Some((target, key)) = rest.split_once(':') {
                live.targets
                    .insert((target.to_string(), key.to_string()), value.to_string());
                rewrite_targets(live);
            }
        }
        name if name.starts_with("link:") => {
            let rest = &name["link:".len()..];
            if let Some((kind, other)) = rest.split_once(':') {
                if value.is_empty() {
                    live.links.remove(&(kind.to_string(), other.to_string()));
                } else {
                    live.links.insert((kind.to_string(), other.to_string()));
                }
                let lines: Vec<String> = live
                    .links
                    .iter()
                    .map(|(kind, other)| format!("{kind} {other}"))
                    .collect();
                replace_drawer(&mut live.heading, "LINKS", &lines);
            }
        }
        _ => {}
    }
}

fn set_property(live: &mut Live, key: &str, value: &str) {
    if value.is_empty() {
        live.heading.properties.remove(key);
        live.heading.property_order.retain(|name| name != key);
        return;
    }
    if !live.heading.property_order.iter().any(|name| name == key) {
        live.heading.property_order.push(key.to_string());
    }
    live.heading
        .properties
        .insert(key.to_string(), value.to_string());
}

fn rewrite_ballots(live: &mut Live) {
    let mut lines: Vec<String> = live.ballots.values().cloned().collect();
    lines.sort();
    replace_drawer(&mut live.heading, "VOTES", &lines);
}

fn rewrite_revisions(live: &mut Live, rev: &str, summary: &str) {
    let mut lines = drawer_lines(&live.heading, "REVISIONS");
    lines.retain(|line| !line.starts_with(&format!("{rev} ")));
    lines.push(format!("{rev} {summary}"));
    lines.sort();
    replace_drawer(&mut live.heading, "REVISIONS", &lines);
}

fn rewrite_targets(live: &mut Live) {
    let mut lines = Vec::new();
    for ((target, key), value) in &live.targets {
        lines.push(format!("{target} {key} {value}"));
    }
    if let Some(target) = &live.retarget {
        lines.push(format!("retarget {target}"));
    }
    lines.sort();
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
}

fn drawer_named(drawer: &str, name: &str) -> bool {
    drawer
        .lines()
        .next()
        .is_some_and(|line| line.trim().eq_ignore_ascii_case(&format!(":{name}:")))
}

fn append_log(mem: &mut Mem, op: &Op, entry: LogEntry) -> Result<()> {
    let live = mem.issues.get_mut(&op.issue).ok_or_else(|| missing(op))?;
    if !live.heading.logbook.iter().any(|have| have == &entry) {
        live.heading.logbook.push(entry);
    }
    live.heading.logbook.sort_by(log_order);
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
    write_union(mem, id, op, &format!("link:{kind}:{other}"), other)
}

fn one_char(value: &str) -> Result<char> {
    let mut chars = value.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Ok(ch),
        _ => Err(bad("priority is one character")),
    }
}

fn load_votes(heading: &IssueHeading) -> BTreeMap<String, String> {
    let mut votes = BTreeMap::new();
    for line in drawer_lines(heading, "VOTES") {
        let key = crate::ops::parse_ballot(&line)
            .map(|ballot| ballot_slot(&ballot.agent, ""))
            .unwrap_or_else(|| format!("line:{line}"));
        votes.insert(key, line);
    }
    votes
}

fn ballot_slot(voter: &str, rev: &str) -> String {
    if rev.is_empty() {
        voter.to_string()
    } else {
        format!("{voter}:{rev}")
    }
}

fn require_derived(project: &str, short_id: &str, key: IssueKey) -> Result<()> {
    if key != IssueKey::derive(project, short_id) {
        return Err(bad("an issue key is not derived from its project and id"));
    }
    Ok(())
}

fn check_op(op: &Op) -> Result<()> {
    match &op.kind {
        OpKind::Create { short_id, .. } => require_derived(&op.project, short_id, op.issue),
        OpKind::GenesisImport { heading, .. } => {
            require_derived(&op.project, &heading.id, op.issue)
        }
        OpKind::SetField { name, .. } | OpKind::Resolve { field: name, .. } => {
            canonical_field(name)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

impl Live {
    fn from_op(op: &Op, short_id: &str, heading: IssueHeading, id: OpId) -> Self {
        let ballots = load_votes(&heading);
        Self {
            project: op.project.clone(),
            short_id: short_id.to_string(),
            heading,
            links: BTreeSet::new(),
            ballots,
            targets: BTreeMap::new(),
            retarget: None,
            born_lamport: op.lamport,
            born_id: id,
        }
    }
}

fn tokens(value: Option<&str>) -> BTreeSet<String> {
    value
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_string)
        .collect()
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
    let seg = Seg { words: &words };
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

struct Seg<'a> {
    words: &'a [u64],
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
                let mut buf = Vec::with_capacity(nwords * 8);
                for word in &self.words[at..at + nwords] {
                    buf.extend_from_slice(&word.to_le_bytes());
                }
                buf.truncate(nbytes);
                Ok(Ptr::Bytes(buf))
            }
            6 => {
                self.need(at, count as usize)?;
                Ok(Ptr::Words {
                    at,
                    count: count as usize,
                })
            }
            7 => {
                self.need(at, 1 + count as usize)?;
                Ok(Ptr::Composite {
                    at,
                    words: count as usize,
                })
            }
            _ => Err(bad("unsupported list element in an op")),
        }
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
        if count * (data + ptrs) != words {
            return Err(bad("a struct list does not match its tag"));
        }
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            out.push(St {
                at: at + 1 + index * (data + ptrs),
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
            targets: BTreeMap::new(),
            retarget: None,
            born_lamport: lamport,
            born_id: OpId([0; 32]),
        }
    }

    #[test]
    fn two_derived_keys_that_share_a_short_id_keep_both_headings() {
        let alpha = IssueKey::derive("proj", "alpha");
        let beta = IssueKey::derive("proj", "beta");
        let mut mem = Mem::default();
        mem.issues.insert(beta, live("Beta", 2));
        mem.issues.insert(alpha, live("Alpha", 1));
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
