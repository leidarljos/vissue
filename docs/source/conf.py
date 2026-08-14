"""Sphinx configuration for the vissue project site (Shibuya theme)."""

from __future__ import annotations

from pathlib import Path

_DOCS = Path(__file__).resolve().parent
_ROOT = _DOCS.parent.parent

project = "vissue"
copyright = "2026, Rohit Goswami"
author = "Rohit Goswami"
release = "0.1.0"
version = "0.1"

extensions = [
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
    "source_user": "HaoZeke",
    "source_repo": "vissue",
    "source_version": "main",
    "source_docs_path": "/docs/source/",
}

html_theme_options = {
    "accent_color": "teal",
    "github_url": "https://github.com/HaoZeke/vissue",
    "nav_links": [
        {"title": "Get started", "url": "getting-started"},
        {"title": "How-to", "url": "howto"},
        {"title": "Reference", "url": "reference"},
        {"title": "Explanation", "url": "explanation"},
        {"title": "Ecosystem", "url": "ecosystem"},
    ],
}

# Offline builds must not reach for an inventory.
intersphinx_mapping: dict = {}

copybutton_prompt_text = r"\$ "
copybutton_prompt_is_regexp = True
