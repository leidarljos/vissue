#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# ///
"""Timed vissue-hud walkthrough (isolated when possible).

Default: **Xephyr + metacity + VISSUE_HUD_WINDOW=1** (decorated pop-out
Boot, icedtea gallery-gif style). Capture is root-crop of the managed
window — non-black on nested X. Does not drive the host overlay.

No session / control RPC: the HUD is ``--offline`` on the fixture vault.

``--backend host`` uses the interactive display (interferes).
``--backend xvfb`` is headless; often black without a managed window.
``--dry-run`` writes the step map and timings.json without PNGs, Xephyr,
or a HUD process. Dry-run does not satisfy verify.

Writes shots/, timings.json, steps.jsonl under --out. Does not commit.
"""

from __future__ import annotations

# Operator-facing CLI: stderr prints are intentional.
# ruff: noqa: T201
import argparse
import atexit
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from datetime import UTC, datetime
from pathlib import Path


STEPS: tuple[tuple[str, str], ...] = (
    ("00-boot", "HUD up on display (window mode)"),
    ("01-home", "project list (home)"),
    ("02-open-atlas", "Enter open atlas"),
    ("03-tree", "tree tab (default after enter)"),
    ("04-list", "2 List pane"),
    ("05-recall", "Enter cycle to recall"),
    ("06-note", "n notes tab and composer"),
)


def _repo_root() -> Path:
    # …/vissue/.grok/skills/hud-visual-walkthrough/scripts/this.py → parents[4]
    return Path(__file__).resolve().parents[4]


def _now_ms() -> int:
    return int(time.monotonic() * 1000)


def _utc_stamp() -> str:
    return datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")


def _which(*names: str) -> str | None:
    for n in names:
        p = shutil.which(n)
        if p:
            return p
    return None


def _run(
    cmd: list[str],
    *,
    check: bool = False,
    env: dict[str, str] | None = None,
    timeout: float | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        check=check,
        text=True,
        capture_output=True,
        env=env,
        timeout=timeout,
    )


