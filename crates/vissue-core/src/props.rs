//! Property names as Org and ELPA already spell them.
//!
//! vissue does not mint a parallel namespace. It writes the key the other
//! tool reads, and it reads the aliases a hand-edited tracker already has.
//! [`canonicalize`] is what a rewrite and `normalize` run.

use std::collections::BTreeMap;

use crate::org::{
    is_edna_blocker, is_org_tag_char, settle_heading_classifiers as settle_tags, split_id_list,
};

/// Org-id. Shared with Emacs.
pub const ID: &str = "ID";
/// Capture / org-expiry stamp. Shared meaning, shared name.
pub const CREATED: &str = "CREATED";
/// Org effort estimate. `org-effort-property` defaults to this spelling.
pub const EFFORT: &str = "Effort";
/// Per-heading agenda category. File-level `#+CATEGORY:` is the default.
pub const CATEGORY: &str = "CATEGORY";
/// org-id export alias.
pub const CUSTOM_ID: &str = "CUSTOM_ID";

/// GNU ELPA org-edna condition. Also org-depend in org-contrib.
pub const EDNA_BLOCKER: &str = "BLOCKER";
/// GNU ELPA org-edna / org-gtd action.
pub const EDNA_TRIGGER: &str = "TRIGGER";

/// Parent in the vissue tree.
pub const PARENT: &str = "PARENT";
/// Partial-order blockers. Read-compatible with org-edna `:BLOCKER:`.
pub const BLOCKED_BY: &str = "BLOCKED_BY";
/// Tracker type (`bug`, `feature`, …). Also a heading tag when legal.
pub const TYPE: &str = "TYPE";
/// Tags Org's character class rejects. Not `TAGS`: Org reserves that name.
pub const TAGS: &str = "VISSUE_TAGS";
/// Identity holding a STARTED issue.
pub const CLAIMED_BY: &str = "CLAIMED_BY";
/// When that claim was taken.
pub const CLAIMED_AT: &str = "CLAIMED_AT";
/// Paths this issue touches.
pub const FILES: &str = "FILES";
/// How to know the issue is done.
pub const VERIFY: &str = "VERIFY";
/// Deed accessions this issue's work produced, as deedar mints them.
pub const DEEDS: &str = "DEEDS";
/// Origin issue for a bounce or discovery.
pub const DISCOVERED_FROM: &str = "DISCOVERED_FROM";
/// Successor issue for a bounce.
pub const PIVOTED_TO: &str = "PIVOTED_TO";
/// The terminal that lost a sibling race.
pub const SIBLING_TERMINAL: &str = "SIBLING_TERMINAL";

/// Drawer keys vissue writes, in house order after `ID`.
pub const CANONICAL_ORDER: &[&str] = &[
    CREATED,
    TYPE,
    PARENT,
    BLOCKED_BY,
    TAGS,
    CLAIMED_BY,
    CLAIMED_AT,
    FILES,
    VERIFY,
    DEEDS,
    DISCOVERED_FROM,
    PIVOTED_TO,
    SIBLING_TERMINAL,
    EFFORT,
];

/// `(legacy, canonical)` pairs a rewrite folds.
pub const ALIASES: &[(&str, &str)] = &[
    ("BLOCKEDBY", BLOCKED_BY),
    ("TAGS", TAGS),
    ("DISCOVERED", DISCOVERED_FROM),
    ("SUPERSEDED_BY", PIVOTED_TO),
    ("EFFORT", EFFORT),
];

/// Read `canonical`, then any legacy alias.
pub fn get<'a>(properties: &'a BTreeMap<String, String>, canonical: &str) -> Option<&'a str> {
    if let Some(value) = properties.get(canonical)
        && !value.trim().is_empty()
    {
        return Some(value.as_str());
    }
    for (alias, dest) in ALIASES {
        if *dest == canonical
            && let Some(value) = properties.get(*alias)
            && !value.trim().is_empty()
        {
            return Some(value.as_str());
        }
    }
    None
}

/// Write `canonical` and drop every alias of that key.
pub fn insert(properties: &mut BTreeMap<String, String>, canonical: &str, value: String) {
    for (alias, dest) in ALIASES {
        if *dest == canonical {
            properties.remove(*alias);
        }
    }
    properties.insert(canonical.to_string(), value);
}

/// Remove `canonical` and every alias of it.
pub fn remove(properties: &mut BTreeMap<String, String>, canonical: &str) {
    properties.remove(canonical);
    for (alias, dest) in ALIASES {
        if *dest == canonical {
            properties.remove(*alias);
        }
    }
}

