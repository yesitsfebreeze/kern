"""Rank arithmetic for the retrieval-only benchmarks. Pure functions, no I/O.

A scored question is `{"gold_n": int, "ranks": [int, ...]}` — how many gold
items existed and the 1-based ranks at which any of them were retrieved.
Multi-evidence questions make "recall@k" ambiguous, so both readings are
computed; any quoted number must say which it is.
"""

import math


def question_result(gold_keys, ranked_key_sets):
	"""Ranks (1-based, sorted) at which each gold key first appears."""
	ranks = []
	for gold in gold_keys:
		for i, keys in enumerate(ranked_key_sets):
			if gold in keys:
				ranks.append(i + 1)
				break
	return {"gold_n": len(gold_keys), "ranks": sorted(ranks)}


def recall_any_at_k(questions, k):
	"""Fraction of questions with at least one gold item in the top k."""
	if not questions:
		return 0.0
	return sum(1 for q in questions if q["ranks"] and q["ranks"][0] <= k) / len(questions)


def recall_all_at_k(questions, k):
	"""Fraction of questions with every gold item in the top k."""
	if not questions:
		return 0.0
	return sum(
		1
		for q in questions
		if len(q["ranks"]) == q["gold_n"] and q["ranks"][-1] <= k
	) / len(questions)


def mrr(questions):
	"""Mean reciprocal rank of the best-ranked gold item; 0 when none found."""
	if not questions:
		return 0.0
	return sum(1.0 / q["ranks"][0] for q in questions if q["ranks"]) / len(questions)


def ndcg_at_k(questions, k):
	"""Binary-relevance NDCG@k against the ideal of all gold ranked first."""
	if not questions:
		return 0.0
	total = 0.0
	for q in questions:
		dcg = sum(1.0 / math.log2(r + 1) for r in q["ranks"] if r <= k)
		idcg = sum(1.0 / math.log2(i + 1) for i in range(1, min(q["gold_n"], k) + 1))
		total += dcg / idcg if idcg else 0.0
	return total / len(questions)


def percentile(values, p):
	"""Linear-interpolation percentile; p in [0, 100]."""
	if not values:
		return 0.0
	xs = sorted(values)
	pos = (len(xs) - 1) * p / 100.0
	lo = int(pos)
	hi = min(lo + 1, len(xs) - 1)
	return xs[lo] + (xs[hi] - xs[lo]) * (pos - lo)


def metrics(questions, k):
	return {
		"questions": len(questions),
		"recall_any@1": recall_any_at_k(questions, 1),
		"recall_any@5": recall_any_at_k(questions, 5),
		f"recall_any@{k}": recall_any_at_k(questions, k),
		f"recall_all@{k}": recall_all_at_k(questions, k),
		"mrr": mrr(questions),
		f"ndcg@{k}": ndcg_at_k(questions, k),
	}
