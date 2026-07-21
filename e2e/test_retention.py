"""Per-source TTL end to end: `kern ingest --retention-secs N` is the writer half,
`retrieval::score::drop_expired` is the reader half. Ingest a fact with a short
retention, prove `kern query` returns it before the deadline and drops it after,
while a fact ingested without the flag keeps coming back.

Wall-clock is load-bearing here — `valid_until` is an absolute instant compared
against `SystemTime::now()` on every delivery, so the test sleeps rather than
mocking. RETENTION is kept short and the durable control fact carries the whole
"the graph is still there" half, so a slow box costs seconds, not a false pass.
"""

import time

from ranking import hits

EPHEMERAL = "The pager rotation this week belongs to Ada"
DURABLE = "Marrow the cat refuses to eat anything except salmon"

RETENTION = 5


def texts_for(project, probe):
	stdout, stderr = project.run("query", probe)
	return [hit.text for hit in hits(stdout)], stdout, stderr


def test_a_retention_expires_the_fact_out_of_query_results(project):
	stdout, stderr = project.run("ingest", EPHEMERAL, "--retention-secs", str(RETENTION))
	assert "status=committed" in stdout, f"ttl ingest failed: out={stdout} err={stderr}"
	stdout, stderr = project.run("ingest", DURABLE)
	assert "status=committed" in stdout, f"control ingest failed: out={stdout} err={stderr}"

	found, stdout, stderr = texts_for(project, EPHEMERAL)
	assert EPHEMERAL in found, (
		f"a fact with an unexpired retention must still be recallable: out={stdout} err={stderr}"
	)

	time.sleep(RETENTION + 2)

	found, stdout, stderr = texts_for(project, EPHEMERAL)
	assert EPHEMERAL not in found, (
		f"an expired fact was still delivered: out={stdout} err={stderr}"
	)

	found, stdout, stderr = texts_for(project, DURABLE)
	assert DURABLE in found, (
		f"expiry took the whole graph with it: out={stdout} err={stderr}"
	)


def test_a_default_ingest_never_expires(project):
	stdout, stderr = project.run("ingest", DURABLE)
	assert "status=committed" in stdout, f"ingest failed: out={stdout} err={stderr}"

	time.sleep(RETENTION + 2)

	found, stdout, stderr = texts_for(project, DURABLE)
	assert DURABLE in found, (
		f"an ingest without --retention-secs must set no valid_until: out={stdout} err={stderr}"
	)
