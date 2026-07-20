"""Answer-retrieval e2e: ingest facts through the real binary, then assert
search, query and the answer path surface the right one. LLM legs are served
by the deterministic fake in fake_llm.py, so ranking is real cosine ranking."""

FACTS = [
	"Ada keeps her bicycle in the garden shed behind the house",
	"The deploy pipeline runs on Jenkins every night at two",
	"Marrow the cat refuses to eat anything except salmon",
]


def ingest_facts(project):
	for fact in FACTS:
		stdout, stderr = project.run("ingest", fact)
		assert "status=committed" in stdout, f"ingest failed: out={stdout} err={stderr}"


def hits(stdout):
	return [line for line in stdout.splitlines() if line[:1].isdigit()]


def test_query_on_an_empty_graph_says_no_results(project):
	stdout, _ = project.run("query", "anything at all")
	assert "no results" in stdout


def test_search_ranks_the_matching_fact_first(project):
	ingest_facts(project)
	stdout, stderr = project.run("search", "where does ada store her bicycle")
	ranked = hits(stdout)
	assert ranked, f"no hits: out={stdout} err={stderr}"
	assert "bicycle" in ranked[0], f"wrong top hit: {ranked[0]}"


def test_query_recalls_each_fact_from_a_paraphrase(project):
	ingest_facts(project)
	for probe, marker in [
		("where does ada store her bicycle", "bicycle"),
		("when does the deploy pipeline run", "pipeline"),
		("what does marrow the cat eat", "salmon"),
	]:
		stdout, stderr = project.run("query", probe)
		ranked = hits(stdout)
		assert ranked, f"no hits for {probe!r}: out={stdout} err={stderr}"
		assert any(marker in line for line in ranked), (
			f"fact absent from results for {probe!r}: {ranked}"
		)


def test_query_ranks_the_matching_fact_first(project):
	# Regression guard for the hybrid-fusion scale mismatch: RRF's reciprocal-rank
	# seed scores let expanded neighbours outscore every seed, inverting ranking.
	ingest_facts(project)
	for probe, marker in [
		("where does ada store her bicycle", "bicycle"),
		("when does the deploy pipeline run", "pipeline"),
		("what does marrow the cat eat", "salmon"),
	]:
		stdout, _ = project.run("query", probe)
		ranked = hits(stdout)
		assert ranked and marker in ranked[0], (
			f"wrong top hit for {probe!r}: {ranked}"
		)


def test_answer_prompt_carries_the_retrieved_fact(project):
	# The fake answer model echoes its prompt, so the retrieved context
	# reaching the answer leg is directly observable in stdout.
	ingest_facts(project)
	stdout, stderr = project.run("query", "where does ada store her bicycle", "--answer")
	assert "--- Answer ---" in stdout, f"no answer section: out={stdout} err={stderr}"
	answer = stdout.split("--- Answer ---", 1)[1]
	assert "garden shed" in answer, f"retrieved fact missing from answer prompt: {answer}"
	assert "where does ada store her bicycle" in answer.lower()
