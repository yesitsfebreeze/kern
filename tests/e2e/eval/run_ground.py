"""kern's own ground-truth benchmark (tests/e2e/eval/ground.json) — the one
corpus that is committed, license-free, and small enough to run on every
change.

Two paths over the same corpus and the same turn-level evidence labels:

- `direct` (default): `kern ingest --file` per session — documents, no LLM.
  The same protocol as the LoCoMo/LongMemEval runners; the verbatim floor.
- `distill`: every session transcript goes through `.kern/intake/` and
  `kern intake drain` — the real pipeline (LLM distills typed claims with
  turn-level provenance). Retrieved claims are mapped back to the turns they
  cite via `kern get`'s Source line, so recall@k measures ingest quality and
  retrieval together, against the same labels the direct path answers to.
  This is the item-104 shape: the graph's number, not the embedder's.

`--path both` runs both into separate projects and reports them side by side.
"""

import json
import re
import tempfile
from collections import defaultdict
from pathlib import Path

import score
from common import (
	LabelMap,
	argparser,
	base_report,
	build_kern,
	ingest_session,
	make_project,
	open_endpoints,
	parse_args,
	ranked_keys,
	sanitize,
	write_report,
)

GROUND = Path(__file__).parent / "ground.json"

# `kern get` provenance line for a distilled claim, e.g.
# `Source: session://session:s3 §2,5` — session id after the intake file's
# stem, cited 1-based turns comma-joined, absent when the distiller cited none.
SOURCE_LINE = re.compile(r"^Source:\s+session://session:([^\s§]+)(?:\s+§([\d,]+))?\s*$", re.M)


def load_ground():
	d = json.loads(GROUND.read_text())
	sessions = [(s["id"], [sanitize(t) for t in s["turns"]]) for s in d["sessions"]]
	questions = [
		(q["q"], q["category"], [("t", sid, turn) for sid, turn, _anchor in q["evidence"]])
		for q in d["questions"]
	]
	return d["version"], sessions, questions


def ingest_direct(project, sessions, tmp):
	"""Documents via `kern ingest --file`; labels map printed text to turns."""
	labels = LabelMap()
	counts = defaultdict(int)
	for sid, turns in sessions:
		for i, text in enumerate(turns):
			labels.add(text, ("t", sid, i + 1))
		status, _ = ingest_session(project, turns, tmp, sid)
		counts[f"ingest_{status}"] += 1
	counts["label_collisions"] = labels.collisions
	return labels, counts


def ingest_distill(project, sessions, retries=3):
	"""Transcripts through the intake queue; the drain runs the real distill.

	A transcript the model answers with prose instead of JSON stays queued for
	retry — that is the pipeline's own recovery path, so the runner retries the
	drain rather than scoring a partial corpus (numbers over 7 of 8 sessions
	would be quietly wrong).
	"""
	intake = project.cwd / ".kern" / "intake"
	intake.mkdir(parents=True, exist_ok=True)
	for sid, turns in sessions:
		(intake / f"{sid}.txt").write_text("\n\n".join(turns))
	stdout = stderr = ""
	for attempt in range(1, retries + 1):
		stdout, stderr = project.run("intake", "drain", timeout=3600)
		pending = re.search(r"pending=(\d+)", stdout)
		if pending and pending.group(1) == "0":
			return {"drained": len(sessions), "drain_attempts": attempt}
		if not pending:
			# The one-line success shape: everything drained on this pass.
			m = re.search(r"drained (\d+) of (\d+) pending", stdout)
			if m and m.group(1) == m.group(2):
				return {"drained": len(sessions), "drain_attempts": attempt}
		print(f"drain attempt {attempt}: {stdout.splitlines()[0] if stdout else stderr}")
	raise RuntimeError(f"drain incomplete after {retries} attempts: out={stdout} err={stderr}")


