"""Unit tests for the retrieval-only benchmark scorer (scripts/eval).

Pure rank arithmetic and label plumbing — fast, no kern binary, runs in CI.
The slow benchmark runners themselves are user-run (`just eval-locomo`).
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "eval"))

import score
from common import LabelMap, label, sanitize


def q(gold_n, ranks):
	return {"gold_n": gold_n, "ranks": sorted(ranks)}


def test_question_result_finds_first_rank_of_each_gold_key():
	ranked = [{"a"}, {"b", "x"}, set(), {"a"}, {"c"}]
	got = score.question_result(["a", "b", "missing"], ranked)
	assert got == {"gold_n": 3, "ranks": [1, 2]}, (
		"a at 1, b at 2, missing absent — and a's later duplicate at 4 ignored"
	)


def test_recall_any_vs_all_disagree_exactly_on_partial_multi_evidence():
	qs = [q(2, [1, 2]), q(2, [3]), q(1, [])]
	assert score.recall_any_at_k(qs, 5) == 2 / 3
	assert score.recall_all_at_k(qs, 5) == 1 / 3


def test_recall_all_respects_k_even_when_every_gold_was_found():
	qs = [q(2, [1, 7])]
	assert score.recall_all_at_k(qs, 5) == 0.0
	assert score.recall_all_at_k(qs, 10) == 1.0


def test_mrr_uses_best_rank_and_zeroes_the_unretrieved():
	qs = [q(1, [2]), q(1, [])]
	assert score.mrr(qs) == (0.5 + 0.0) / 2


def test_ndcg_is_one_for_perfect_ranking_and_zero_for_none():
	assert score.ndcg_at_k([q(2, [1, 2])], 10) == 1.0
	assert score.ndcg_at_k([q(2, [])], 10) == 0.0


def test_percentile_interpolates():
	assert score.percentile([1.0, 2.0, 3.0, 4.0], 50) == 2.5
	assert score.percentile([1.0], 95) == 1.0
	assert score.percentile([], 50) == 0.0


def test_metrics_reports_every_headline_number():
	m = score.metrics([q(1, [1])], 10)
	assert m["recall_any@1"] == 1.0
	assert m["recall_all@10"] == 1.0
	assert m["ndcg@10"] == 1.0
	assert m["questions"] == 1


def test_label_mirrors_kern_truncate_at_120_chars():
	short = "x" * 120
	assert label(short) == short, "at the cap: printed verbatim"
	assert label("x" * 121) == "x" * 120 + "...", "past the cap: cut + ellipsis"


def test_sanitize_flattens_every_newline_to_one_line():
	assert sanitize("a\r\n\n  b\nc") == "a b c"


def test_labelmap_counts_truncation_collisions_and_merges_dedup_keys():
	m = LabelMap()
	m.add("same text", "k1")
	m.add("same text", "k2")
	assert m.collisions == 0, "identical full text is dedup, not collision"
	assert m.get("same text") == {"same text": {"k1", "k2"}}

	a = "y" * 121
	b = "y" * 120 + "z"
	m.add(a, "ka")
	m.add(b, "kb")
	assert m.collisions == 1, "two full texts under one printed label"
	assert set(m.get(label(a))) == {a, b}
