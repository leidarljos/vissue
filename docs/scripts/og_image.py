#!/usr/bin/env python3
"""Paint a 1200x630 preview image from the docs mark.

Reads ``docs/source/_static/mark.svg`` and writes a PNG. The geometry is that
file, placed on a card the size link previews expect. Nothing is drawn by hand.
"""

from __future__ import annotations

import sys
import xml.etree.ElementTree as ET
from pathlib import Path

import cairosvg

WIDTH = 1200
HEIGHT = 630
MARK_PX = 460

_SVG = "{http://www.w3.org/2000/svg}"
_ROOT = Path(__file__).resolve().parents[2]
_DEFAULT_SVG = _ROOT / "docs" / "source" / "_static" / "mark.svg"


def _rgb(value: str) -> tuple[int, int, int]:
    text = value.strip()
    if not text.startswith("#") or len(text) not in (4, 7):
        raise ValueError(f"unsupported colour {value!r}")
    if len(text) == 4:
        text = "#" + "".join(ch * 2 for ch in text[1:])
    return tuple(int(text[i : i + 2], 16) for i in (1, 3, 5))  # type: ignore[return-value]


def _plate(root: ET.Element) -> str:
    for element in root.iter():
        if element.tag.removeprefix(_SVG) == "rect" and "fill" in element.attrib:
            red, green, blue = _rgb(element.attrib["fill"])
            return f"#{red:02x}{green:02x}{blue:02x}"
    return "#0b0b0a"


def card_svg(svg_path: Path) -> str:
    ET.register_namespace("", "http://www.w3.org/2000/svg")
    mark = ET.parse(svg_path).getroot()
    card = ET.Element(
        f"{_SVG}svg",
        {"width": str(WIDTH), "height": str(HEIGHT), "viewBox": f"0 0 {WIDTH} {HEIGHT}"},
    )
    ET.SubElement(
        card,
        f"{_SVG}rect",
        {"width": str(WIDTH), "height": str(HEIGHT), "fill": _plate(mark)},
    )
    left = (WIDTH - MARK_PX) / 2
    top = (HEIGHT - MARK_PX) / 2
    nested = ET.SubElement(
        card,
        f"{_SVG}svg",
        {
            "x": str(left),
            "y": str(top),
            "width": str(MARK_PX),
            "height": str(MARK_PX),
            "viewBox": mark.attrib.get("viewBox", "0 0 256 256"),
        },
    )
    for child in list(mark):
        nested.append(child)
    return ET.tostring(card, encoding="unicode")


def _png_size(path: Path) -> tuple[int, int]:
    data = path.read_bytes()
    if data[12:16] != b"IHDR":
        raise RuntimeError(f"{path} is not a PNG")
    return int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")


def render(svg_path: Path, out_path: Path) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    cairosvg.svg2png(
        bytestring=card_svg(svg_path).encode(),
        write_to=str(out_path),
        output_width=WIDTH,
        output_height=HEIGHT,
    )
    size = _png_size(out_path)
    if size != (WIDTH, HEIGHT):
        raise RuntimeError(f"{out_path} is {size}, expected {(WIDTH, HEIGHT)}")


def main(argv: list[str]) -> int:
    svg = Path(argv[1]) if len(argv) > 1 else _DEFAULT_SVG
    out = Path(argv[2]) if len(argv) > 2 else _ROOT / "docs" / "build" / "_static" / "og.png"
    if not svg.is_file():
        print(f"og_image: missing {svg}", file=sys.stderr)
        return 1
    render(svg, out)
    print(f"og_image: wrote {out} ({WIDTH}x{HEIGHT})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
