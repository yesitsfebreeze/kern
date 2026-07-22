"""LongMemEval-S, retrieval-only.

Per question: a fresh kern project ingests the full haystack (every session,
every turn, direct path, no LLM), then `kern query` runs the question and the
evidence ranks are scored at two granularities from one retrieval pass:
- session: any hit from an `answer_session_ids` session in the top k
- turn: the `has_answer` turns themselves in the top k

The full set is 500 questions x ~115k haystack tokens — hours of embedding on
a real model — so the default is a seeded 100-question sample, logged loudly;
`--full` runs everything.
"""

import json
import random
import tempfile
from collections import defaultdict
from pathlib import Path

import score
from common import (
	DATA_DIR,
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

SAMPLE_SEED = 13


def run_question(kern_bin, item, args, embed, llm_url, counts, latencies):
	labels = LabelMap()
	with tempfile.TemporaryDirectory(prefix="kern-lme-") as tmp:
		tmp = Path(tmp)
		project = make_project(kern_bin, tmp, embed, llm_url, args.k)
		gold_turns = []
		for sid, session in zip(item["haystack_session_ids"], item["haystack_sessions"]):
			texts = []
			for i, turn in enumerate(session):
				text = sanitize(f"{turn['role']}: {turn.get('content', '')}")
				if not text:
					continue
				labels.add(text, ("s", sid))
				labels.add(text, ("t", sid, i))
				if turn.get("has_answer"):
					gold_turns.append(("t", sid, i))
				texts.append(text)
			if texts:
				status, _ = ingest_session(project, texts, tmp, str(sid))
				counts[f"ingest_{status}"] += 1
		counts["label_collisions"] += labels.collisions

		ranked, secs = ranked_keys(project, item["question"], labels)
		latencies.append(secs)
		gold_sessions = [("s", sid) for sid in item.get("answer_session_ids") or []]
		by_session = score.question_result(gold_sessions, ranked) if gold_sessions else None
		by_turn = score.question_result(gold_turns, ranked) if gold_turns else None
		project.kill_all()
	return by_session, by_turn


def main():
	parser = argparser(__doc__)
	parser.add_argument("--data", type=Path, default=DATA_DIR / "longmemeval_s")
	parser.add_argument("--limit", type=int, default=100)
	parser.add_argument("--full", action="store_true")
	args = parse_args(parser)
	if not args.data.exists():
		raise SystemExit(f"{args.data} missing — run `just eval-fetch` first")

	items = json.loads(args.data.read_text())
	if not args.full and args.limit < len(items):
		print(
			f"LIMIT: seeded sample of {args.limit}/{len(items)} questions "
			f"(seed {SAMPLE_SEED}) — pass --full for the whole set"
		)
		items = random.Random(SAMPLE_SEED).sample(items, args.limit)

	embed, llm_url, closer = open_endpoints(args)
	kern_bin = build_kern()
	counts = defaultdict(int)
	latencies = []
	session_scored = []
	turn_scored = []
	types = defaultdict(list)
	try:
		for i, item in enumerate(items):
			by_session, by_turn = run_question(
				kern_bin, item, args, embed, llm_url, counts, latencies
			)
			if by_session is None:
				counts["excluded_no_evidence"] += 1
			else:
				session_scored.append(by_session)
				types[item.get("question_type", "?")].append(by_session)
			if by_turn is not None:
				turn_scored.append(by_turn)
			print(f"question {i + 1}/{len(items)}: {len(session_scored)} scored")
	finally:
		closer()

	report = base_report(args, f"LongMemEval-S ({args.data.name})")
	report["sampled"] = None if args.full else {"limit": args.limit, "seed": SAMPLE_SEED}
	report["counts"] = dict(counts)
	report["session_granularity"] = score.metrics(session_scored, args.k)
	report["turn_granularity"] = score.metrics(turn_scored, args.k)
	report["by_question_type"] = {
		t: score.metrics(qs, args.k) for t, qs in sorted(types.items())
	}
	report["query_latency_secs"] = {
		"note": "cold-process CLI wall clock: spawn + graph load + embed + retrieve",
		"p50": score.percentile(latencies, 50),
		"p95": score.percentile(latencies, 95),
	}

	path = write_report(args.report_dir, "longmemeval", report)
	print("session granularity:", json.dumps(report["session_granularity"], indent=1))
	print("turn granularity:   ", json.dumps(report["turn_granularity"], indent=1))
	print(f"report: {path}")


if __name__ == "__main__":
	main()
