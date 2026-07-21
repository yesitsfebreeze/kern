"""Corpus-independent retrieval invariants — properties that must hold whatever
was ingested. Each test names the `docs/oracle/VISION.md` "The test" criterion it
defends. Scoring is pure rank arithmetic over the binary's own stdout; the LLM
legs are the deterministic fake in fake_llm.py, so no model judges anything.

EMPTY GRAPH ("query on an empty graph returns no results, never a spurious
hit") is already defended by test_retrieval.py::test_query_on_an_empty_graph_
says_no_results and is deliberately not duplicated here.

Several criteria are unreachable from the shipped CLI and stand as skipped tests
naming the missing surface. They are owed, not passing.
"""

import pytest

from ranking import connections, full_id, hits, ingest_all, link

CORPUS = [
	"Ada keeps her bicycle in the garden shed behind the house",
	"Quill the parrot mimics the doorbell every afternoon",
	"The deploy pipeline runs on Jenkins every night at two",
	"Marrow the cat refuses to eat anything except salmon",
	"Postgres holds the invoices table on the analytics replica",
	"Helen swims at the lido on Tuesday mornings",
	"The night train to Vienna leaves platform nine",
	"Sourdough starter doubles in eight hours at room temperature",
	"Bees prefer lavender over roses in the courtyard",
	"Solar panels on the roof produce four kilowatts at noon",
	"The library fine is ten cents per overdue day",
	"Kubernetes ingress terminates TLS at the edge proxy",
]

# Eight A->B pairs whose members share no content words, so nothing but the
# reason edge can connect them. Mirrors the n=8 smoke recorded in
# docs/oracle/ROADMAP.md item 1.
LINKED_PAIRS = [
	("Ada keeps her bicycle in the garden shed behind the house", "Quill the parrot mimics the doorbell every afternoon"),
	("The deploy pipeline runs on Jenkins every night at two", "Bees prefer lavender over roses in the courtyard"),
	("Postgres holds the invoices table on the analytics replica", "The night train to Vienna leaves platform nine"),
	("Helen swims at the lido on Tuesday mornings", "Kubernetes ingress terminates TLS at the edge proxy"),
	("Sourdough starter doubles in eight hours at room temperature", "Mortgage rates were renegotiated in October"),
	("The library fine is ten cents per overdue day", "Solar panels on the roof produce four kilowatts at noon"),
	("Marrow the cat refuses to eat anything except salmon", "The staging cluster lives in Frankfurt"),
	("Tomatoes need six hours of direct sunlight", "The router firmware update broke IPv6 for a week"),
]

PAIR_FILLER = [
	"The office coffee machine was replaced in March",
	"Rust builds are cached in the shared sccache bucket",
	"Invoices older than seven years get archived to cold storage",
	"The fire drill happens on the first Monday of each quarter",
	"The quarterly board deck is due the Friday before the meeting",
	"The dog walker comes at eleven on weekdays",
	"Whales migrate past the cape in November",
	"The espresso grinder needs a burr change annually",
]

REACHABLE_WITHIN = 5


def rank_of(project, probe, text):
	stdout, _ = project.run("query", probe)
	for hit in hits(stdout):
		if hit.text == text:
			return hit.rank, hit.score, stdout
	return None, None, stdout


def test_every_ingested_fact_is_its_own_top_hit(project):
	"""A graph, not a bag — recall must at minimum return the exact thing it was
	given. If self-recall breaks, no other retrieval number means anything."""
	ingest_all(project, CORPUS)
	misses = []
	for fact in CORPUS:
		rank, _, stdout = rank_of(project, fact, fact)
		if rank != 1:
			misses.append((fact, rank, hits(stdout)[:3]))
	assert not misses, f"{len(misses)}/{len(CORPUS)} facts are not their own top hit: {misses}"


def test_identical_text_ingested_twice_is_one_node(project):
	"""'Ids are content hashes, so identical content is the same node
	everywhere.' Re-ingest reports status=committed either way, so the node
	count in `health` is the only observable — assert on that, not the string."""
	fact = "Ada keeps her bicycle in the garden shed behind the house"
	ingest_all(project, [fact, fact])
	stdout, stderr = project.run("health")
	thoughts = [ln for ln in stdout.splitlines() if ln.startswith("thoughts:")]
	assert thoughts, f"health printed no thoughts line: out={stdout} err={stderr}"
	assert thoughts[0].split()[1] == "1", f"content addressing broke: {thoughts[0]}"


