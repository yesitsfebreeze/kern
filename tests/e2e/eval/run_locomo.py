"""LoCoMo-10, retrieval-only.

Per conversation: a fresh kern project ingests every dialog turn verbatim
(direct path, no LLM), then every evidence-labelled question runs `kern query`
and the evidence turns' ranks are scored. Category 5 (adversarial) has no
evidence turns to rank, so it is excluded and counted — the Zep/Mem0 84->58
dispute lived in exactly that category.
"""

import json
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

CATEGORY = {1: "multi-hop", 2: "temporal", 3: "open-domain", 4: "single-hop", 5: "adversarial"}


def session_keys(conversation):
	keys = []
	for key in conversation:
		if key.startswith("session_") and isinstance(conversation[key], list):
			keys.append(key)
	return sorted(keys, key=lambda s: int(s.split("_")[1]))


def turn_text(turn):
	text = (turn.get("text") or "").strip()
	caption = (turn.get("blip_caption") or "").strip()
	if caption:
		text = f"{text} [shares photo: {caption}]".strip()
	return sanitize(f"{turn['speaker']}: {text}")


def evidence_ids(qa):
	flat = []
	for e in qa.get("evidence") or []:
		if isinstance(e, list):
			flat.extend(str(x) for x in e)
		else:
			flat.append(str(e))
	return flat


def run_conversation(kern_bin, sample, args, embed, llm_url, counts, latencies):
	labels = LabelMap()
	per_qa = []
	with tempfile.TemporaryDirectory(prefix="kern-locomo-") as tmp:
		tmp = Path(tmp)
		project = make_project(kern_bin, tmp, embed, llm_url, args.k)
		for skey in session_keys(sample["conversation"]):
			texts = []
			for turn in sample["conversation"][skey]:
				text = turn_text(turn)
				if not text:
					continue
				labels.add(text, turn["dia_id"])
				texts.append(text)
			if texts:
				status, _ = ingest_session(project, texts, tmp, skey)
				counts[f"ingest_{status}"] += 1
		counts["label_collisions"] += labels.collisions

		for qa in sample["qa"]:
			cat = qa.get("category")
			if cat == 5:
				counts["excluded_adversarial"] += 1
				continue
			gold = evidence_ids(qa)
			if not gold:
				counts["excluded_no_evidence"] += 1
				continue
			ranked, secs = ranked_keys(project, qa["question"], labels)
			latencies.append(secs)
			result = score.question_result(gold, ranked)
			result["category"] = cat
			per_qa.append(result)
		project.kill_all()
	return per_qa


def main():
	parser = argparser(__doc__)
	parser.add_argument("--data", type=Path, default=DATA_DIR / "locomo10.json")
	parser.add_argument(
		"--conversations", type=int, default=0, help="first N only (0 = all 10)"
	)
	args = parse_args(parser)
	if not args.data.exists():
		raise SystemExit(f"{args.data} missing — run `just eval-fetch` first")

	samples = json.loads(args.data.read_text())
	if args.conversations:
		print(f"LIMIT: scoring {args.conversations} of {len(samples)} conversations")
		samples = samples[: args.conversations]

	embed, llm_url, closer = open_endpoints(args)
	kern_bin = build_kern()
	counts = defaultdict(int)
	latencies = []
	scored = []
	try:
		for i, sample in enumerate(samples):
			scored.extend(
				run_conversation(kern_bin, sample, args, embed, llm_url, counts, latencies)
			)
			print(f"conversation {i + 1}/{len(samples)}: {len(scored)} questions scored")
	finally:
		closer()

	report = base_report(args, f"LoCoMo-10 ({args.data.name})")
	report["counts"] = dict(counts)
	report["overall"] = score.metrics(scored, args.k)
	report["by_category"] = {
		f"{cat}-{CATEGORY.get(cat, '?')}": score.metrics(
			[q for q in scored if q["category"] == cat], args.k
		)
		for cat in sorted({q["category"] for q in scored})
	}
	report["query_latency_secs"] = {
		"note": "cold-process CLI wall clock: spawn + graph load + embed + retrieve",
		"p50": score.percentile(latencies, 50),
		"p95": score.percentile(latencies, 95),
	}

	path = write_report(args.report_dir, "locomo", report)
	print(json.dumps(report["overall"], indent=1))
	for name, m in report["by_category"].items():
		print(f"{name}: any@5 {m['recall_any@5']:.4f}  mrr {m['mrr']:.4f}  n={m['questions']}")
	print(f"report: {path}")


if __name__ == "__main__":
	main()
