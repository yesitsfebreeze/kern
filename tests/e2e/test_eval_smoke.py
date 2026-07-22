"""End-to-end plumbing smoke for the LoCoMo runner.

A two-session synthetic conversation in LoCoMo's shape runs through
`run_locomo.py --fake-llm`: real ingest, real query, real report — fake
embedder, so the numbers mean nothing and the report must say so itself.
"""

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "tests" / "e2e" / "eval" / "run_locomo.py"

SAMPLE = [
	{
		"sample_id": "smoke-1",
		"conversation": {
			"speaker_a": "Ada",
			"speaker_b": "Ben",
			"session_1": [
				{
					"speaker": "Ada",
					"dia_id": "D1:1",
					"text": "I moved my pottery studio into the old boathouse",
				},
				{
					"speaker": "Ben",
					"dia_id": "D1:2",
					"text": "The marathon training plan starts on the first of May",
				},
			],
			"session_1_date_time": "1 May 2026",
			"session_2": [
				{
					"speaker": "Ada",
					"dia_id": "D2:1",
					"text": "The kiln in the boathouse takes eight hours to fire",
				},
			],
			"session_2_date_time": "2 May 2026",
		},
		"qa": [
			{
				"question": "where is the pottery studio",
				"answer": "the old boathouse",
				"evidence": ["D1:1"],
				"category": 4,
			},
			{
				"question": "how long does the kiln take to fire",
				"answer": "eight hours",
				"evidence": ["D2:1"],
				"category": 4,
			},
			{
				"question": "did Ben ever win a chess tournament",
				"adversarial_answer": "no information",
				"evidence": [],
				"category": 5,
			},
		],
	}
]


def test_locomo_runner_end_to_end_on_the_fake_embedder(kern_bin, tmp_path):
	data = tmp_path / "locomo-smoke.json"
	data.write_text(json.dumps(SAMPLE))
	reports = tmp_path / "reports"

	out = subprocess.run(
		[
			sys.executable,
			str(RUNNER),
			"--fake-llm",
			"--data",
			str(data),
			"--report-dir",
			str(reports),
		],
		cwd=ROOT,
		env=os.environ | {"KERN_BIN": str(kern_bin)},
		capture_output=True,
		text=True,
		timeout=300,
	)
	assert out.returncode == 0, f"runner failed:\nout={out.stdout}\nerr={out.stderr}"

	written = list(reports.glob("locomo-*.json"))
	assert len(written) == 1, f"expected one report, got {written}"
	report = json.loads(written[0].read_text())

	assert "MEANINGLESS" in report, "a fake-llm report must disclaim itself"
	assert report["counts"]["excluded_adversarial"] == 1
	assert report["overall"]["questions"] == 2
	# Bag-of-words overlap between probe and fact is strong here by
	# construction; both probes must at least retrieve their turn somewhere.
	assert report["overall"]["recall_any@10"] > 0.0
	assert report["query_latency_secs"]["p95"] > 0.0
