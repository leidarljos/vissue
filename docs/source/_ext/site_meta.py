"""Page titles, preview tags, and source links for the generated site.

The rst under docs/source is produced in CI and is not committed. The file a
page is edited in is the org document it was exported from. Titles in the
HTML come from that document's #+title, not from whichever section happens
to be first.
"""

from __future__ import annotations

import re
import subprocess
from html import escape
from pathlib import Path

from sphinx.application import Sphinx
from sphinx.util import logging
from sphinx.writers.html5 import HTML5Translator

logger = logging.getLogger(__name__)

_SOURCE = Path(__file__).resolve().parents[1]
_ROOT = _SOURCE.parents[1]
_ORG = _ROOT / "docs" / "orgmode"
_OG_SCRIPT = _ROOT / ".github" / "render-og-png.sh"

_TITLE = re.compile(r"^#\+title:\s*(.*?)\s*$", re.IGNORECASE)
_DESCRIPTION = re.compile(r"^#\+description:\s*(.*?)\s*$", re.IGNORECASE)

# Pages Sphinx writes that have no org source.
_GENERATED = {
    "search": ("Search", "Search the vissue documentation."),
    "genindex": ("Index", "Index of the vissue documentation."),
}

_RAW = "https://raw.githubusercontent.com/leidarljos/vissue/refs/heads/main/"
_BLOB = "https://github.com/leidarljos/vissue/blob/main/"
_SOURCE_SUFFIXES = (".rst.txt", ".rst", ".org", ".md", ".txt")


class VissueHTMLTranslator(HTML5Translator):
    """Section headings start at h2 so the page title can be the only h1."""

    def __init__(self, document, builder) -> None:  # type: ignore[no-untyped-def]
        super().__init__(document, builder)
        self.initial_header_level = 2


def _read_org(path: Path) -> tuple[str, str]:
    """The page's own keywords, not an example file quoted later in the page."""
    title = ""
    description = ""
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("* "):
            break
        if not title and (match := _TITLE.match(line)):
            title = match.group(1).strip()
        elif not description and (match := _DESCRIPTION.match(line)):
            description = match.group(1).strip()
    return title, description


def _page_fields(pagename: str) -> tuple[str, str] | None:
    org = _ORG / f"{pagename}.org"
    if org.is_file():
        title, description = _read_org(org)
        if not title or not description:
            logger.warning(
                "%s needs a #+title and a #+description",
                org.relative_to(_ROOT),
                location=pagename,
            )
            return None
        return title, description
    if pagename in _GENERATED:
        return _GENERATED[pagename]
    return None


def _document_title(page_title: str, pagename: str, site: str) -> str:
    if pagename == "index" or page_title == site:
        return page_title
    return f"{page_title} - {site}"


def _stem(filename: str) -> str:
    name = filename.strip().split("/")[-1]
    for suffix in _SOURCE_SUFFIXES:
        if name.endswith(suffix):
            return name[: -len(suffix)]
    return name


def _org_repo_path(filename: str) -> str | None:
    stem = _stem(filename)
    if not stem or not (_ORG / f"{stem}.org").is_file():
        return None
    return f"docs/orgmode/{stem}.org"


def _retarget_source_links(app: Sphinx) -> None:
    """View, ask, and edit links go to the committed org file."""
    if not hasattr(app.builder, "templates"):
        return

    def view_source_link(filename: str) -> str | None:
        path = _org_repo_path(filename)
        if path is None:
            return None
        return _RAW + path

    def edit_source_link(filename: str) -> str | None:
        path = _org_repo_path(filename)
        if path is None:
            return None
        return _BLOB + path

    app.builder.templates.environment.globals["view_source_link"] = view_source_link
    app.builder.templates.environment.globals["edit_source_link"] = edit_source_link


def _page_meta(
    app: Sphinx,
    pagename: str,
    templatename: str,
    context: dict,
    doctree: object,
) -> None:
    del templatename, doctree
    fields = _page_fields(pagename)
    if fields is None:
        return
    page_title, description = fields
    site = str(app.config.html_title or app.config.project)
    context["vi_title"] = page_title
    context["vi_document_title"] = _document_title(page_title, pagename, site)
    context["vi_description"] = description
    base = str(app.config.html_baseurl or "").rstrip("/")
    if base:
        context["vi_og_image"] = f"{base}/_static/og.png"
    body = context.get("body") or ""
    # Search and the general index put their h1 in the page template, which
    # does not use this body. A body that already has an h1 keeps it.
    if "<h1" in body.lower():
        return
    context["body"] = f'<h1 class="vi-page-title">{escape(page_title)}</h1>\n{body}'


def _write_og(app: Sphinx, exception: Exception | None) -> None:
    if exception is not None or app.builder.format != "html":
        return
    out = Path(app.outdir) / "_static" / "og.png"
    out.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(_OG_SCRIPT), str(out)], check=True)


def setup(app: Sphinx) -> dict:
    app.set_translator("html", VissueHTMLTranslator, override=True)
    app.connect("builder-inited", _retarget_source_links, priority=900)
    app.connect("html-page-context", _page_meta)
    app.connect("build-finished", _write_og)
    return {"version": "1", "parallel_read_safe": True, "parallel_write_safe": True}
