#!/usr/bin/env python3
"""Measured repository snapshot: build, tests, code shape, oracle state.

Every number comes from a run in this invocation — nothing is cached or
assumed. `--json` emits machine-readable output for CI diffing.
"""

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def run(cmd, timeout=600):
    t0 = time.monotonic()
    try:
        p = subprocess.run(
            cmd, cwd=ROOT, capture_output=True, text=True, timeout=timeout
        )
        return p.returncode, p.stdout, p.stderr, round(time.monotonic() - t0, 1)
    except FileNotFoundError:
        return None, "", f"{cmd[0]}: not found", 0.0
    except subprocess.TimeoutExpired:
        return None, "", f"timeout after {timeout}s", float(timeout)


def section_git():
    _, branch, _, _ = run(["git", "rev-parse", "--abbrev-ref", "HEAD"])
    _, head, _, _ = run(["git", "log", "-1", "--format=%h %s"])
    _, status, _, _ = run(["git", "status", "--porcelain"])
    return {
        "branch": branch.strip(),
        "head": head.strip(),
        "dirty_files": len(status.splitlines()),
    }


def section_build():
    code, _, err, secs = run(["cargo", "check", "--workspace"])
    warnings = len(re.findall(r"^warning", err, re.M))
    return {"ok": code == 0, "seconds": secs, "warnings": warnings}


def section_tests():
    code, out, err, secs = run(
        ["cargo", "nextest", "list", "--workspace", "--message-format", "json"]
    )
    if code != 0:
        return {"ok": False, "error": err.strip().splitlines()[-1] if err else "?"}
    data = json.loads(out)
    suites = data.get("rust-suites", {})
    count = sum(len(s.get("testcases", {})) for s in suites.values())
    return {"ok": True, "tests": count, "binaries": len(suites), "seconds": secs}


def section_code():
    code, out, _, _ = run(["tokei", "src", "--output", "json"])
    if code != 0:
        return {}
    rust = json.loads(out).get("Rust", {})
    per_module = {}
    for report in rust.get("reports", []):
        parts = Path(report["name"]).parts
        module = parts[1] if len(parts) > 2 else Path(parts[-1]).stem
        per_module[module] = per_module.get(module, 0) + report["stats"]["code"]
    _, todos, _, _ = run(["git", "grep", "-cE", "TODO|FIXME|XXX", "--", "src"])
    todo_count = sum(int(line.rsplit(":", 1)[1]) for line in todos.splitlines())
    return {
        "rust_loc": rust.get("code", 0),
        "files": len(rust.get("reports", [])),
        "todos": todo_count,
        "modules": dict(
            sorted(per_module.items(), key=lambda kv: -kv[1])[:12]
        ),
    }


def oracle_file(name):
    """Governance files moved to docs/oracle/ (2026-07-20); accept either home.

    Returns None when genuinely absent so a missing file reports as missing
    rather than silently counting zero — a plausible-looking 0 is worse than
    an error, and this script exists to report only measured things.
    """
    for candidate in (ROOT / "docs/oracle" / name, ROOT / name):
        if candidate.exists():
            return candidate.read_text()
    return None


def section_oracle():
    out, missing = {}, []

    def text(name):
        body = oracle_file(name)
        if body is None:
            missing.append(name)
        return body or ""

    features = text("FEATURES.md")
    out["features_active"] = len(re.findall(r"`active`\s*$", features, re.M))
    out["features_building"] = len(re.findall(r"`building`\s*$", features, re.M))
    roadmap = text("ROADMAP.md")
    out["roadmap_questions"] = len(re.findall(r"^\d+\. ", roadmap, re.M))
    out["roadmap_undecidable"] = len(re.findall(r"none yet", roadmap))
    out["decisions_recorded"] = text("CHANGELOG.md").count("Decided by:")
    out["specialists"] = len(re.findall(r"^## ", text("SPECIALISTS.md"), re.M))
    local = (ROOT / "CLAUDE.local.md").read_text() if (ROOT / "CLAUDE.local.md").exists() else ""
    out["behaviors_pinned"] = len(re.findall(r"behaviors/[a-z-]+\.md", local))
    out["missing_files"] = missing
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--skip-tests", action="store_true", help="skip nextest list (slow on a cold target)")
    args = ap.parse_args()

    snap = {"git": section_git(), "build": section_build()}
    if not args.skip_tests:
        snap["tests"] = section_tests()
    snap["code"] = section_code()
    snap["oracle"] = section_oracle()

    if args.json:
        print(json.dumps(snap, indent=2))
        return 0 if snap["build"]["ok"] else 1

    g, b, c, o = snap["git"], snap["build"], snap["code"], snap["oracle"]
    print(f"kern @ {g['head']}  ({g['branch']}, {g['dirty_files']} dirty)")
    print(f"build     {'ok' if b['ok'] else 'FAILING'} in {b['seconds']}s, {b['warnings']} warnings")
    if "tests" in snap:
        t = snap["tests"]
        if t["ok"]:
            print(f"tests     {t['tests']} across {t['binaries']} binaries (listed in {t['seconds']}s)")
        else:
            print(f"tests     LIST FAILED: {t['error']}")
    print(f"code      {c['rust_loc']} LOC rust, {c['files']} files, {c['todos']} TODO/FIXME")
    top = ", ".join(f"{m} {n}" for m, n in list(c["modules"].items())[:6])
    print(f"          top: {top}")
    print(
        f"oracle    {o['features_active']} active + {o['features_building']} building features, "
        f"{o['roadmap_questions']} open questions ({o['roadmap_undecidable']} undecidable), "
        f"{o['decisions_recorded']} decisions, {o['behaviors_pinned']} behaviors, {o['specialists']} specialists"
    )
    if o["missing_files"]:
        print(f"          NOT FOUND: {', '.join(o['missing_files'])} (counts above understate)")
    return 0 if b["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