def claim_keys(project, short_id, cache):
	"""Turn keys a distilled claim cites, via `kern get`'s Source line."""
	if short_id not in cache:
		stdout, _ = project.run("get", short_id)
		keys = set()
		m = SOURCE_LINE.search(stdout)
		if m and m.group(2):
			sid = m.group(1)
			keys = {("t", sid, int(t)) for t in m.group(2).split(",")}
		cache[short_id] = keys
	return cache[short_id]


def ranked_by_provenance(project, question, cache):
	"""Query once; map each hit to the turns its Source cites."""
	import time

	from ranking import hits

	t0 = time.monotonic()
	stdout, _ = project.run("query", question)
	secs = time.monotonic() - t0
	ranked = [claim_keys(project, h.short_id, cache) for h in hits(stdout)]
	return ranked, secs


def run_path(path, kern_bin, sessions, questions, args, embed, llm_url, reason):
	counts = defaultdict(int)
	latencies = []
	scored = []
	by_category = defaultdict(list)
	with tempfile.TemporaryDirectory(prefix=f"kern-ground-{path}-") as tmp:
		tmp = Path(tmp)
		project = make_project(kern_bin, tmp, embed, llm_url, args.k)
		if path == "distill":
			project.write_config(embed=embed, reason=reason)
			counts.update(ingest_distill(project, sessions))
			cache = {}
			uncited = 0
			total_hits = 0
			for q, cat, gold in questions:
				ranked, secs = ranked_by_provenance(project, q, cache)
				latencies.append(secs)
				total_hits += len(ranked)
				uncited += sum(1 for keys in ranked if not keys)
				r = score.question_result(gold, ranked)
				scored.append(r)
				by_category[cat].append(r)
			counts["claims_retrieved_distinct"] = len(cache)
			counts["hits_without_cited_turns"] = uncited
			counts["hits_total"] = total_hits
		else:
			labels, c = ingest_direct(project, sessions, tmp)
			counts.update(c)
			for q, cat, gold in questions:
				ranked, secs = ranked_keys(project, q, labels)
				latencies.append(secs)
				r = score.question_result(gold, ranked)
				scored.append(r)
				by_category[cat].append(r)
		project.kill_all()

	return {
		"counts": dict(counts),
		"turn_granularity": score.metrics(scored, args.k),
		"by_category": {c: score.metrics(qs, args.k) for c, qs in sorted(by_category.items())},
		"query_latency_secs": {
			"note": "cold-process CLI wall clock: spawn + graph load + embed + retrieve",
			"p50": score.percentile(latencies, 50),
			"p95": score.percentile(latencies, 95),
		},
	}


def main():
	parser = argparser(__doc__)
	parser.add_argument("--path", choices=["direct", "distill", "both"], default="direct")
	parser.add_argument("--llm-url", default=None, help="completion endpoint for distill (default: --embed-url)")
	parser.add_argument("--llm-model", default="qwen3.5:4b", help="completion model for distill")
	args = parse_args(parser)

	version, sessions, questions = load_ground()
	embed, llm_url, closer = open_endpoints(args)
	reason = (args.llm_url or args.embed_url, args.llm_model)
	if args.fake_llm:
		reason = None  # make_project's fake stays; distill parses its stub
	kern_bin = build_kern()
	paths = ["direct", "distill"] if args.path == "both" else [args.path]

	try:
		report = base_report(args, f"kern-ground v{version} ({len(questions)} questions, turn-level evidence)")
		if "distill" in paths:
			report["protocol"] = (
				"ground corpus, two paths: direct-path documents (verbatim floor) "
				"and intake-drain distilled claims scored by cited-turn provenance "
				"(`kern get` Source line) — same turn-level labels for both"
			)
			report["distill_model"] = "fake-reason" if args.fake_llm else args.llm_model
		for p in paths:
			print(f"== path: {p} ==")
			report[p] = run_path(p, kern_bin, sessions, questions, args, embed, llm_url, reason)
			print(json.dumps(report[p]["turn_granularity"], indent=1))
	finally:
		closer()

	out = write_report(args.report_dir, "ground", report)
	print(f"report: {out}")


if __name__ == "__main__":
	main()
