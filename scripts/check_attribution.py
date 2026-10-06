# Added by HyperSpaces (2026): Check license integrity, fork notices and the provenance inventory. Original: durable-streams 0.1.5, Apache-2.0.
"""Check every added or modified file against the imported release."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

IMPORTED_BASE = "d286a4b2ebfd040908ae9f07849582707774a125"
# Preserve the received license text exactly, including its original whitespace.
LICENSE_SHA256 = "0d542e0c8804e39aa7f37eb00da5a762149dc682d7829451287e11b938e94594"


def check(repo, base):
    diff = subprocess.run(
        ["git", "diff", "--name-status", "--no-renames", "--diff-filter=AMT", "-z", base, "HEAD", "--"],
        cwd=repo, check=True, capture_output=True,
    ).stdout.decode().split("\0")[:-1]
    changed = {path: "Added" if status == "A" else "Modified"
               for status, path in zip(diff[::2], diff[1::2])}
    inventory = {}
    errors = []
    if hashlib.sha256((repo / "LICENSE").read_bytes()).hexdigest() != LICENSE_SHA256:
        errors.append("LICENSE: must match the exact received Apache-2.0 text (SHA-256 "
                      + LICENSE_SHA256 + ")")
    for line in (repo / "UPSTREAM.md").read_text().splitlines():
        match = re.fullmatch(r"\| `([^`]+)` \| (Added|Modified) \| ([^|]+) \|", line)
        if match:
            path, kind, description = match.groups()
            if path in inventory:
                errors.append(f"{path}: duplicate UPSTREAM.md entry")
            if not description.strip():
                errors.append(f"{path}: empty UPSTREAM.md description")
            inventory[path] = kind

    for path, kind in changed.items():
        if inventory.get(path) != kind:
            errors.append(f"{path}: missing or incorrect {kind} entry in UPSTREAM.md")
        # LICENSE remains the exact legal text; its attribution is in NOTICE.
        if path == "LICENSE":
            continue
        extension = Path(path).suffix
        if extension == ".md":
            prefix, suffix = "<!-- ", " -->"
        elif extension in {".rs", ".js", ".mjs", ".cjs", ".ts"}:
            prefix, suffix = "// ", ""
        elif path == "NOTICE" or path.endswith(".NOTICE"):
            prefix, suffix = "", ""
        elif (extension in {".py", ".sh", ".toml", ".lock", ".yml", ".yaml"}
              or Path(path).name in {"Dockerfile", ".dockerignore", ".gitignore"}
              or path.endswith(".toml.orig")):
            prefix, suffix = "# ", ""
        else:
            # Commentless formats carry their notice alongside the unchanged format.
            companion = path + ".NOTICE"
            if companion not in changed:
                errors.append(f"{path}: requires adjacent {companion} and its inventory entry")
                continue
            try:
                lines = (repo / companion).read_text().splitlines()
                line = lines[1] if len(lines) > 1 else ""
                if extension == ".json":
                    json.loads((repo / path).read_bytes())
            except (OSError, ValueError) as error:
                errors.append(f"{path}: invalid source or adjacent notice: {error}")
                continue
            prefix, suffix = "", ""
        if prefix or path == "NOTICE" or path.endswith(".NOTICE"):
            try:
                lines = (repo / path).read_text().splitlines()
            except (OSError, UnicodeError) as error:
                errors.append(f"{path}: cannot read notice: {error}")
                continue
            if lines and (lines[0].startswith("#!") or lines[0].startswith("# syntax=")):
                lines = lines[1:]
            line = lines[0] if lines else ""
        notice = (re.escape(prefix + kind + " by HyperSpaces (") + r"\d{4}\): \S[^\n]+"
                  + re.escape(". Original: durable-streams 0.1.5, Apache-2.0." + suffix))
        if not re.fullmatch(notice, line):
            errors.append(f"{path}: prominent {kind} notice must be the first line "
                          "(after a shebang or Docker syntax directive), or the second "
                          "line of its adjacent .NOTICE for a commentless format")
    for path in inventory.keys() - changed.keys():
        errors.append(f"{path}: stale UPSTREAM.md entry; file no longer differs from the import")
    return errors, len(changed)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--base", default=IMPORTED_BASE)
    args = parser.parse_args()
    try:
        errors, count = check(args.repo, args.base)
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"Attribution check failed: {error}", file=sys.stderr)
        return 1
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Attribution check passed: {count} changed files have inventory entries "
          "and required notices (LICENSE is exempt from notices).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
