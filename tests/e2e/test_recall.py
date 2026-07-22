"""The number: recall@1, recall@5 and MRR for `kern query` over an inline
corpus, with no LLM anywhere in the scoring loop.

Ground truth is written by the test — it ingests the facts, so it knows which
text is correct for each probe. Scoring is rank arithmetic over the binary's own
stdout. Embeddings come from fake_llm.py's feature-hashed bag of words, which is
deterministic, so the same corpus produces the same number on every machine and
every run.

What this measures is kern's retrieval stack — seeding, graph expansion, fusion,
ranking, delivery — over a fixed lexical signal. It does NOT measure semantic
embedding quality: the fake embedder has none, by design, because a real one
would make the number a property of the model rather than of kern. So this is a
regression detector. It cannot say kern is good; it can say kern got worse.
"""

from ranking import hits, ingest_all

# RECORDED BASELINE, NOT A TARGET. Measured on this corpus 2026-07-21 and
# rounded down. These floors exist to catch a regression, not to certify
# quality — kern claims no retrieval quality until ROADMAP item 1 is answered.
# Raising a floor is a deliberate act: do it only when a real improvement has
# been measured and you intend to defend the new level.
RECALL_AT_1_FLOOR = 0.90  # measured 0.9306 (2026-07-21, item 86 traversal credit)
RECALL_AT_5_FLOOR = 0.95  # measured 0.9722
MRR_FLOOR = 0.92  # measured 0.9471

# (fact, [paraphrase probes]) — several unrelated topics so every probe faces
# real distractors, not just the one right answer.
CORPUS = [
	(
		"Ada keeps her bicycle in the garden shed behind the house",
		["where does ada keep her bicycle", "which shed holds ada's bicycle"],
	),
	(
		"Marrow the cat refuses to eat anything except salmon",
		["what does marrow the cat eat", "which food will the cat marrow accept"],
	),
	(
		"Quill the parrot mimics the doorbell every afternoon",
		["what sound does quill the parrot mimic", "when does the parrot quill mimic the doorbell"],
	),
	(
		"The dog walker comes at eleven on weekdays",
		["when does the dog walker come", "which days does the dog walker visit"],
	),
	(
		"Bees prefer lavender over roses in the courtyard",
		["which flower do the bees prefer", "do bees like roses or lavender"],
	),
	(
		"Tomatoes need six hours of direct sunlight",
		["how much sunlight do tomatoes need", "do tomatoes want direct sunlight"],
	),
	(
		"Whales migrate past the cape in November",
		["when do whales migrate past the cape", "which month do the whales pass"],
	),
	(
		"Sourdough starter doubles in eight hours at room temperature",
		["how long does sourdough starter take to double", "what temperature does the starter double at"],
	),
	(
		"The espresso grinder needs a burr change annually",
		["how often does the espresso grinder need a burr change", "when to change the grinder burr"],
	),
	(
		"The office coffee machine was replaced in March",
		["when was the office coffee machine replaced", "which month was the coffee machine swapped"],
	),
	(
		"The deploy pipeline runs on Jenkins every night at two",
		["when does the deploy pipeline run", "what runs the nightly deploy"],
	),
	(
		"Rust builds are cached in the shared sccache bucket",
		["where are rust builds cached", "which bucket holds the build cache"],
	),
	(
		"Kubernetes ingress terminates TLS at the edge proxy",
		["where does kubernetes ingress terminate TLS", "which proxy handles TLS termination"],
	),
	(
		"The staging cluster lives in Frankfurt",
		["where does the staging cluster live", "which city hosts staging"],
	),
	(
		"Postgres holds the invoices table on the analytics replica",
		["where is the invoices table", "which replica holds postgres invoices"],
	),
	(
		"The router firmware update broke IPv6 for a week",
		["what broke IPv6", "how long was IPv6 broken after the firmware update"],
	),
	(
		"Container images are signed with cosign before release",
		["how are container images signed", "what signs images before release"],
	),
	(
		"The nightly backup writes to object storage in two regions",
		["where does the nightly backup write", "how many regions hold the backup"],
	),
	(
		"Invoices older than seven years get archived to cold storage",
		["when are invoices archived", "where do old invoices go"],
	),
	(
		"Mortgage rates were renegotiated in October",
		["when were mortgage rates renegotiated", "which month changed the mortgage rates"],
	),
	(
		"The library fine is ten cents per overdue day",
		["how much is the library fine", "what does an overdue day cost"],
	),
	(
		"The quarterly board deck is due the Friday before the meeting",
		["when is the board deck due", "which friday is the quarterly deck due"],
	),
	(
		"Payroll closes on the twenty second of each month",
		["when does payroll close", "which day of the month closes payroll"],
	),
	(
		"Solar panels on the roof produce four kilowatts at noon",
		["how many kilowatts do the solar panels produce", "what do the roof panels make at noon"],
	),
	(
		"Helen swims at the lido on Tuesday mornings",
		["when does helen swim", "where does helen swim on tuesday"],
	),
	(
		"The night train to Vienna leaves platform nine",
		["which platform does the vienna night train leave from", "when does the train to vienna leave"],
	),
	(
		"The fire drill happens on the first Monday of each quarter",
		["when is the fire drill", "which monday holds the fire drill"],
	),
	(
		"The cabin key hangs on the hook by the back door",
		["where is the cabin key", "which hook holds the cabin key"],
	),
	(
		"The ferry to the island runs hourly until dusk",
		["how often does the island ferry run", "until when does the ferry run"],
	),
	(
		"Passports must be renewed six months before travel",
		["when must passports be renewed", "how long before travel to renew a passport"],
	),
	(
		"The museum closes early on public holidays",
		["when does the museum close early", "does the museum shut early on holidays"],
	),
	(
		"Ivan tunes the piano twice a year",
		["how often does ivan tune the piano", "who tunes the piano"],
	),
	(
		"The rooftop antenna was aligned with a compass in spring",
		["how was the rooftop antenna aligned", "when was the antenna aligned"],
	),
	(
		"Winter tyres are mandatory north of the pass from November",
		["when are winter tyres mandatory", "where are winter tyres required"],
	),
	(
		"The chess club meets in the annex on Thursday evenings",
		["when does the chess club meet", "where does the chess club meet"],
	),
	(
		"Rain gauges are read every morning before seven",
		["when are rain gauges read", "how often are the rain gauges read"],
	),
]

