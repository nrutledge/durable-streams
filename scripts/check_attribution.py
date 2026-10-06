# Added by HyperSpaces (2026): Check fork file notices and the provenance inventory. Original: durable-streams 0.1.5, Apache-2.0.
"""Check every added or modified file against the imported release."""

import argparse
from pathlib import Path
import re
import subprocess
import sys

IMPORTED_BASE = "d286a4b2ebfd040908ae9f07849582707774a125"


def check(repo, base):
    diff = subprocess.run(
        ["git", "diff", "--name-status", "--no-renames", "--diff-filter=AMT", "-z", base, "HEAD", "--"],
        cwd=repo, check=True, capture_output=True,
    ).stdout.decode().split("\0")[:-1]
    changed = {path: "Added" if status == "A" else "Modified"
               for status, path in zip(diff[::2], diff[1::2])}
    inventory = {}
    errors = []
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
        try:
            lines = (repo / path).read_text().splitlines()
        except (OSError, UnicodeError) as error:
            errors.append(f"{path}: cannot read notice: {error}")
            continue
        if lines and (lines[0].startswith("#!") or lines[0].startswith("# syntax=")):
            lines = lines[1:]
        line = lines[0] if lines else ""
        if Path(path).suffix == ".md":
            prefix, suffix = "<!-- ", " -->"
        elif Path(path).suffix == ".rs":
            prefix, suffix = "// ", ""
        elif path in {"LICENSE", "NOTICE"}:
            prefix, suffix = "", ""
        else:
            prefix, suffix = "# ", ""
        notice = (re.escape(prefix + kind + " by HyperSpaces (") + r"\d{4}\): \S[^\n]+"
                  + re.escape(". Original: durable-streams 0.1.5, Apache-2.0." + suffix))
        if not re.fullmatch(notice, line):
            errors.append(f"{path}: prominent {kind} notice must be the first line "
                          "(after a shebang or Docker syntax directive)")
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