def _kill_display_server(display: str) -> None:
    """Stop Xvfb/Xephyr bound to *display* without pkill -f self-match."""
    marker = f" {display} "
    for name in ("Xephyr", "Xvfb"):
        for pid_s in _run(["pgrep", "-x", name]).stdout.split():
            try:
                pid = int(pid_s)
                cmd = Path(f"/proc/{pid}/cmdline").read_bytes().replace(b"\0", b" ").decode()
            except (ValueError, OSError):
                continue
            if marker in f" {cmd} " or cmd.rstrip().endswith(display):
                try:
                    os.kill(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
    time.sleep(0.25)


class VirtualDisplay:
    """Nested/virtual X server for isolated walk keys and screenshots.

    *xephyr* (default): nested window on the host DISPLAY — real GL often works.
    *xvfb*: fully headless; iced/wgpu may paint black on some hosts.
    """

    def __init__(
        self,
        *,
        backend: str = "xephyr",
        width: int = 1280,
        height: int = 800,
        display_num: int = 99,
        host_display: str | None = None,
    ) -> None:
        self.backend = backend.casefold()
        self.width = width
        self.height = height
        self.display_num = display_num
        self.display = f":{display_num}"
        self.host_display = host_display or os.environ.get("DISPLAY") or ":0"
        self.proc: subprocess.Popen[bytes] | None = None
        self.wm_proc: subprocess.Popen[bytes] | None = None
        self._owned = False

    def start(self) -> str:
        if self.backend == "xephyr":
            binary = _which("Xephyr")
            if not binary:
                raise SystemExit("Xephyr not found (apt install xserver-xephyr)")
            if not self.host_display:
                raise SystemExit("Xephyr needs a host DISPLAY to nest on")
            _kill_display_server(self.display)
            cmd = [
                binary,
                self.display,
                "-screen",
                f"{self.width}x{self.height}",
                "-ac",
                "-nolisten",
                "tcp",
                "+extension",
                "RANDR",
                "+extension",
                "COMPOSITE",
                "+extension",
                "XFIXES",
                "-title",
                "vissue-hud-walk",
            ]
            env = os.environ.copy()
            env["DISPLAY"] = self.host_display
            print(
                f"starting Xephyr {self.display} ({self.width}x{self.height}) "
                f"on host {self.host_display}",
                file=sys.stderr,
            )
            self.proc = subprocess.Popen(
                cmd,
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
        elif self.backend == "xvfb":
            binary = _which("Xvfb")
            if not binary:
                raise SystemExit("Xvfb not found (apt install xvfb)")
            _kill_display_server(self.display)
            cmd = [
                binary,
                self.display,
                "-screen",
                "0",
                f"{self.width}x{self.height}x24",
                "-ac",
                "+extension",
                "GLX",
                "-nolisten",
                "tcp",
            ]
            print(f"starting Xvfb {self.display} ({self.width}x{self.height})", file=sys.stderr)
            self.proc = subprocess.Popen(
                cmd,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
        else:
            raise SystemExit(f"unknown display backend: {self.backend}")

        self._owned = True
        ready = False
        for _ in range(50):
            if self.proc.poll() is not None:
                raise SystemExit(f"{self.backend} exited early (code {self.proc.returncode})")
            r = _run(["xdpyinfo", "-display", self.display], timeout=2.0)
            if r.returncode == 0:
                ready = True
                break
            time.sleep(0.15)
        if not ready:
            raise SystemExit(f"{self.backend} {self.display} did not become ready")

        # icedtea gallery-gif: metacity so the client is a managed window.
        if self.backend in ("xephyr", "xvfb") and _which("metacity"):
            print(f"starting metacity on {self.display}", file=sys.stderr)
            env = os.environ.copy()
            env["DISPLAY"] = self.display
            env.pop("WAYLAND_DISPLAY", None)
            self.wm_proc = subprocess.Popen(
                ["metacity", "--sm-disable"],
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            time.sleep(0.4)
            _run(["xsetroot", "-solid", "#1a1a1a"], env=env, check=False)
        elif self.backend in ("xephyr", "xvfb"):
            print(
                "warning: metacity missing; nested capture may fail (apt install metacity)",
                file=sys.stderr,
            )
        return self.display

    def stop(self) -> None:
        if self.wm_proc is not None:
            try:
                os.killpg(self.wm_proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.wm_proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(self.wm_proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            self.wm_proc = None
        if not self._owned or self.proc is None:
            return
        try:
            os.killpg(self.proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(self.proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        self.proc = None
        self._owned = False


class WalkEnv:
    """Process environment bound to the walk DISPLAY (virtual or host)."""

    def __init__(
        self,
        display: str,
        *,
        window_mode: bool = True,
        vault_root: Path,
        summon_socket: Path,
    ) -> None:
        self.display = display
        self.window_mode = window_mode
        self.vault_root = vault_root
        self.env = os.environ.copy()
        self.env["DISPLAY"] = display
        # Prefer X11 for the nested HUD; avoid stealing the host Wayland session.
        self.env.pop("WAYLAND_DISPLAY", None)
        # Decorated pop-out Boot (not override-redirect overlay). Required for
        # Xephyr + metacity root-crop capture (same idea as icedtea gallery).
        if window_mode:
            self.env["VISSUE_HUD_WINDOW"] = "1"
        else:
            self.env.pop("VISSUE_HUD_WINDOW", None)
        self.env["VISSUE_HUD_TRAY"] = "0"
        self.env["VISSUE_HUD_NOTIFY"] = "0"
        self.env["VISSUE_HUD_SUMMON_SOCKET"] = str(summon_socket)
        summon_socket.parent.mkdir(parents=True, exist_ok=True)
        self.hud_proc: subprocess.Popen[bytes] | None = None
        self._cached_wid: str | None = None
        self._cached_geom: tuple[int, int, int, int] | None = None

    def run(
        self,
        cmd: list[str],
        *,
        timeout: float | None = 8.0,
    ) -> subprocess.CompletedProcess[str]:
        return _run(cmd, env=self.env, timeout=timeout)

    def place_managed_window(self, *, force: bool = False) -> str | None:
        """Place the HUD client with wmctrl (window mode) or xdotool.

        Caches the window id after first place so settle captures do not
        re-run ``wmctrl -a`` (focus thrash) on every screenshot.
        """
        if self.hud_proc is None:
            return None
        if self._cached_wid and not force:
            return self._cached_wid
        pid = self.hud_proc.pid
        # Wait for a managed window (window mode shows in wmctrl).
        if _which("wmctrl"):
            for _ in range(40):
                r = self.run(["wmctrl", "-lp"], timeout=3.0)
                if r.returncode == 0:
                    for line in r.stdout.splitlines():
                        parts = line.split(None, 4)
                        if len(parts) >= 3 and parts[2] == str(pid):
                            wid = parts[0]
                            self.run(
                                [
                                    "wmctrl",
                                    "-i",
                                    "-r",
                                    wid,
                                    "-e",
                                    "0,32,24,960,760",
                                ],
                                timeout=3.0,
                            )
                            self.run(["wmctrl", "-i", "-a", wid], timeout=3.0)
                            self._cached_wid = wid
                            return wid
                time.sleep(0.15)
        # Fallback xdotool place.
        self.place_hud_on_screen()
        ids = self._hud_window_ids()
        if ids:
            self._cached_wid = ids[0]
            return ids[0]
        return None

    def capture(self, path: Path) -> None:
        """Capture the HUD client (icedtea-style root crop preferred)."""
        path.parent.mkdir(parents=True, exist_ok=True)
        wid = self.place_managed_window()
        imp = _which("import")
        scrot = _which("scrot")

        # Nested window mode: root crop of absolute geometry (gallery-gif).
        if wid and imp and _which("xwininfo"):
            geom = self._cached_geom
            if geom is None:
                info = self.run(["xwininfo", "-id", wid], timeout=3.0)
                if info.returncode == 0:
                    x = y = w = h = 0
                    for line in info.stdout.splitlines():
                        if "Absolute upper-left X" in line:
                            x = int(line.split()[-1])
                        elif "Absolute upper-left Y" in line:
                            y = int(line.split()[-1])
                        elif line.strip().startswith("Width:"):
                            w = int(line.split()[-1])
                        elif line.strip().startswith("Height:"):
                            h = int(line.split()[-1])
                    if w > 100 and h > 100 and x >= 0 and y >= 0:
                        geom = (x, y, w, h)
                        self._cached_geom = geom
            if geom is not None:
                x, y, w, h = geom
                r = self.run(
                    [
                        "import",
                        "-window",
                        "root",
                        "-crop",
                        f"{w}x{h}+{x}+{y}",
                        "+repage",
                        str(path),
                    ],
                    timeout=15.0,
                )
                if r.returncode == 0 and path.is_file() and path.stat().st_size > 2000:
                    return

        # Host path: import the window id (works for GL on real DISPLAY).
        if wid and imp:
            r = self.run([imp, "-window", wid, str(path)], timeout=15.0)
            if r.returncode == 0 and path.is_file() and path.stat().st_size > 2000:
                return

        if wid and scrot:
            geom = self.run(
                ["xdotool", "getwindowgeometry", "--shell", wid],
                timeout=3.0,
            )
            if geom.returncode == 0:
                env_map: dict[str, str] = {}
                for line in geom.stdout.splitlines():
                    if "=" in line:
                        k, v = line.split("=", 1)
                        env_map[k.strip()] = v.strip()
                try:
                    x = int(env_map["X"])
                    y = int(env_map["Y"])
                    w = int(env_map["WIDTH"])
                    h = int(env_map["HEIGHT"])
                except (KeyError, ValueError):
                    x = y = w = h = 0
                if w > 0 and h > 0:
                    r = self.run(
                        [scrot, "-z", "-a", f"{x},{y},{w},{h}", str(path)],
                        timeout=15.0,
                    )
                    if r.returncode == 0 and path.is_file() and path.stat().st_size > 2000:
                        return

        if scrot:
            r = self.run([scrot, "-z", str(path)], timeout=15.0)
            if r.returncode == 0 and path.is_file() and path.stat().st_size > 0:
                return
        if imp:
            r = self.run([imp, "-window", "root", str(path)], timeout=15.0)
            if r.returncode == 0 and path.is_file() and path.stat().st_size > 0:
                return
        raise SystemExit("HUD window capture failed")

    def xdotool(self, *args: str, timeout: float = 3.0) -> bool:
        xd = _which("xdotool")
        if not xd:
            return False
        try:
            r = self.run([xd, *args], timeout=timeout)
            return r.returncode == 0
        except subprocess.TimeoutExpired:
            print(f"xdotool timeout: {' '.join(args)}", file=sys.stderr)
            return False

    def _hud_window_ids(self) -> list[str]:
        ids: list[str] = []
        if self.hud_proc is not None:
            r = self.run(
                ["xdotool", "search", "--pid", str(self.hud_proc.pid)],
                timeout=2.0,
            )
            if r.returncode == 0:
                ids.extend(r.stdout.split())
        for pattern in ("vissue-hud", "vissue_hud", "vissue"):
            for flag in ("--name", "--class"):
                r = self.run(
                    ["xdotool", "search", flag, pattern],
                    timeout=2.0,
                )
                if r.returncode == 0:
                    for wid in r.stdout.split():
                        if wid not in ids:
                            ids.append(wid)
        return ids

    def place_hud_on_screen(self, *, x: int = 32, y: int = 24) -> bool:
        """Move HUD windows into the nested screen.

        On Xephyr/Xvfb the palette often maps at a negative origin (host
        geometry vs nested size), so scrot of the root is pure black.
        """
        ok = False
        for wid in self._hud_window_ids():
            self.xdotool("windowmap", wid, timeout=1.0)
            if self.xdotool("windowmove", wid, str(x), str(y), timeout=1.0):
                ok = True
            self.xdotool("windowactivate", "--sync", wid, timeout=1.5)
            self.xdotool("windowraise", wid, timeout=1.0)
        return ok

    def focus_hud(self) -> bool:
        if not _which("xdotool"):
            return False
        self.place_hud_on_screen()
        for wid in self._hud_window_ids():
            if self.xdotool("windowactivate", "--sync", wid, timeout=2.0):
                return True
        return False

    def click_detail_pane(self) -> None:
        """Click the right-hand pane so search loses focus for key delivery."""
        wid = self.place_managed_window()
        if not wid:
            return
        self.xdotool(
            "mousemove",
            "--window",
            wid,
            "620",
            "380",
            timeout=2.0,
        )
        self.xdotool("click", "1", timeout=2.0)

    def shot_digest(self, path: Path) -> str | None:
        """Fast content fingerprint for non-identical pane checks."""
        if not path.is_file():
            return None
        import hashlib

        return hashlib.md5(path.read_bytes()).hexdigest()  # noqa: S324

    def first_visual_change_ms(
        self,
        t_action: float,
        baseline: Path | None,
        probe: Path,
        *,
        timeout_ms: int = 2000,
        poll_ms: int = 80,
    ) -> int | None:
        """Sparse root-crop until pixels differ from *baseline*; ms from *t_action*.

        External observation only — does not require product instrumentation.
        Sparse polling avoids starving nested X (dense import thrash is invalid).
        """
        base = self.shot_digest(baseline) if baseline is not None else None
        if base is None:
            return None
        deadline = t_action + timeout_ms / 1000.0
        while time.perf_counter() < deadline:
            try:
                self.capture(probe)
            except SystemExit:
                time.sleep(poll_ms / 1000.0)
                continue
            dig = self.shot_digest(probe)
            if dig is not None and dig != base:
                return int((time.perf_counter() - t_action) * 1000)
            time.sleep(poll_ms / 1000.0)
        return None

    def key(self, *keys: str) -> None:
        if not self.xdotool("key", "--clearmodifiers", *keys, timeout=3.0):
            raise RuntimeError("xdotool key failed")

    def type_text(self, text: str) -> None:
        if not self.xdotool("type", "--clearmodifiers", "--delay", "12", text, timeout=15.0):
            raise RuntimeError("xdotool type failed")

    def resolve_hud_bin(self, repo: Path) -> tuple[list[str], str]:
        """Prefer in-tree release, then debug, then PATH. Never cargo-build."""
        release = repo / "target" / "release" / "vissue-hud"
        debug = repo / "target" / "debug" / "vissue-hud"
        if release.is_file():
            return [str(release)], "release"
        if debug.is_file():
            return [str(debug)], "debug"
        found = _which("vissue-hud")
        if found:
            return [found], "path"
        return [], "missing"

    def start_hud(self, repo: Path) -> str:
        """Always start a fresh HUD on this DISPLAY — never reuse the host HUD."""
        cmd, kind = self.resolve_hud_bin(repo)
        if not cmd:
            raise SystemExit(
                "vissue-hud binary not found (build on the builder; do not invent shots)"
            )
        cmd += [
            "--offline",
            "--foreground",
            "--root",
            str(self.vault_root),
        ]
        mode = "window" if self.window_mode else "overlay"
        print(
            f"launching on {self.display} ({mode}): {' '.join(cmd)}",
            file=sys.stderr,
        )
        self.hud_proc = subprocess.Popen(
            cmd,
            env=self.env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        for _ in range(60):
            if self.hud_proc.poll() is not None:
                raise SystemExit(f"vissue-hud exited early (code {self.hud_proc.returncode})")
            time.sleep(0.25)
            if self.hud_proc.poll() is None and _ > 4:
                return kind
        if self.hud_proc.poll() is not None:
            raise SystemExit("vissue-hud did not stay up")
        return kind

    def stop_hud(self) -> None:
        if self.hud_proc is None:
            return
        try:
            os.killpg(self.hud_proc.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            self.hud_proc.wait(timeout=4)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(self.hud_proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        self.hud_proc = None


def free_display_num(start: int = 90) -> int:
    for n in range(start, start + 40):
        lock = Path(f"/tmp/.X{n}-lock")
        if not lock.exists():
            return n
    return start


def _write_report(
    out: Path,
    *,
    display: str,
    backend: str,
    window_mode: bool,
    auto_keys: bool,
    dry_run: bool,
    root: Path,
    steps: list[dict[str, object]],
    binary: str,
    identical_pane_frames: bool,
) -> None:
    repo = _repo_root()
    report = {
        "utc": _utc_stamp(),
        "out": str(out),
        "root": str(root),
        "display": display,
        "backend": backend,
        "window_mode": window_mode,
        "auto_keys": auto_keys,
        "dry_run": dry_run,
        "steps": steps,
        "branch": _run(["git", "-C", str(repo), "branch", "--show-current"]).stdout.strip(),
        "binary": binary,
        "identical_pane_frames": identical_pane_frames,
    }
    (out / "timings.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    with (out / "steps.jsonl").open("w", encoding="utf-8") as fh:
        for s in steps:
            fh.write(json.dumps(s) + "\n")


def _dry_run(out: Path, vault: Path, settle_ms: int) -> int:
    out.mkdir(parents=True, exist_ok=True)
    steps: list[dict[str, object]] = []
    for name, action in STEPS:
        rec: dict[str, object] = {
            "step": name,
            "action": action,
            "ms": 0,
            "response_ms": None,
            "settle_ms": settle_ms,
            "shot": None,
            "shot_md5": None,
            "error": None,
        }
        steps.append(rec)
        print(json.dumps(rec), flush=True)
    _write_report(
        out,
        display="dry-run",
        backend="dry-run",
        window_mode=True,
        auto_keys=False,
        dry_run=True,
        root=vault,
        steps=steps,
        binary="missing",
        identical_pane_frames=False,
    )
    print(f"\nout_dir={out}", file=sys.stderr)
    print(f"timings={out / 'timings.json'}", file=sys.stderr)
    print("dry-run: step map only; no PNGs. Does not satisfy verify.", file=sys.stderr)
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--root",
        type=Path,
        default=None,
        help="vault root (default: <repo>/tests/fixture_vault)",
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=None,
        help="output directory (default: <repo>/tmp/hud-walk/<utc>)",
    )
    ap.add_argument(
        "--manual-keys",
        action="store_true",
        help="do not inject keys; wait for Enter between steps",
    )
    ap.add_argument(
        "--settle-ms",
        type=int,
        default=450,
        help="settle time after each UI action before screenshot (default 450)",
    )
    ap.add_argument(
        "--no-xvfb",
        action="store_true",
        help="use current DISPLAY (interferes; alias for --backend host)",
    )
    ap.add_argument(
        "--backend",
        choices=("xephyr", "host", "xvfb"),
        default="xephyr",
        help="xephyr+metacity+window mode (default); host interferes; xvfb often black",
    )
    ap.add_argument(
        "--overlay",
        action="store_true",
        help="use overlay mode instead of VISSUE_HUD_WINDOW=1",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="write step map and timings.json without PNGs, Xephyr, or HUD",
    )
    ap.add_argument(
        "--display-num",
        type=int,
        default=None,
        help="nested display number (default: first free from :90)",
    )
    ap.add_argument("--width", type=int, default=1280)
    ap.add_argument("--height", type=int, default=800)
    args = ap.parse_args()

    root = _repo_root()
    vault = (args.root or (root / "tests" / "fixture_vault")).expanduser().resolve()
    out = args.out or (root / "tmp" / "hud-walk" / _utc_stamp())
    out = out.expanduser().resolve()

    if args.dry_run:
        return _dry_run(out, vault, args.settle_ms)

    if not vault.is_dir():
        print(f"error: vault root not a directory: {vault}", file=sys.stderr)
        return 2

    backend = "host" if args.no_xvfb else args.backend
    nested: VirtualDisplay | None = None
    if backend == "host":
        display = os.environ.get("DISPLAY") or ""
        if not display:
            print("error: host backend requires DISPLAY", file=sys.stderr)
            return 2
        print(f"WARNING: using host display {display} (will interfere)", file=sys.stderr)
    else:
        host = os.environ.get("DISPLAY") or ""
        if backend == "xephyr":
            if not host:
                print("error: Xephyr needs a host DISPLAY to nest on", file=sys.stderr)
                return 2
            if not _which("Xephyr"):
                print("error: Xephyr not found; not inventing PNGs", file=sys.stderr)
                return 2
        num = args.display_num if args.display_num is not None else free_display_num(90)
        nested = VirtualDisplay(
            backend=backend,
            width=args.width,
            height=args.height,
            display_num=num,
            host_display=host or None,
        )
        display = nested.start()

    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    summon = runtime / "vissue" / f"hud-walk-{os.getpid()}.sock"
    walk = WalkEnv(
        display,
        window_mode=not args.overlay,
        vault_root=vault,
        summon_socket=summon,
    )
    atexit.register(walk.stop_hud)
    if nested is not None:
        atexit.register(nested.stop)

    binary_kind = walk.start_hud(root)
    time.sleep(1.2)
    if walk.window_mode:
        for _ in range(20):
            if walk.place_managed_window():
                break
            time.sleep(0.2)

    has_xdo = _which("xdotool") is not None
    auto = has_xdo and not args.manual_keys
    if not auto:
        print(
            "key injection: MANUAL" + ("" if args.manual_keys else " (install xdotool for auto)"),
            file=sys.stderr,
        )
    else:
        print(f"key injection: xdotool on {display}", file=sys.stderr)

    shots = out / "shots"
    shots.mkdir(parents=True, exist_ok=True)
    steps: list[dict[str, object]] = []
    settle = max(0, args.settle_ms) / 1000.0
    prev_shot: Path | None = None

    def step(
        name: str,
        action: str,
        fn: object,
        *,
        measure_visual: bool = False,
    ) -> None:
        """Run *fn*, optional first-pixel ``response_ms``, then settle + shot."""
        nonlocal prev_shot
        shot = shots / f"{name}.png"
        probe = shots / f".probe-{name}.png"
        err: str | None = None
        response_ms: int | None = None
        wall0 = _now_ms()
        try:
            if auto:
                base_path = (
                    prev_shot
                    if measure_visual and prev_shot is not None and prev_shot.is_file()
                    else None
                )
                t_action = time.perf_counter()
                assert callable(fn)
                fn()
                if measure_visual and base_path is not None:
                    response_ms = walk.first_visual_change_ms(
                        t_action, base_path, probe, timeout_ms=1500, poll_ms=100
                    )
            else:
                print(f"\n>>> STEP {name}: {action}", file=sys.stderr)
                print(f"    (walk DISPLAY={display}) Enter when done…", file=sys.stderr)
                try:
                    input()
                except EOFError:
                    time.sleep(2.0)
            time.sleep(settle)
            try:
                walk.capture(shot)
            except SystemExit as cap_exc:
                err = f"capture: {cap_exc}"
            if auto:
                time.sleep(0.12)
        except Exception as exc:
            err = str(exc)
            try:
                walk.capture(shot)
            except Exception:
                pass
        wall = _now_ms() - wall0
        rec: dict[str, object] = {
            "step": name,
            "action": action,
            "ms": wall,
            "response_ms": response_ms,
            "settle_ms": args.settle_ms,
            "shot": str(shot.relative_to(out)) if shot.is_file() else None,
            "shot_md5": walk.shot_digest(shot) if shot.is_file() else None,
            "error": err,
        }
        steps.append(rec)
        if shot.is_file():
            prev_shot = shot
        print(json.dumps(rec), flush=True)
        if err is not None and err.startswith("capture:"):
            raise SystemExit(err)

    def home() -> None:
        if walk.window_mode:
            walk.place_managed_window()
            walk.focus_hud()
            return
        walk.focus_hud()

    def open_atlas() -> None:
        walk.focus_hud()
        walk.key("Return")

    def list_pane() -> None:
        walk.key("2")

    def recall_tab() -> None:
        # Tree → Related → Notes → Recall.
        walk.key("Return")
        walk.key("Return")
        walk.key("Return")

    def note_tab() -> None:
        walk.key("n")

    step("00-boot", STEPS[0][1], home)
    step("01-home", STEPS[1][1], home)
    step("02-open-atlas", STEPS[2][1], open_atlas, measure_visual=True)
    step("03-tree", STEPS[3][1], lambda: None)
    step("04-list", STEPS[4][1], list_pane, measure_visual=True)
    step("05-recall", STEPS[5][1], recall_tab, measure_visual=True)
    step("06-note", STEPS[6][1], note_tab, measure_visual=True)

    pane_steps = [
        s
        for s in steps
        if s.get("step") in {"03-tree", "04-list", "05-recall", "06-note"}
    ]
    md5s = [s.get("shot_md5") for s in pane_steps if s.get("shot_md5")]
    identical_panes = len(md5s) >= 2 and len(set(md5s)) == 1

    _write_report(
        out,
        display=display,
        backend=backend,
        window_mode=walk.window_mode,
        auto_keys=auto,
        dry_run=False,
        root=vault,
        steps=steps,
        binary=binary_kind,
        identical_pane_frames=identical_panes,
    )

    walk.stop_hud()
    if nested is not None:
        nested.stop()

    print(f"\nout_dir={out}", file=sys.stderr)
    print(f"timings={out / 'timings.json'}", file=sys.stderr)
    print(f"display={display} backend={backend}", file=sys.stderr)
    print(f"identical_pane_frames={identical_panes}", file=sys.stderr)
    print(
        "done — agent must read each shots/*.png (vision) and write VISUAL_REPORT.md",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
