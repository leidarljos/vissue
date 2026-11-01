# HUD visual rubric

Score each screenshot from **pixels**, not from memory of prior bugs.
Prefer evidence over taste essays. When a defect fits a **category**
below, name the category in the shot note (not only the surface symptom).

This rubric is for **vissue-hud** panes (home, atlas board, tree, list,
recall, notes). Do not score anqa session/timeline chrome against these
shots.

## Verdicts per shot

| Tag | Meaning |
|-----|---------|
| **ok** | Usable, hierarchy clear, no category failure |
| **ugly** | Usable but density/contrast/polish issues |
| **broken** | Wrong state, dishonest chrome, clipped/overlapped controls, unreadable, or dead-end UX |

Any **broken** in the walk → overall report **BROKEN** unless the step is an intentional error demo.

---

## Category A — Geometry (clip, overlap, occlusion)

Ask on **every** shot that has chrome or a filter:

1. **Clip** — is any label, chip, badge, pick list, button, or **search field** cut off by the window, parent, or sibling? A field reduced to a sliver or a single word remnant is broken.
2. **Overlap** — do two controls paint on top of each other (chip row over search, count meta over Ready, cards under footer)? Stacked full-width rows still need clear separation and no shared hit box.
3. **Occlusion** — is a primary control covered so it cannot be used without guessing?
4. **Edges** — content inset from card edge (~12–16px). No text flush to border.
5. **Alignment** — rail vs detail; filter baselines; multi-row filters still leave every control fully visible.
6. **Structured fields** — property / recall stacks must share **one label gutter**: every value column starts on the same vertical line. Short keys (`id`) and long keys (`claimed by`) must not shift values.

Do not mark “ok” because *most* of the filter row fits if **any** primary control is clipped or overlapped.

---

## Category B — Control honesty (label matches behavior)

Chrome text must not lie about what the product will do.

| Pattern | Broken when |
|---------|-------------|
| Chip / tab label | Says **Ready** (or **List** / **tree** / **recall**) but body is a different pane *and* no load is underway |
| Empty-state copy | Contradicts the controls (“no projects” while cards are painted, or “no logbook” while notes are listed) |
| Count / range meta | Shows a11y ids, placeholders, or junk (`count`, `label`, widget ids) instead of a real count, blank, or honest zero |
| Loading | Chrome implies data exists while body is a permanent empty with no load path |

**Honest empty** is allowed: clear copy, controls that match the empty reason, a path to get data.

**Dishonest empty** is broken: label promises a pane that never loads; search is advertised but inert; meta invents text from accessibility names when caption is empty.

---

## Category C — Gate consistency (enabled chrome vs required state)

Controls that require a project (or a selected issue) must match body state.

| State | Expect |
|-------|--------|
| Home / no project | Body: project cards. **List/Claims/Agenda chips** must not look fully live if they only show the same empty home — either they enter a project, or they stay clearly home-scoped. |
| Inside a project | Ready / List / Claims / Agenda switch the board; tree / related / notes / recall switch the detail tab. |
| Tab active | Filename/step and selected tab chrome match. |

Ask: *If I click this control, does the UI already claim I can, while the body says I cannot?*

---

## Category D — Identity and labels (scanability)

Project cards and issue rows must stay scannable like the TUI product language.

1. **State color** — TODO families keep brand roles (open / started / blocked / done / cancelled). Monochrome titles with no state cue are **ugly** at best.
2. **Human labels** — issue ids (`atlas-1a2b`) and titles appear as the operator reads them, not wire names or widget ids.
3. **Hierarchy** — title = who/what (project or issue), face = preview, open body = full content.
4. **No a11y leakage** — accessibility names (`count`, `Ready`, role strings) never paint as the only visible caption when data is empty.

---

## Category E — Density and empty shells

1. **Card height** — open/closed cards should not be large blank slabs. Sparse chrome events as full empty expanders = **ugly** or **broken** if unusable.
2. **Spacing rhythm** — ~8px; more space between sections than within a card.
3. **Contrast** — muted meta still readable; status color means something.

