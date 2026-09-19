"""Sphinx configuration for the vissue project site (Shibuya theme)."""

from __future__ import annotations

from pathlib import Path

_DOCS = Path(__file__).resolve().parent
_ROOT = _DOCS.parent.parent

project = "vissue"
copyright = "2026, Rohit Goswami"
author = "Rohit Goswami"
release = "0.13.1"
version = "0.13"

extensions = [
    "sphinx.ext.mathjax",
    "sphinx_copybutton",
    "sphinx_design",
]

templates_path = ["_templates"]
exclude_patterns: list[str] = []

html_theme = "shibuya"
html_static_path = ["_static"]
html_favicon = "_static/favicon.svg"
html_logo = "_static/logo.svg"
html_title = "vissue"
html_css_files = ["custom.css"]

html_context = {
    "source_type": "github",
    "source_user": "leidarljos",
    "source_repo": "vissue",
    "source_version": "main",
    "source_docs_path": "/docs/source/",
}

html_theme_options = {
    "accent_color": "gold",
    "color_mode": "dark",
    "dark_code": True,
    "github_url": "https://github.com/leidarljos/vissue",
    "nav_links": [
        {"title": "Get started", "url": "getting-started"},
        {"title": "How-to", "url": "howto"},
        {"title": "Harness", "url": "harness"},
        {"title": "Reference", "url": "reference"},
        {"title": "Control", "url": "control"},
        {"title": "Explanation", "url": "explanation"},
        {"title": "Emacs", "url": "emacs"},
        {"title": "Search", "url": "search"},
    ],
}

# Offline builds must not reach for an inventory.
intersphinx_mapping: dict = {}

copybutton_prompt_text = r"\$ "
copybutton_prompt_is_regexp = True
