---
name: hud-visual-walkthrough
description: >
  Launch vissue-hud, open the fixture vault, walk home / atlas / tree /
  list / recall / note, capture timed screenshots, then visually review
  each grab for polish, layout, and broken state using multimodal image
  inspection. Use when the user runs /hud-visual-walkthrough, asks for
  HUD screenshot review, pixel perfection, visual QA, or "is the HUD
  ugly / broken".
metadata:
  short-description: "vissue-hud timed walkthrough + visual QA"
---

# HUD visual walkthrough

End-to-end **visual** review of the icedtea vissue HUD. Product eyes on
real pixels — not a unit suite. There is no control-plane RPC sample on
this walk (`vissue-hud --offline`).

## Prerequisites

- Host X11 `DISPLAY` (Xephyr nests on it).
- **`Xephyr`**, **`metacity`**, **`wmctrl`**, **`import`** (ImageMagick).
- In-tree `vissue-hud` binary: prefers **`target/release/vissue-hud`**,
  then debug, then `vissue-hud` on `PATH` (document release for snappier
  walks). Do not cargo-build on the laptop; build on the builder.
- Auto keys: **`xdotool`** (or `--manual-keys`).
- Fixture: `tests/fixture_vault` (atlas + beacon).

## Isolation (default)

Matches icedtea `gallery-gif.sh` ideas:

1. **Xephyr** nested display
2. **metacity** WM inside it
3. **`VISSUE_HUD_WINDOW=1`** — decorated pop-out Boot
   (`me.rgoswami.vissue-hud.window`), not the overlay
4. **wmctrl place** + **`import -window root -crop`** of the client

Does **not** `--restart` a host HUD. Overlay mode stays off unless
`--overlay`. Tray and desktop notices are off (`VISSUE_HUD_TRAY=0`,
`VISSUE_HUD_NOTIFY=0`). The summon socket is unique to the walk.

| Backend | Flag | Notes |
|---------|------|-------|
| Xephyr | `--backend xephyr` (default) | Isolated; window mode paints |
| Host | `--backend host` | Interferes; still works |
| Xvfb | `--backend xvfb` | Often black |

`--dry-run` writes the step map and `timings.json` without PNGs, Xephyr,
or a HUD process. It does **not** satisfy verify.

## Inputs

| Arg / env | Meaning |
|-----------|---------|
| `--root` | Vault root (default `<repo>/tests/fixture_vault`) |
| Out dir | Default `tmp/hud-walk/<timestamp>/` under the repo |
| `--manual-keys` | Do not inject keys; wait for Enter between steps |
| `--backend` | `xephyr` (default), `host`, or `xvfb` |
| `--overlay` | Force overlay mode (skip `VISSUE_HUD_WINDOW`) |
| `--dry-run` | Step map + `timings.json` only; no shots |
| `--display-num N` | Nested display number |
| `--settle-ms N` | Sleep after action before settled screenshot (default 450) |
| `VISSUE_HUD_SUMMON_SOCKET` | Unused; the harness sets a unique walk socket |

## Agent procedure

### 1. Run the harness

From the **vissue** repo root:

```bash
# Prefer a release HUD built on the builder, not this laptop:
#   cargo build --release -p vissue-hud   # on rg.terra, never locally

python3 .grok/skills/hud-visual-walkthrough/scripts/hud_walkthrough.py \
  --out tmp/hud-walk/latest
```

Read stdout. Note `out_dir`, `timings.json`, `display=`, `backend=`,
`window_mode=true`, key injection live vs manual, step errors.

If the script fails before any screenshots, fix environment (binary, X,
Xephyr) and re-run. Do not invent screenshots.

`--dry-run` is a harness check only. Verify needs real PNGs from Xephyr
(or Terra) against `vissue-hud --offline` on the fixture.

### 2. Timings

`timings.json` is wall-clock and optional first-pixel `response_ms`.
There is no session RPC block. Report as printed — do not invent.

Optional `response_ms` on a step is **first pixel delta** from action
delivery (external observation). It is **not** product instrumentation.
Settled `ms` includes settle sleep and is wall-clock only.

### 2b. Interactive latency bar (HUD product)

**Every operator action on the hot path must feel under 100ms** when
measured externally (first meaningful pixel change after key/click
delivery — `response_ms` when the harness reports it, not settled `ms`
which includes settle sleep).

| In bar (must stay snappy) | Out of bar |
|---------------------------|------------|
| Key nav (j/k Enter Esc pane digits) | Cold first paint / font load |
| Home list step / typeahead | Disk catalog on a huge vault |
| Tab switch chrome (tree / recall / notes) | |
| Open project (Enter on a card) | |

**QA rules**

1. Flag any step whose **`response_ms` ≥ 100** for in-bar actions as
   **broken** (or **ugly** if only borderline and rare). Cite the step
   name and ms in the report.
2. Prefer release binary for timing walks; note debug builds as
   non-binding for the 100ms bar.