---

## Category F — Product-state validity (by pane)

Score the **vissue** panes the walk actually opens. Fixture is
`tests/fixture_vault` (atlas + beacon).

1. **Home** (`01-home`) — project cards named **atlas** and **beacon**; ready counts; no issue rows in the ready/list body while browsing. Empty “no projects” while cards exist is broken.
2. **Open atlas** (`02-open-atlas`) — atlas issue rows (Parse / Publish / Emit / Rename as the list pane allows); a selected issue; excerpt/preview. Still on the home card list is broken.
3. **Tree** (`03-tree`) — tree tab active. atlas-1a2b is parent of atlas-2c3d; a parent/child outline or an honest “no links” on a node with none. Raw panic / blank right pane is broken.
4. **List** (`04-list`) — List chip/pane, not only Ready. Closed work (DONE Rename, BLOCKED Publish) belongs here; a Ready-only body after `2` is broken.
5. **Recall** (`05-recall`) — recall tab. Working set / deeds when present (atlas-4g5h cites `deed-patch-config-key`); honest empty (“stands on nothing”) when none. Tree outline still showing as recall is broken.
6. **Notes** (`06-note`) — notes tab and/or note composer (`n`). Logbook lines when the issue has them; composer visible after `n`. Recall/tree chrome as the only body is broken.
7. **Hard broken** — control socket down (this walk is `--offline`; a live-socket error is still broken), panic, zero-size window, all-black frame.

---

## Category G — Walk-path transitions (cross-shot)

Compare adjacent steps, not only isolated frames:

1. **Cold boot / home** (`00-boot`, `01-home`) — decorated window, project list, not an empty overlay crop. Gate consistency (Category C) before Enter.
2. **Home → atlas** (`02-open-atlas`) — body fills with atlas issues; not stuck on the card list.
3. **Atlas → tree** (`03-tree`) — tree tab selected; outline or honest empty, not a bit-identical home frame.
4. **→ list** (`04-list`) — list pane; not bit-identical to Ready if closed work exists in the fixture.
5. **→ recall** (`05-recall`) — recall tab; list/tree chrome must not be the only change (tab + body).
6. **→ note** (`06-note`) — notes tab and/or composer; not a bit-identical recall frame.

If the harness skips a frame, still reason about those states from the nearest shots and note the gap.

---

## “Highlight polished” bar

Polished = **calm density**: clear who/what hierarchy, consistent chip/badge language, quiet filters/footer, no competing borders. Flag **ugly** if it feels like a debug dump or terminal chrome pasted into a card.

## Timing interpretation

### Interactive bar (in-bar UI — hard)

**First meaningful pixel after key/click (`response_ms`) must stay under
100ms** for operator hot-path actions: list nav, pane switch chrome,
open project, open/close local detail shell. See skill §2b.

| `response_ms` (in-bar) | Verdict |
|------------------------|---------|
| &lt; 100ms | ok for latency |
| ≥ 100ms | **broken** (cite step + ms); overall walk **BROKEN** if in-bar |

Settled step `ms` includes harness settle sleep — **do not** use it for the
100ms bar. Prefer release binary; note debug as non-binding.

### Offline walk (no control RPC)

This harness launches `vissue-hud --offline`. There is no session RPC
block in `timings.json`. Do not invent RPC rows. A frozen input loop or
blank freeze **is** a fail.

## Anti-patterns for the reviewing agent

- Scoring “ok” from tab label alone without reading filter + body.
- Treating “empty tree” as fine without checking whether the selected issue has children (atlas-1a2b does).
- Treating “empty list” as fine after `2` when DONE/BLOCKED rows exist.
- Only checking for last week’s bug without Category A full geometry pass.
- Filename-only review without multimodal inspection of the PNG.
- Scoring anqa Timeline / All turns language against these shots.
