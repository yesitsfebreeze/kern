"""ground.json stays internally consistent — every cited turn really says
what the question claims it says.

The dataset cites evidence by (session_id, 1-based turn index, anchor
substring). An index slip after an edit would silently misgrade every run,
so the anchor is load-bearing: it must appear verbatim in the cited turn.
Pure data validation, no kern binary, no embedder — runs in CI.
"""

import json
from pathlib import Path

GROUND = Path(__file__).parent / "eval" / "ground.json"


def load():
	return json.loads(GROUND.read_text())


def test_every_anchor_appears_in_its_cited_turn():
	d = load()
	sessions = {s["id"]: s["turns"] for s in d["sessions"]}
	for q in d["questions"]:
		assert q["evidence"], f"question without evidence: {q['q']}"
		for sid, turn, anchor in q["evidence"]:
			turns = sessions[sid]
			assert 1 <= turn <= len(turns), f"turn {turn} out of range in {sid}: {q['q']}"
			assert anchor in turns[turn - 1], (
				f"anchor not in {sid} turn {turn}: {anchor!r} — evidence index slipped"
			)


def test_turns_are_single_paragraph_and_nonempty():
	# One turn = one blank-line-separated unit; an embedded blank line would
	# shift every later index for both paragraph_split and distill's
	# split_turns, so it is a data error here, not a runner concern.
	d = load()
	for s in d["sessions"]:
		assert s["turns"], f"empty session {s['id']}"
		for i, t in enumerate(s["turns"]):
			assert t.strip(), f"blank turn {s['id']}:{i + 1}"
			assert "\n" not in t, f"embedded newline in {s['id']}:{i + 1}"


def test_categories_are_the_known_four():
	d = load()
	allowed = {"single-hop", "multi-hop", "temporal", "update"}
	for q in d["questions"]:
		assert q["category"] in allowed, f"unknown category {q['category']!r}: {q['q']}"


def test_session_ids_are_unique_and_referenced():
	d = load()
	ids = [s["id"] for s in d["sessions"]]
	assert len(ids) == len(set(ids)), "duplicate session id"
	cited = {sid for q in d["questions"] for sid, _, _ in q["evidence"]}
	assert cited <= set(ids), f"evidence cites unknown session: {cited - set(ids)}"