def test_a_reason_edge_makes_its_neighbour_reachable(project):
	"""'A graph, not a bag — recall can walk them.' Probing with A's own text puts
	A at rank 1; B shares no content words with the probe, so the reason edge
	A->B is the only thing that can surface B. THE priority invariant.

	Closed by ROADMAP item 86 (2026-07-21): bounded source-weighted traversal
	credit — every examined edge credits its far endpoint with
	source_score * edge_evidence, summed, capped, and clamped below the
	strongest voucher's own walk score, so a walk pays without ever outranking
	a direct match. Deliberate links carry asserted confidence as their score
	(user 1.0 / agent 0.95), not endpoint cosine, which is what gives an edge
	between dissimilar texts enough evidence to lift its far end."""
	ingest_all(project, [t for pair in LINKED_PAIRS for t in pair] + PAIR_FILLER)
	for a, b in LINKED_PAIRS:
		link(project, a, b, "linked for the multi-hop invariant")

	unreachable = []
	for a, b in LINKED_PAIRS:
		rank, _, stdout = rank_of(project, a, b)
		if rank is None or rank > REACHABLE_WITHIN:
			unreachable.append((b, rank, "in-chain" if b[:40] in connections(stdout) else "no-chain"))
	assert not unreachable, (
		f"{len(unreachable)}/{len(LINKED_PAIRS)} linked neighbours are outside the top "
		f"{REACHABLE_WITHIN}: {unreachable}"
	)


@pytest.mark.skip(
	reason="OWED: no `supersede`/`correct` subcommand, and no flag on ingest or link "
	"reaching one. accept::supersede needs an external_id collision, which cmd_ingest "
	"cannot produce (external_id is a hash of the exact text); "
	"accept::supersede_by_contradiction needs the defer_contradiction hook, which "
	"cmd_ingest passes as None. `get` also never prints EntityStatus, so a superseded "
	"entity is not even distinguishable in CLI output."
)
def test_a_correction_outranks_the_claim_it_superseded(project):
	"""'Superseded, never deleted.'"""


@pytest.mark.skip(
	reason="OWED: no --as-of / --valid-at / --include-history on `kern query`. "
	"Bi-temporal point query exists only on the MCP `query` tool "
	"(src/mcp/tools_query.rs), unreachable from the subcommand surface this suite "
	"shells out to."
)
def test_a_point_in_time_query_returns_the_pre_correction_claim(project):
	"""'An updated or contradicted claim becomes bi-temporal history queryable
	`as_of` a past instant.'"""


def test_degrading_an_entity_punishes_its_delivered_score(project):
	"""'Retrieval learns from use — degrade down-weights bad paths; a bad result
	never keeps ranking unpunished.'

	Entity-scoped only. An earlier version linked A->B here and claimed the edge
	was what put B in the ranking; it was not — at the time the multi-hop walk
	changed no outcome (ROADMAP item 86, since closed). The link stays out so
	this test keeps measuring the entity-scoped half in isolation."""
	ingest_all(project, CORPUS[:6])
	a, b = CORPUS[0], CORPUS[1]

	_, before, _ = rank_of(project, a, b)
	assert before is not None, f"{b!r} absent from results for {a!r} before degrade"

	stdout, stderr = project.run("degrade", full_id(project, b))
	assert stdout.startswith("degraded "), f"degrade failed: out={stdout} err={stderr}"

	_, after, _ = rank_of(project, a, b)
	assert after is not None and after < before, (
		f"degrade left the score unpunished: {before} -> {after}"
	)


@pytest.mark.skip(
	reason="OWED: `kern degrade <ID>` takes one entity and decays every edge incident "
	"on it (src/commands/graph_ops.rs::degrade_entity_reasons). There is no CLI way to "
	"name a path or chain, so 'degrade THIS retrieval path' is inexpressible."
)
def test_degrading_one_path_leaves_other_paths_to_the_same_entity_intact(project):
	"""'degrade down-weights bad paths' — the path-scoped half."""


def test_a_fact_survives_gc_and_refuses_to_be_forgotten(project):
	"""'The hot graph stays bounded ... Facts are never auto-forgotten.'
	Everything the CLI ingests comes back Kind: Fact, so both halves of
	durability are observable: gc must not drop it, forget must refuse it."""
	ingest_all(project, CORPUS[:6])
	fact = CORPUS[0]
	before, _ = project.run("health")

	stdout, stderr = project.run("gc")
	assert "gc:" in stdout, f"gc did not run: out={stdout} err={stderr}"

	after, _ = project.run("health")
	counts = lambda out: [ln for ln in out.splitlines() if ln.startswith("thoughts:")]
	assert counts(after) == counts(before), f"gc changed the thought count: {counts(before)} -> {counts(after)}"

	rank, _, stdout = rank_of(project, fact, fact)
	assert rank == 1, f"fact not recallable after gc: {stdout}"

	stdout, stderr = project.run("forget", full_id(project, fact))
	assert "cannot forget a fact" in stderr, (
		f"a Fact was forgettable: out={stdout} err={stderr}"
	)


@pytest.mark.skip(
	reason="OWED: the evictable half needs a non-Fact thought, and every statement "
	"ingested through the CLI comes back `Kind: Fact`. No ingest flag selects a kind, "
	"and `forget` rejects Facts outright, so 'ordinary thoughts are evictable' has no "
	"CLI construction."
)
def test_an_ordinary_thought_is_evictable(project):
	"""'The hot graph stays bounded' — the evictable half."""