3. When fixing product code after a failed bar, prefer moving work off
   the keystroke — do not raise the bar.

### 3. Visual review (mandatory — every shot)

For **each** `*.png` under `out_dir/shots/` in step order:

1. Open with the **read_file** tool (image path). You **must** inspect
   pixels with multimodal vision — **filename-only scoring fails**.
2. Score against `references/rubric.md` using **categories A–G**, not a
   single remembered regression:
   - **A Geometry** — clip, overlap, occlusion; card/list gutters
   - **B Control honesty** — labels match load/empty behavior
   - **C Gate consistency** — chips/tabs vs body (home vs inside a project)
   - **D Identity** — project/issue labels, TODO color, no a11y-id as caption
   - **E Density** — empty shells, spacing, contrast
   - **F Pane validity** — home / atlas / tree / list / recall / note
   - **G Transitions** — boot → home → atlas → tree → list → recall → note
3. Human-usefulness bar: correct pane; primary controls usable without
   guesswork; no empty wrong pane when data should show.
4. Write one short note per shot: **ok / ugly / broken** + **category
   letter(s)** + one sentence why.

Do **not** use `image_gen` to “fix” the UI. Prefer plain description + path.

Do **not** stop at one remembered bug if Category B/C still fail.

### 4. Report

Write `out_dir/VISUAL_REPORT.md` (or `REPORT.md`) with:

1. **Environment** — branch, walk `DISPLAY`, backend, release vs debug.
2. **Vault** — `--root` (fixture by default).
3. **Timings** — from `timings.json`; **interactive `response_ms`** vs
   the **100ms** bar (§2b) for in-bar steps.
4. **Shot review** — ordered table: step · file · verdict · categories · notes.
5. **Category rollup** — which of A–G failed (with one example shot each).
6. **Latency rollup** — any in-bar step with `response_ms` ≥ 100 (step + ms).
7. **Verdict** — `SHIPPABLE` / `POLISH` / `BROKEN` (latency bar failures
   count as **BROKEN** when in-bar).
8. **Top fixes** — concrete product/UI asks (grouped by category when useful).

Paste a short summary into chat. Keep the full report on disk.

The visual workflow `vissue-hud-visual` fans four reviewers over the
shots; default rubric is this skill's `references/rubric.md`.

### 5. Do not

- Commit screenshots, casts, or `tmp/hud-walk/**` unless the user asks.
- Push a demo binary or force-push.
- Invent PNGs, use `image_gen`, or score from filenames only.
- Claim pixel perfection without reading every shot.
- Treat `--dry-run` as verify.
- Run overlay-only or Xvfb-default as the happy path (black crops).

## Step map

| Step | Action | Shot name | Category stress |
|------|--------|-----------|-----------------|
| 00 | Start HUD (window mode) | `00-boot` | — |
| 01 | Home project list | `01-home` | **C** cold: chips vs project cards |
| 02 | Enter opens atlas | `02-open-atlas` | F project board, G enter |
| 03 | Tree tab (default after enter) | `03-tree` | **F** parent/child |
| 04 | `2` List pane | `04-list` | F full list including closed |
| 05 | Enter×3 cycles to recall | `05-recall` | F working set / deeds |
| 06 | `n` notes tab and composer | `06-note` | F logbook / note field |

Atlas is first on the fixture home list (more ready work than beacon).
Enter on the first card is open-atlas. After enter, the detail tab is
tree. `2` is the list pane. Enter cycles tree → related → notes →
recall. `n` jumps to notes and opens the composer.

## `timings.json`

```json
{
  "utc": "YYYYMMDDTHHMMSSZ",
  "out": "<abs out dir>",
  "root": "<vault root>",
  "display": ":N or dry-run",
  "backend": "xephyr | host | xvfb | dry-run",
  "window_mode": true,
  "auto_keys": true,
  "dry_run": false,
  "steps": [
    {
      "step": "00-boot",
      "action": "…",
      "ms": 0,
      "response_ms": null,
      "settle_ms": 450,
      "shot": "shots/00-boot.png",
      "shot_md5": null,
      "error": null
    }
  ],
  "branch": "<git branch>",
  "binary": "release | debug | path | missing",
  "identical_pane_frames": false
}
```

`--dry-run` sets `dry_run: true`, `backend: "dry-run"`, `shot: null`.
No `shots/` PNGs. That is not verify.

## Quality

- Script is plain Python 3.12+; keep **ruff** clean (`ruff check` + `ruff format`).
- Prefer release binary; document when debug is used.
- Measurement is external (pixels) — no product file I/O hooks.
- Rubric is category-driven; extend categories when a new *class* of bug
  appears, not only a one-line regression for the last incident.

## Related

- Rubric: `references/rubric.md`
- Harness: `scripts/hud_walkthrough.py`
- Product keys: HUD `?` help; catalog in `vissue-core` `keys.rs`
- Workflow: `~/.grok/workflows/vissue-hud-visual.rhai`
