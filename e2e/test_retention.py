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

# DEDUPED_NEAR is the same token bag as DEDUPED, so the fake embedder returns an
# identical vector and `find_duplicate` — the FIRST dedup gate — merges it, while
# the differing case and trailing period give it a different content hash, which
# is what makes the ingest report `deduped` rather than a no-op re-commit. The
# second gate is unreachable from here (both entities sit in `entity_idx`, so the
# first gate always wins); it is covered by
# `place::tests::the_second_dedup_gate_tightens_too_and_orphans_no_delta`.
DEDUPED = "Vault rotation for the staging cluster is owned by Rui"
DEDUPED_NEAR = "vault rotation for the staging cluster is owned by rui."

RETENTION = 5


def texts_for(project, probe):
	stdout, stderr = project.run("query", probe)
	return [hit.text for hit in hits(stdout)], stdout, stderr


def wait_past_deadline(seconds):
	"""Block until CLOCK_REALTIME has advanced `seconds`, not the monotonic clock.

	`valid_until` is an absolute instant the reader compares against
	`SystemTime::now()`, so realtime is the clock that has to move. `time.sleep`
	waits on the monotonic one, and a host that steps realtime backwards — WSL2's
	hv time sync does, by ~3s every half minute — leaves a monotonic sleep short.
	"""
	target = time.time() + seconds
	cap = time.monotonic() + 4 * seconds + 30
	while time.time() < target:
		assert time.monotonic() < cap, (
			"the realtime clock never advanced past the retention deadline"
		)
		time.sleep(0.25)


def wait_until_dropped(project, probe, text):
	"""Poll until `text` stops being delivered; return the last query output.

	Polling rather than one check after a fixed wait: the same backward realtime
	steps can land BETWEEN the wait and the query and put an already-passed
	deadline back in the future. The monotonic cap keeps a genuine failure to
	expire a failure — it just refuses to read clock noise as one.
	"""
	cap = time.monotonic() + 4 * RETENTION + 30
	while True:
		found, stdout, stderr = texts_for(project, probe)
		if text not in found:
			return stdout, stderr
		assert time.monotonic() < cap, (
			f"{text!r} still delivered well past its retention deadline: "
			f"out={stdout} err={stderr}"
		)
		time.sleep(0.5)


def test_a_retention_expires_the_fact_out_of_query_results(project):
	stdout, stderr = project.run("ingest", EPHEMERAL, "--retention-secs", str(RETENTION))
	assert "status=committed" in stdout, f"ttl ingest failed: out={stdout} err={stderr}"
	stdout, stderr = project.run("ingest", DURABLE)
	assert "status=committed" in stdout, f"control ingest failed: out={stdout} err={stderr}"

	found, stdout, stderr = texts_for(project, EPHEMERAL)
	assert EPHEMERAL in found, (
		f"a fact with an unexpired retention must still be recallable: out={stdout} err={stderr}"
	)

	wait_past_deadline(RETENTION)
	wait_until_dropped(project, EPHEMERAL, EPHEMERAL)

	found, stdout, stderr = texts_for(project, DURABLE)
	assert DURABLE in found, (
		f"expiry took the whole graph with it: out={stdout} err={stderr}"
	)


def test_a_deduped_ingest_still_applies_its_retention(project):
	"""A retention-carrying ingest that lands on a near-duplicate must tighten the
	SURVIVOR's deadline. It reports `deduped` — the incoming entity is dropped
	whole — so before item 88 the flag silently did nothing on this path and left
	an entity that never expires.
	"""
	stdout, stderr = project.run("ingest", DEDUPED)
	assert "status=committed" in stdout, f"seed ingest failed: out={stdout} err={stderr}"

	found, stdout, stderr = texts_for(project, DEDUPED)
	assert DEDUPED in found, f"seed fact not recallable: out={stdout} err={stderr}"

	stdout, stderr = project.run("ingest", DEDUPED_NEAR, "--retention-secs", str(RETENTION))
	assert "status=deduped" in stdout, (
		f"the near-duplicate must merge, not place: out={stdout} err={stderr}"
	)

	found, stdout, stderr = texts_for(project, DEDUPED)
	assert DEDUPED in found, (
		f"the survivor must still be recallable before the deadline: out={stdout} err={stderr}"
	)

	wait_past_deadline(RETENTION)
	wait_until_dropped(project, DEDUPED, DEDUPED)


def test_a_default_ingest_never_expires(project):
	stdout, stderr = project.run("ingest", DURABLE)
	assert "status=committed" in stdout, f"ingest failed: out={stdout} err={stderr}"

	wait_past_deadline(RETENTION + 2)

	found, stdout, stderr = texts_for(project, DURABLE)
	assert DURABLE in found, (
		f"an ingest without --retention-secs must set no valid_until: out={stdout} err={stderr}"
	)
