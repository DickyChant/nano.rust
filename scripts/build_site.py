#!/usr/bin/env python3
"""Assemble the GitHub Pages site into ./public:

  public/index.html      landing page (docs/site/index.html)
  public/style.css       styling
  public/blog/           docs/blog/*.md rendered to HTML + an index
  public/api/            rustdoc output (target/doc)

Run `cargo doc --no-deps --all-features --workspace` first so target/doc exists.
Requires the `markdown` package for blog rendering.
"""
from __future__ import annotations

import datetime
import os
import re
import shutil
import subprocess
from pathlib import Path

import markdown

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = ROOT / "public"
RUSTDOC = ROOT / "target" / "doc"
BLOG_SRC = ROOT / "docs" / "blog"
SITE_SRC = ROOT / "docs" / "site"


def build_stamp() -> str:
    """A short, deterministic identifier for this deploy: <ref> · <short-sha> · <date>.

    Reads CI env (GITHUB_SHA / GITHUB_REF_NAME) when present, else falls back to
    git. This is the lightweight precursor to full tag-based versioning: when
    releases are tagged, GITHUB_REF_NAME becomes the version and this same string
    labels each built version.
    """
    sha = os.environ.get("GITHUB_SHA", "")
    if not sha:
        try:
            sha = subprocess.run(
                ["git", "rev-parse", "HEAD"],
                capture_output=True,
                text=True,
                cwd=ROOT,
            ).stdout.strip()
        except Exception:
            sha = ""
    short = sha[:7] if sha else "dev"
    ref = os.environ.get("GITHUB_REF_NAME", "main")
    date = os.environ.get("BUILD_DATE") or datetime.date.today().isoformat()
    return f"{ref} · {short} · {date}"


BUILD_STAMP = build_stamp()

NAV = (
    '<nav class="nav"><a href="{up}index.html">nano.rust</a> · '
    '<a href="{up}blog/index.html">blog</a> · '
    '<a href="{up}api/nano_io/index.html">api</a> · '
    '<a href="https://github.com/DickyChant/nano.rust">github</a></nav>'
)


def page(title: str, body: str, depth: int) -> str:
    up = "../" * depth
    return (
        '<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width, initial-scale=1">'
        f"<title>{title}</title>"
        f'<link rel="stylesheet" href="{up}style.css"></head><body>'
        + NAV.format(up=up)
        + f'<main class="post">{body}</main>'
        + f'<footer class="build">build {BUILD_STAMP}</footer></body></html>'
    )


def build() -> None:
    if PUBLIC.exists():
        shutil.rmtree(PUBLIC)
    PUBLIC.mkdir(parents=True)

    # landing (stamped) + style
    landing = (SITE_SRC / "index.html").read_text()
    landing = landing.replace(
        "loop.</p>",
        f'loop. · <span class="build">{BUILD_STAMP}</span></p>',
    )
    (PUBLIC / "index.html").write_text(landing)
    shutil.copy(SITE_SRC / "style.css", PUBLIC / "style.css")

    # static assets (asciinema casts, etc.) -> /
    for asset in SITE_SRC.glob("*.cast"):
        shutil.copy(asset, PUBLIC / asset.name)

    # rustdoc -> /api
    if RUSTDOC.exists():
        shutil.copytree(RUSTDOC, PUBLIC / "api")

    # blog -> /blog
    blog_out = PUBLIC / "blog"
    blog_out.mkdir(parents=True, exist_ok=True)
    posts = []
    for md_path in sorted(BLOG_SRC.glob("*.md"), reverse=True):
        text = md_path.read_text()
        m = re.search(r"^#\s+(.+)$", text, re.M)
        title = m.group(1).strip() if m else md_path.stem
        dm = re.match(r"(\d{4}-\d{2}-\d{2})", md_path.name)
        date = dm.group(1) if dm else ""
        body = markdown.markdown(text, extensions=["fenced_code", "tables"])
        (blog_out / f"{md_path.stem}.html").write_text(page(title, body, 1))
        posts.append((date, title, f"{md_path.stem}.html"))

    items = "\n".join(
        f'<li><a href="{href}">{title}</a><div class="date">{date}</div></li>'
        for date, title, href in posts
    )
    index_body = (
        "<h1>Notes / blog</h1>"
        "<p>Design notes and essays — the working drafts of our arXiv note.</p>"
        f'<ul class="postlist">{items}</ul>'
    )
    (blog_out / "index.html").write_text(page("nano.rust — blog", index_body, 1))

    print(f"site assembled at {PUBLIC} ({len(posts)} blog post(s))")


if __name__ == "__main__":
    build()
