#!/usr/bin/env python3
"""Read-only context builder for the /update-version skill."""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from datetime import date
from pathlib import Path

MANIFEST = "crates/app/Cargo.toml"
VERSION_RE = re.compile(
    r'^version\s*=\s*"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"\s*$'
)
EXPLICIT_RE = re.compile(r"^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")
VERSION_COMMIT_RE = re.compile(r"^feat: update version to v\d+\.\d+\.\d+")


def git(repo: Path, *args: str) -> str:
    proc = subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        detail = proc.stderr.strip() or proc.stdout.strip()
        raise RuntimeError(f"git {' '.join(args)} failed: {detail}")
    return proc.stdout.strip()


def read_current_version(repo: Path) -> tuple[str, int]:
    lines = (repo / MANIFEST).read_text(encoding="utf-8").splitlines()
    in_package = False
    for number, line in enumerate(lines, 1):
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            in_package = stripped == "[package]"
            continue
        if not in_package:
            continue
        match = VERSION_RE.match(line)
        if match:
            return ".".join(match.groups()), number
    raise RuntimeError(f"no valid version line in [package] of {MANIFEST}")


def bump_patch(version: str) -> str:
    major, minor, patch = (int(part) for part in version.split("."))
    return f"v{major}.{minor}.{patch + 1}"


def resolve_target(requested: str | None, current: str) -> dict:
    requested = requested or "next"
    if requested == "next":
        return {
            "mode": "next",
            "requested": requested,
            "target": bump_patch(current),
            "fallback_reason": None,
        }
    match = EXPLICIT_RE.fullmatch(requested)
    if match:
        return {
            "mode": "explicit",
            "requested": requested,
            "target": "v" + ".".join(match.groups()),
            "fallback_reason": None,
        }
    return {
        "mode": "next",
        "requested": requested,
        "target": bump_patch(current),
        "fallback_reason": (
            f"{requested!r} is not vMAJOR.MINOR.PATCH with integer components; "
            "using next"
        ),
    }


def last_version_commit(repo: Path) -> tuple[str | None, str | None]:
    output = git(repo, "log", "--format=%H%x09%s", "HEAD")
    for line in output.splitlines():
        commit_hash, _, subject = line.partition("\t")
        if VERSION_COMMIT_RE.match(subject):
            return commit_hash, subject
    return None, None


def commits_since(repo: Path, base: str | None) -> tuple[list[dict], str]:
    fmt = "--format=%h%x09%s"
    if base:
        short = git(repo, "rev-parse", "--short", base)
        output = git(repo, "log", "--reverse", fmt, f"{base}..HEAD")
        label = f"{short}..HEAD"
    else:
        output = git(repo, "log", "--reverse", fmt, "HEAD")
        label = "HEAD (all history)"
    commits = []
    for line in output.splitlines():
        short_hash, _, subject = line.partition("\t")
        commits.append({"hash": short_hash, "subject": subject})
    return commits, label


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", nargs="?", help="next or vMAJOR.MINOR.PATCH")
    parser.add_argument("--repo", default=".", help="repository path (default: cwd)")
    args = parser.parse_args()

    try:
        start = Path(args.repo).resolve()
        root = Path(git(start, "rev-parse", "--show-toplevel")).resolve()
        current, line_number = read_current_version(root)
        resolution = resolve_target(args.version, current)
        base, base_subject = last_version_commit(root)
        commits, range_label = commits_since(root, base)
        target = resolution["target"]
    except (OSError, RuntimeError, UnicodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    result = {
        "repo": str(root),
        "manifest": MANIFEST,
        "manifest_version_line": line_number,
        "current_version": current,
        "manifest_version": target[1:],
        "target_version": target,
        "note_path": f"docs/releases/{target}.md",
        "today": date.today().isoformat(),
        **resolution,
        "previous_version_commit": base,
        "previous_version_subject": base_subject,
        "range": range_label,
        "commits": commits,
    }
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