/// Fold aliases and merge a bare `:BLOCKER:` id list into `:BLOCKED_BY:`.
///
/// A real org-edna condition (`prev-sibling`, `ids(...)`, `headings`,
/// ...) stays on `:BLOCKER:`. vissue never writes that key.
///
/// Returns how many keys moved or merged.
pub fn canonicalize(properties: &mut BTreeMap<String, String>) -> usize {
    let mut moved = 0usize;
    for (alias, dest) in ALIASES {
        let Some(value) = properties.remove(*alias) else {
            continue;
        };
        moved += 1;
        if *dest == BLOCKED_BY || *dest == DISCOVERED_FROM || *dest == PIVOTED_TO {
            merge_id_valued(properties, dest, &value);
        } else if *dest == TAGS {
            let existing = properties.remove(TAGS).unwrap_or_default();
            let mut parts: Vec<String> = Vec::new();
            for item in existing.split([',', ':']).chain(value.split([',', ':'])) {
                let item = item.trim();
                if !item.is_empty() && !parts.iter().any(|seen| seen == item) {
                    parts.push(item.to_string());
                }
            }
            if !parts.is_empty() {
                properties.insert(TAGS.to_string(), parts.join(","));
            }
        } else {
            properties.entry((*dest).to_string()).or_insert(value);
        }
    }
    if let Some(raw) = properties.get(EDNA_BLOCKER).cloned()
        && !is_edna_blocker(&raw)
    {
        merge_id_valued(properties, BLOCKED_BY, &raw);
        properties.remove(EDNA_BLOCKER);
        moved += 1;
    }
    moved
}

fn merge_id_valued(properties: &mut BTreeMap<String, String>, dest: &str, extra: &str) {
    let mut ids = properties
        .get(dest)
        .map(|s| split_id_list(s))
        .unwrap_or_default();
    for id in split_id_list(extra) {
        if !ids.iter().any(|seen| seen == &id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        properties.remove(dest);
    } else {
        properties.insert(dest.to_string(), ids.join(" "));
    }
}

/// Canonicalize keys, then move legal type/tags onto the heading.
pub fn settle(org_tags: &mut Vec<String>, properties: &mut BTreeMap<String, String>) -> usize {
    let moved = canonicalize(properties);
    if let Some(kind) = get(properties, TYPE).map(str::to_string)
        && kind.chars().all(is_org_tag_char)
        && !kind.is_empty()
        && !org_tags.iter().any(|seen| seen == &kind)
    {
        org_tags.push(kind);
    }
    settle_tags(org_tags, properties);
    moved
}

/// Blocker ids from `:BLOCKED_BY:`, aliases, and org-edna `ids(...)`.
pub fn blocker_ids(properties: &BTreeMap<String, String>) -> Vec<String> {
    crate::org::blocker_ids_from_properties(properties)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_folds_typos_and_leaves_edna() {
        let mut props = BTreeMap::new();
        props.insert("BLOCKEDBY".into(), "a-1".into());
        props.insert(EDNA_BLOCKER.into(), "a-2".into());
        props.insert("TYPE".into(), "bug".into());
        let moved = canonicalize(&mut props);
        assert!(moved >= 1, "{props:?}");
        assert_eq!(get(&props, BLOCKED_BY), Some("a-1 a-2"));
        assert!(!props.contains_key(EDNA_BLOCKER), "{props:?}");
        assert_eq!(get(&props, TYPE), Some("bug"));
        assert!(props.contains_key("TYPE"));

        let mut edna = BTreeMap::new();
        edna.insert(BLOCKED_BY.into(), "a-1".into());
        edna.insert(EDNA_BLOCKER.into(), "prev-sibling".into());
        canonicalize(&mut edna);
        assert_eq!(
            edna.get(EDNA_BLOCKER).map(String::as_str),
            Some("prev-sibling")
        );
        assert_eq!(get(&edna, BLOCKED_BY), Some("a-1"));

        let mut minted = BTreeMap::new();
        minted.insert(BLOCKED_BY.into(), "a-1".into());
        minted.insert(EDNA_BLOCKER.into(), "ids(a-1)".into());
        canonicalize(&mut minted);
        assert_eq!(
            minted.get(EDNA_BLOCKER).map(String::as_str),
            Some("ids(a-1)")
        );
        assert_eq!(get(&minted, BLOCKED_BY), Some("a-1"));
    }

    #[test]
    fn get_reads_legacy_then_canonical() {
        let mut props = BTreeMap::new();
        props.insert("BLOCKEDBY".into(), "old".into());
        assert_eq!(get(&props, BLOCKED_BY), Some("old"));
        insert(&mut props, BLOCKED_BY, "new".into());
        assert_eq!(get(&props, BLOCKED_BY), Some("new"));
        assert!(!props.contains_key("BLOCKEDBY"));
        assert!(
            !props.contains_key(EDNA_BLOCKER),
            "writing BLOCKED_BY must not mint BLOCKER: {props:?}"
        );
    }
}