FACTS = [fact for fact, _ in CORPUS]
PROBES = [(probe, fact) for fact, probes in CORPUS for probe in probes]


def gold_rank(project, probe, gold):
	stdout, _ = project.run("query", probe)
	for hit in hits(stdout):
		if hit.text == gold:
			return hit.rank
	return None


def test_recall_over_the_inline_corpus(project):
	ingest_all(project, FACTS)

	ranks = [gold_rank(project, probe, gold) for probe, gold in PROBES]
	n = len(ranks)
	at1 = sum(1 for r in ranks if r == 1) / n
	at5 = sum(1 for r in ranks if r is not None and r <= 5) / n
	mrr = sum(1.0 / r for r in ranks if r is not None) / n

	print()
	print(f"kern-recall corpus       {len(FACTS)} facts, {n} probes")
	print(f"kern-recall recall@1     {at1:.4f}  (floor {RECALL_AT_1_FLOOR:.4f})")
	print(f"kern-recall recall@5     {at5:.4f}  (floor {RECALL_AT_5_FLOOR:.4f})")
	print(f"kern-recall MRR          {mrr:.4f}  (floor {MRR_FLOOR:.4f})")
	print(f"kern-recall unretrieved  {sum(1 for r in ranks if r is None)}")

	missed = sorted(
		((r, probe) for r, (probe, _) in zip(ranks, PROBES) if r is None or r > 5),
		key=lambda x: (x[0] is not None, x[0]),
	)
	if missed:
		print(f"kern-recall worst        {missed[:5]}")

	assert at1 >= RECALL_AT_1_FLOOR, f"recall@1 regressed: {at1:.4f} < {RECALL_AT_1_FLOOR}"
	assert at5 >= RECALL_AT_5_FLOOR, f"recall@5 regressed: {at5:.4f} < {RECALL_AT_5_FLOOR}"
	assert mrr >= MRR_FLOOR, f"MRR regressed: {mrr:.4f} < {MRR_FLOOR}"
