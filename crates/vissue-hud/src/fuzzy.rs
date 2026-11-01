//! Rank ready and search rows for the palette filter.

use crate::palette::HudItem;

const SCORE_MATCH: i32 = 16;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_BOUNDARY_WHITE: i32 = 10;
const BONUS_CAMEL: i32 = 10;
const BONUS_CONSECUTIVE: i32 = 4;
const BONUS_FIRST_MULT: i32 = 2;
const PENALTY_GAP_START: i32 = -3;
const PENALTY_GAP_EXTEND: i32 = -1;

const BOOST_EXACT_ID: i32 = 100_000;
const BOOST_ID_PREFIX: i32 = 80_000;
const BOOST_ID_SUBSTR: i32 = 50_000;
const BOOST_TITLE_PREFIX: i32 = 40_000;
const BOOST_TITLE_SUBSTR: i32 = 20_000;
const BOOST_FUZZY: i32 = 1_000;

fn is_boundary(c: char) -> bool {
    matches!(
        c,
        '/' | '-' | '_' | '.' | ' ' | ',' | ';' | ':' | '\\' | '\t'
    )
}

fn char_bonus(prev: Option<char>, curr: char) -> i32 {
    let Some(p) = prev else {
        return BONUS_BOUNDARY_WHITE;
    };
    if is_boundary(p) {
        return if p == ' ' || p == '\t' {
            BONUS_BOUNDARY_WHITE
        } else {
            BONUS_BOUNDARY
        };
    }
    if p.is_lowercase() && curr.is_uppercase() {
        return BONUS_CAMEL;
    }
    0
}

/// fzf-style subsequence score. Zero means no match.
pub fn fzf_score(query: &str, candidate: &str) -> i32 {
    let ql: Vec<char> = query.to_lowercase().chars().collect();
    let cl: Vec<char> = candidate.to_lowercase().chars().collect();
    let orig: Vec<char> = candidate.chars().collect();
    let qchars: Vec<char> = query.chars().collect();
    if ql.is_empty() {
        return 0;
    }
    if ql.len() > cl.len() {
        return 0;
    }
    let mut positions = Vec::new();
    let mut j = 0;
    for (i, c) in cl.iter().enumerate() {
        if j < ql.len() && *c == ql[j] {
            positions.push(i);
            j += 1;
        }
    }
    if j < ql.len() {
        return 0;
    }
    let mut score = 0;
    let mut consecutive = 0;
    for (k, &pos) in positions.iter().enumerate() {
        let prev = if pos > 0 {
            orig.get(pos - 1).copied()
        } else {
            None
        };
        let curr = orig.get(pos).copied().unwrap_or(' ');
        let bonus = char_bonus(prev, curr);
        let mut char_score = SCORE_MATCH + bonus;
        if k == 0 && bonus > 0 {
            char_score += bonus * (BONUS_FIRST_MULT - 1);
        }
        if consecutive > 0 {
            char_score += BONUS_CONSECUTIVE;
        } else if k > 0 {
            let gap = pos - (positions[k - 1] + 1);
            if gap > 0 {
                char_score += PENALTY_GAP_START + PENALTY_GAP_EXTEND * (gap as i32 - 1);
            }
        }
        if qchars.get(k).copied() == orig.get(pos).copied() {
            char_score += 1;
        }
        score += char_score.max(0);
        if k > 0 && pos == positions[k - 1] + 1 {
            consecutive += 1;
        } else {
            consecutive = 0;
        }
    }
    score
}

/// Rank one catalog row. Exact id outranks a title substring of the same query.
pub fn score_item(query: &str, item: &HudItem) -> i32 {
    let q = query.trim();
    if q.is_empty() {
        return 0;
    }
    let ql = q.to_ascii_lowercase();
    let idl = item.id.to_ascii_lowercase();
    let tl = item.title.to_ascii_lowercase();
    let mut best = 0_i32;

    if idl == ql {
        best = best.max(BOOST_EXACT_ID + fzf_score(q, &item.id).max(SCORE_MATCH));
    } else if idl.starts_with(&ql) {
        best = best.max(BOOST_ID_PREFIX + fzf_score(q, &item.id).max(SCORE_MATCH));
    } else if idl.contains(&ql) {
        best = best.max(BOOST_ID_SUBSTR + fzf_score(q, &item.id).max(SCORE_MATCH));
    } else {
        let s = fzf_score(q, &item.id);
        if s > 0 {
            best = best.max(s + BOOST_ID_SUBSTR / 2);
        }
    }

    if tl.starts_with(&ql) {
        best = best.max(BOOST_TITLE_PREFIX + fzf_score(q, &item.title).max(SCORE_MATCH));
    } else if tl.contains(&ql) {
        best = best.max(BOOST_TITLE_SUBSTR + fzf_score(q, &item.title).max(SCORE_MATCH));
    } else {
        let s = fzf_score(q, &item.title);
        if s > 0 {
            best = best.max(s + BOOST_FUZZY);
        }
    }

    best
}

/// Indices into `items` ordered by [`score_item`] descending, then source order.
pub fn rank_indices(query: &str, items: &[HudItem]) -> Vec<usize> {
    let q = query.trim();
    if q.is_empty() {
        return (0..items.len()).collect();
    }
    let mut scored: Vec<(i32, usize)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let score = score_item(q, item);
        if score > 0 {
            scored.push((score, i));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::{HudItem, ItemSource};

    fn item(id: &str, title: &str) -> HudItem {
        HudItem {
            id: id.into(),
            title: title.into(),
            project: "atlas".into(),
            state: "TODO".into(),
            priority: "C".into(),
            source: ItemSource::Ready,
            claimed_by: None,
            due: None,
            blocked_by: Vec::new(),
            extra: String::new(),
            parent: None,
            depth: 0,
            group: None,
        }
    }

    #[test]
    fn exact_id_ranks_above_title_substring() {
        let by_id = item("atlas-1a2b", "Parse the header");
        let by_title = item("beacon-5j6k", "See atlas-1a2b in the notes");
        let id_s = score_item("atlas-1a2b", &by_id);
        let title_s = score_item("atlas-1a2b", &by_title);
        assert!(id_s > title_s, "id {id_s} vs title {title_s}");
        let rows = [by_title.clone(), by_id.clone()];
        let order = rank_indices("atlas-1a2b", &rows);
        assert_eq!(rows[order[0]].id, "atlas-1a2b");
    }

    #[test]
    fn empty_query_keeps_source_order() {
        let rows = [item("b", "B"), item("a", "A")];
        assert_eq!(rank_indices("", &rows), vec![0, 1]);
    }

    #[test]
    fn subsequence_still_matches() {
        assert!(fzf_score("hud", "vissue-hud palette") > 0);
        assert_eq!(fzf_score("zzz", "vissue"), 0);
    }
}
