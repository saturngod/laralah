#!/usr/bin/env python3
"""Single source of truth for the Laralah release version.

The version lives in three places and they must always agree:
  Cargo.toml ([package] version), Cargo.lock (the `laralah` package entry),
  and assets/macos/Info.plist (CFBundleShortVersionString and CFBundleVersion).
The Windows exe resource and the Linux .deb read the version from Cargo at build time.

Usage (from anywhere in the repo):
  python3 scripts/release-version.py check [--tag v0.1.0] [--ref <git-ref>]
  python3 scripts/release-version.py set <0.1.0 | v0.1.0>
  python3 scripts/release-version.py tag <0.1.0 | v0.1.0> [-m "version 0.1.0"]

`check --tag` is what the pre-push hook (githooks/pre-push) and the release
workflow run, so a local failure and a CI failure mean the same thing. `tag`
bumps every file, commits them and creates the annotated tag in one step.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = "Cargo.toml"
CARGO_LOCK = "Cargo.lock"
INFO_PLIST = "assets/macos/Info.plist"
VERSION_FILES = [CARGO_TOML, CARGO_LOCK, INFO_PLIST]
PACKAGE_NAME = "laralah"
PLIST_KEYS = ["CFBundleShortVersionString", "CFBundleVersion"]

SEMVER = r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?"
SEMVER_RE = re.compile(rf"^{SEMVER}$")
TAG_RE = re.compile(rf"^v{SEMVER}$")


def fail(message: str) -> "NoReturn":  # type: ignore[name-defined]
    print(f"release-version: {message}", file=sys.stderr)
    sys.exit(1)


def git(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True)


def read_worktree(path: str) -> str:
    file = ROOT / path
    if not file.exists():
        fail(f"{path}: file not found")
    return file.read_text(encoding="utf-8")


def read_at_ref(ref: str, path: str) -> str:
    peeled = git("rev-list", "-n", "1", ref)
    commit = peeled.stdout.strip()
    if peeled.returncode != 0 or not commit:
        fail(f'cannot resolve git ref "{ref}": {peeled.stderr.strip()}')
    shown = git("show", f"{commit}:{path}")
    if shown.returncode != 0:
        fail(f"{path}: not present at {ref}")
    return shown.stdout


# --- Cargo.toml -------------------------------------------------------------

SECTION_RE = re.compile(r"^\s*\[([^\]]+)\]\s*(?:#.*)?$")
VERSION_LINE_RE = re.compile(r'^(\s*version\s*=\s*")([^"]*)(".*)$')


def cargo_toml_line(lines: list[str]) -> int:
    in_package = False
    for i, line in enumerate(lines):
        section = SECTION_RE.match(line)
        if section:
            in_package = section.group(1).strip() == "package"
            continue
        if in_package and VERSION_LINE_RE.match(line):
            return i
    fail(f"{CARGO_TOML}: no version under [package]")


# --- Cargo.lock -------------------------------------------------------------


def cargo_lock_line(lines: list[str]) -> int:
    name = None
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped == "[[package]]":
            name = None
            continue
        m = re.match(r'^name\s*=\s*"([^"]+)"$', stripped)
        if m:
            name = m.group(1)
            continue
        if name == PACKAGE_NAME and VERSION_LINE_RE.match(line):
            return i
    fail(f'{CARGO_LOCK}: no version for the "{PACKAGE_NAME}" package')


# --- Info.plist -------------------------------------------------------------


def plist_lines(lines: list[str]) -> dict[str, int]:
    found: dict[str, int] = {}
    for i, line in enumerate(lines):
        for key in PLIST_KEYS:
            if f"<key>{key}</key>" in line:
                for j in range(i + 1, min(i + 3, len(lines))):
                    if "<string>" in lines[j]:
                        found[key] = j
                        break
    missing = [k for k in PLIST_KEYS if k not in found]
    if missing:
        fail(f"{INFO_PLIST}: missing {', '.join(missing)}")
    return found


def plist_value(line: str) -> str:
    m = re.search(r"<string>([^<]*)</string>", line)
    return m.group(1) if m else "?"


# --- read / write -----------------------------------------------------------


def load_versions(ref: str | None) -> list[tuple[str, str]]:
    versions: list[tuple[str, str]] = []
    for path in VERSION_FILES:
        content = read_at_ref(ref, path) if ref else read_worktree(path)
        lines = content.split("\n")
        if path == CARGO_TOML:
            versions.append((path, VERSION_LINE_RE.match(lines[cargo_toml_line(lines)]).group(2)))
        elif path == CARGO_LOCK:
            versions.append((path, VERSION_LINE_RE.match(lines[cargo_lock_line(lines)]).group(2)))
        else:
            for key, idx in plist_lines(lines).items():
                versions.append((f"{path} ({key})", plist_value(lines[idx])))
    return versions


def write_version(version: str) -> None:
    print(f"setting version to {version}:")
    for path in VERSION_FILES:
        content = read_worktree(path)
        lines = content.split("\n")
        changed = []
        if path in (CARGO_TOML, CARGO_LOCK):
            idx = cargo_toml_line(lines) if path == CARGO_TOML else cargo_lock_line(lines)
            m = VERSION_LINE_RE.match(lines[idx])
            if m.group(2) != version:
                changed.append(m.group(2))
                lines[idx] = f"{m.group(1)}{version}{m.group(3)}"
        else:
            for idx in plist_lines(lines).values():
                old = plist_value(lines[idx])
                if old != version:
                    changed.append(old)
                    lines[idx] = re.sub(r"<string>[^<]*</string>", f"<string>{version}</string>", lines[idx])
        if changed:
            (ROOT / path).write_text("\n".join(lines), encoding="utf-8")
            print(f"  ~ {path}: {changed[0]} -> {version}")
        else:
            print(f"  = {path} already {version}")


def normalize(raw: str) -> str:
    version = raw[1:] if raw.startswith("v") else raw
    if not SEMVER_RE.match(version):
        fail(f'expected a version like 0.1.0, got "{raw}"')
    return version


# --- commands ---------------------------------------------------------------


def cmd_check(tag: str | None, ref: str | None) -> None:
    versions = load_versions(ref)
    at = f" at {ref}" if ref else ""
    if tag is not None:
        if not TAG_RE.match(tag):
            fail(f'expected a version tag like v0.1.0, got "{tag}"')
        expected = tag[1:]
        mismatched = [(p, v) for p, v in versions if v != expected]
        if mismatched:
            for path, v in mismatched:
                print(f"  {path}: version {v} does not match release tag {tag}", file=sys.stderr)
            print(f"  fix: python3 scripts/release-version.py set {expected}  (then commit before tagging)",
                  file=sys.stderr)
            sys.exit(1)
        print(f"✓ {tag}: all version fields agree ({expected}){at}")
        return
    unique = {v for _, v in versions}
    if len(unique) > 1:
        for path, v in versions:
            print(f"  {path}: {v}", file=sys.stderr)
        print("  fix: python3 scripts/release-version.py set <version> to sync them", file=sys.stderr)
        sys.exit(1)
    print(f"✓ all version fields agree ({unique.pop()}){at}")


def cmd_tag(raw: str, message: str | None) -> None:
    version = normalize(raw)
    tag = f"v{version}"
    if git("rev-parse", "--show-toplevel").returncode != 0:
        fail("not inside a git checkout")

    status = git("status", "--porcelain")
    if status.returncode != 0:
        fail(f"git status failed: {status.stderr.strip()}")
    blocking = [
        line[3:].strip('"')
        for line in status.stdout.rstrip().split("\n")
        if line and not line.startswith("??") and line[3:].strip('"') not in VERSION_FILES
    ]
    if blocking:
        print("refusing to tag with unrelated changes present:", file=sys.stderr)
        for path in blocking:
            print(f"  {path}", file=sys.stderr)
        fail("commit or stash them first, then re-run")

    if git("rev-parse", "-q", "--verify", f"refs/tags/{tag}").returncode == 0:
        fail(f"tag {tag} already exists")

    write_version(version)
    add = git("add", "--", *VERSION_FILES)
    if add.returncode != 0:
        fail(f"git add failed: {add.stderr.strip()}")
    if git("diff", "--cached", "--quiet").returncode == 0:
        print("  = version files already committed, nothing to commit")
    else:
        commit = git("commit", "-m", f"chore(release): {tag}")
        if commit.returncode != 0:
            fail(f"git commit failed: {(commit.stderr or commit.stdout).strip()}")
        print(f"  + committed version files (chore(release): {tag})")

    created = git("tag", "-a", tag, "-m", message or f"version {version}")
    if created.returncode != 0:
        fail(f"git tag failed: {(created.stderr or created.stdout).strip()}")
    print(f"✓ created annotated tag {tag}")
    branch = git("branch", "--show-current").stdout.strip()
    print(f"next: git push origin {branch + ' ' if branch else ''}{tag}")
    print("the pre-push hook re-validates the tag before it leaves your machine")


def main() -> None:
    parser = argparse.ArgumentParser(prog="release-version.py", description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("check", help="verify every version field agrees (and matches --tag)")
    check.add_argument("--tag")
    check.add_argument("--ref", help="read the files at this git ref instead of the worktree")
    set_ = sub.add_parser("set", help="write the version to every file")
    set_.add_argument("version")
    tag = sub.add_parser("tag", help="set the version, commit it and create the annotated tag")
    tag.add_argument("version")
    tag.add_argument("-m", "--message")
    args = parser.parse_args()

    if args.command == "check":
        cmd_check(args.tag, args.ref)
    elif args.command == "set":
        write_version(normalize(args.version))
    else:
        cmd_tag(args.version, args.message)


if __name__ == "__main__":
    main()
