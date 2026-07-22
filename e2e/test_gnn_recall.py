"""The same recall number as `test_recall.py`, but measured on a graph the GNN
has actually propagated.

`test_recall.py` runs entirely through the CLI, which has no tick loop at all, so
`do_gnn_propagate` is never even called there. A daemon does tick — and still
skips the GNN, because `DEFAULT_MIN_THOUGHTS` is 128 and the corpus is 36. Both
halves are why "recall unchanged" has never meant anything about a GNN change
(ROADMAP item 97).

Two assertions, and the first is the point of the file: the propagation must be
observed to have RUN, on a stated number of nodes, before any number is scored.
A recall floor alone would re-create exactly the gate this file exists to
replace — green, and about code that never executed.

What it does NOT measure: production scale. `min_thoughts` is lowered here so
the 36-fact corpus trains, and a 36-node graph is not the regime a real
propagation runs in. See ROADMAP item 97 for why the corpus was not grown
instead.
"""

import re
import sys
import time

import pytest

from ranking import hits, ingest_all
from test_recall import FACTS, PROBES

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="unix sockets only")

# e2e-only, and deliberately far below the shipped 128 (src/gnn/propagate.rs):
# the point is that the propagation runs, and a floor just under the corpus size
# would go quietly vacuous again the day the corpus or the clustering moves.
GNN_MIN_THOUGHTS = 4

# RECORDED BASELINE, NOT A TARGET — same standard as `test_recall.py`, and a
# separate set of numbers because this is a different graph: the seed index is
# fused 0.6/0.4 with the propagated one (src/base/search.rs), so these are not
# comparable to the CLI corpus floors and neither set may be copied to the other.
# Propagation is stochastic (unseeded weight init and negative-edge sampling in
# src/gnn/propagate.rs), so these are set below the worst of a multi-run sample —
# see the CHANGELOG entry for the run count and the spread.
RECALL_AT_1_FLOOR = 0.85  # 8 runs, 2026-07-22: 0.8889 - 0.9306
RECALL_AT_5_FLOOR = 0.93  # 0.9583 - 0.9722
MRR_FLOOR = 0.88  # 0.9219 - 0.9508

# `min_thoughts` bounds this from below, but only the propagation's own report
# says how many nodes it really covered.
MIN_PROPAGATED_NODES = 30

_APPLIED = re.compile(r"learned propagation applied.*\bnodes=(\d+)")
# tracing's fmt layer colours field names when stderr is a file too, so the
# `nodes=` the regex is looking for arrives wrapped in SGR escapes.
_ANSI = re.compile(r"\x1b\[[0-9;]*m")


def _propagated_nodes(log, secs=60):
	"""Every `nodes=` the daemon has reported, once at least one has arrived."""
	deadline = time.monotonic() + secs
	while time.monotonic() < deadline:
		text = _ANSI.sub("", log.read_text() if log.exists() else "")
		counts = [int(n) for n in _APPLIED.findall(text)]
		if counts:
			return counts
		time.sleep(0.2)
	raise AssertionError(
		f"no propagation in {secs}s — the GNN never ran, so nothing below this "
		f"line would have been a measurement of it. daemon log:\n"
		f"{log.read_text() if log.exists() else '<no daemon log>'}"
	)


def test_recall_over_a_graph_the_gnn_has_propagated(project):
	project.write_config(gnn_min_thoughts=GNN_MIN_THOUGHTS, tick_interval_secs=0)
	ingest_all(project, FACTS)

	log = project.cwd / "daemon.log"
	# The propagation's only trace outside the graph. `interval_secs = 0` leaves
	# just the one maintenance pass a daemon enqueues at boot, so the embeddings
	# are not still moving under the 72 probes below.
	project.env["RUST_LOG"] = "kern.gnn=info"
	with log.open("wb") as sink:
		project.start_daemon(stderr=sink)
		nodes = _propagated_nodes(log)

	ranks = []
	for probe, gold in PROBES:
		stdout, _ = project.run("query", probe)
		ranks.append(next((h.rank for h in hits(stdout) if h.text == gold), None))

	n = len(ranks)
	at1 = sum(1 for r in ranks if r == 1) / n
	at5 = sum(1 for r in ranks if r is not None and r <= 5) / n
	mrr = sum(1.0 / r for r in ranks if r is not None) / n

	print()
	print(f"kern-gnn propagations    {len(nodes)}, nodes={nodes}")
	print(f"kern-gnn recall@1        {at1:.4f}  (floor {RECALL_AT_1_FLOOR:.4f})")
	print(f"kern-gnn recall@5        {at5:.4f}  (floor {RECALL_AT_5_FLOOR:.4f})")
	print(f"kern-gnn MRR             {mrr:.4f}  (floor {MRR_FLOOR:.4f})")

	assert max(nodes) >= MIN_PROPAGATED_NODES, (
		f"propagation covered {max(nodes)} nodes, under {MIN_PROPAGATED_NODES}: "
		f"the floors below would be describing a stub graph, not the corpus"
	)
	assert at1 >= RECALL_AT_1_FLOOR, f"recall@1 regressed: {at1:.4f} < {RECALL_AT_1_FLOOR}"
	assert at5 >= RECALL_AT_5_FLOOR, f"recall@5 regressed: {at5:.4f} < {RECALL_AT_5_FLOOR}"
	assert mrr >= MRR_FLOOR, f"MRR regressed: {mrr:.4f} < {MRR_FLOOR}"
