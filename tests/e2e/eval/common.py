"""Shared plumbing for the retrieval-only benchmark runners.

Reuses the e2e harness (KernProject drives the built binary; ranking.hits
parses its stdout) so the eval exercises exactly the CLI path users hit —
no bespoke ingest or query code that could drift from the product.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from harness import KernProject  # noqa: E402
from ranking import hits  # noqa: E402

DATA_DIR = REPO / "eval"
REPORT_DIR = DATA_DIR / "reports"

# kern's own default embedder (src/config/embed.rs) — the pinned model every
# real run reports. A different --embed-model changes what the number means.
DEFAULT_EMBED_URL = "http://localhost:11434"
DEFAULT_EMBED_MODEL = "qwen3-embedding:0.6b"

# Mirrors base::util::truncate(_, 120) — what `kern query` prints per hit.
PRINT_CAP = 120


def build_kern():
	# KERN_BIN skips the release build — the e2e smoke test hands over its
	# already-built debug binary; real runs measure release.
	if os.environ.get("KERN_BIN"):
		return Path(os.environ["KERN_BIN"])
	subprocess.run(["cargo", "build", "--release", "--bin", "kern"], cwd=REPO, check=True)
	meta = subprocess.run(
		["cargo", "metadata", "--format-version", "1", "--no-deps"],
		cwd=REPO,
		capture_output=True,
		text=True,
		check=True,
	)
	return Path(json.loads(meta.stdout)["target_directory"]) / "release" / "kern"


def add_common_args(parser):
	parser.add_argument("--embed-url", default=DEFAULT_EMBED_URL)
	parser.add_argument("--embed-model", default=DEFAULT_EMBED_MODEL)
	parser.add_argument("--k", type=int, default=10)
	parser.add_argument(
		"--fake-llm",
		action="store_true",
		help="plumbing smoke test against the e2e bag-of-words embedder — "
		"the resulting numbers are MEANINGLESS and the report says so",
	)
	parser.add_argument("--report-dir", type=Path, default=REPORT_DIR)


def open_endpoints(args):
	"""(embed_pair, llm_url, closer). With --fake-llm both point at the fake."""
	if args.fake_llm:
		from fake_llm import FakeLlm

		fake = FakeLlm()
		return (fake.url, "fake-embed"), fake.url, fake.close
	# The reason endpoint is unused on the direct ingest + query path; it points
	# at the embed host only so the config validates.
	return (args.embed_url, args.embed_model), args.embed_url, lambda: None


def make_project(kern_bin, tmp_path, embed, llm_url, k):
	p = KernProject(kern_bin, tmp_path, llm_url)
	p.write_config(
		embed=embed,
		# The default preset delivers 25; only widen when k asks past it, so a
		# default run measures the default config.
		max_deliver_results=k if k > 25 else None,
	)
	return p


def sanitize(text):
	"""One turn -> one single-line stored Document.

	Newlines are flattened to spaces for two reasons: kern's heuristic split
	(src/ingest/split.rs::paragraph_split) chunks on blank lines, so a
	multi-paragraph turn would shatter into chunks the label map no longer
	matches; and `kern query` prints one hit per line, so an embedded newline
	would truncate the printed label mid-text.
	"""
	return re.sub(r"\s*\n\s*", " ", text.replace("\r", "")).strip()


def label(text):
	"""What `kern query` will print for this stored text (util::truncate)."""
	return text[:PRINT_CAP] + "..." if len(text) > PRINT_CAP else text


def ingest_session(project, turn_texts, tmp_path, name):
	"""Batch one session through a single `kern ingest --file` call.

	Turns joined with a blank line ride kern's own paragraph split: one
	process, one graph load, one flush — ~15x fewer process spawns than
	per-turn ingest, with identical stored text per turn.
	"""
	f = tmp_path / f"{name}.txt"
	f.write_text("\n\n".join(turn_texts))
	stdout, stderr = project.run("ingest", "--file", str(f), timeout=600)
	m = re.search(r"status=(\w+)", stdout)
	status = m.group(1) if m else "missing"
	if status not in ("committed", "deduped", "partial"):
		raise RuntimeError(f"ingest {name} failed: out={stdout} err={stderr}")
	return status, stderr


FULL_TEXT = re.compile(r"^Text:\s+(.*)$", re.M)


def resolve_full_text(project, short_id):
	"""Full stored text for one hit, for the rare truncation-collision case."""
	stdout, _ = project.run("get", short_id)
	m = FULL_TEXT.search(stdout)
	return m.group(1) if m else None


def ranked_keys(project, question, label_map):
	"""Query once; map printed hits back to provenance keys.

	Returns (ordered list of key-sets, query wall seconds). Each element is
	the set of provenance keys the hit's label could name; ambiguous labels
	(two different full texts sharing a 120-char prefix) are resolved through
	`kern get`, which prints the full text.
	"""
	t0 = time.monotonic()
	stdout, _ = project.run("query", question)
	secs = time.monotonic() - t0
	out = []
	for hit in hits(stdout):
		entry = label_map.get(hit.text)
		if entry is None:
			out.append(set())
		elif len(entry) == 1:
			out.append(next(iter(entry.values())))
		else:
			full = resolve_full_text(project, hit.short_id)
			out.append(entry.get(full, set().union(*entry.values())))
	return out, secs


class LabelMap:
	"""label -> {full_text -> set(keys)}; collision-aware both ways.

	The same full text ingested twice dedups to one entity, so its key set
	holds every provenance that text carries. Two different full texts under
	one label is the truncation collision `resolve_full_text` untangles.
	"""

	def __init__(self):
		self.by_label = {}
		self.collisions = 0

	def add(self, text, key):
		entry = self.by_label.setdefault(label(text), {})
		if text not in entry and entry:
			self.collisions += 1
		entry.setdefault(text, set()).add(key)

	def get(self, printed, default=None):
		return self.by_label.get(printed, default)


def git_head():
	return subprocess.run(
		["git", "rev-parse", "--short", "HEAD"],
		cwd=REPO,
		capture_output=True,
		text=True,
		check=True,
	).stdout.strip()


def write_report(report_dir, name, report):
	report_dir.mkdir(parents=True, exist_ok=True)
	stamp = time.strftime("%Y%m%d-%H%M%S")
	path = report_dir / f"{name}-{stamp}.json"
	path.write_text(json.dumps(report, indent=1))
	return path


def base_report(args, dataset):
	report = {
		"dataset": dataset,
		"protocol": "retrieval-only: direct-path `kern ingest` per turn, "
		"`kern query` per question, rank arithmetic over CLI stdout; no LLM "
		"anywhere in ingest, retrieval, or scoring",
		"comparable_to": "pure-retrieval recall@k numbers only — NOT to "
		"LLM-judged scores (Zep/Mem0/Letta LoCoMo or LongMemEval J-scores)",
		"embed_model": "fake-embed (bag of words)" if args.fake_llm else args.embed_model,
		"k": args.k,
		"commit": git_head(),
	}
	if args.fake_llm:
		report["MEANINGLESS"] = (
			"--fake-llm run: bag-of-words embedder, numbers test plumbing only"
		)
	return report


def parse_args(parser):
	args = parser.parse_args()
	if not args.fake_llm:
		print(f"embed endpoint {args.embed_url} model {args.embed_model}")
	return args


def argparser(description):
	parser = argparse.ArgumentParser(description=description)
	add_common_args(parser)
	return parser
