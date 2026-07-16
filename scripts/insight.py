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


def section_oracle():
    out = {}
    features = (ROOT / "FEATURES.md").read_text() if (ROOT / "FEATURES.md").exists() else ""
    out["features_active"] = len(re.findall(r"`active`\s*$", features, re.M))
    out["features_building"] = len(re.findall(r"`building`\s*$", features, re.M))
    roadmap = (ROOT / "ROADMAP.md").read_text() if (ROOT / "ROADMAP.md").exists() else ""
    out["roadmap_questions"] = len(re.findall(r"^\d+\. ", roadmap, re.M))
    out["roadmap_undecidable"] = len(re.findall(r"none yet", roadmap))
    changelog = (ROOT / "CHANGELOG.md").read_text() if (ROOT / "CHANGELOG.md").exists() else ""
    out["decisions_recorded"] = changelog.count("Decided by:")
    local = (ROOT / "CLAUDE.local.md").read_text() if (ROOT / "CLAUDE.local.md").exists() else ""
    out["behaviors_pinned"] = len(re.findall(r"behaviors/[a-z-]+\.md", local))
    out["specialists"] = len(re.findall(r"^## ", (ROOT / "SPECIALISTS.md").read_text(), re.M)) if (ROOT / "SPECIALISTS.md").exists() else 0
    return out


def section_bench():
    return {
        "retrieval_baseline": (ROOT / "docs/kern/bench-retrieval.md").exists(),
        "workload_trace": (ROOT / "traces/workload.json").exists(),
        "locomo_baseline": any(ROOT.glob("docs/kern/*locomo*")),
    }


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
    snap["bench"] = section_bench()

    if args.json:
        print(json.dumps(snap, indent=2))
        return 0 if snap["build"]["ok"] else 1

    g, b, c, o, be = snap["git"], snap["build"], snap["code"], snap["oracle"], snap["bench"]
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
    missing = [k for k, v in be.items() if not v]
    print(f"bench     {'all baselines present' if not missing else 'missing: ' + ', '.join(missing)}")
    return 0 if b["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
