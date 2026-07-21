"""The one-shot-writer half of ROADMAP item 9: `kern graviton add`/`remove` and
`kern claim-kind add`/`rm` go through the daemon rather than around it.

The local path is `with_graph` — load, mutate, `save_graph_unguarded` — which
writes the WHOLE kern map back holding no writer lock and doing no epoch check.
Run beside a daemon it overwrites everything that daemon has committed since the
CLI loaded, and the daemon's own next persist drops the graviton the CLI just
wrote, because the daemon's live graph never had it.

The proof uses the same blinding as test_intake_drain: a daemon reads its config
once at startup and then holds its store open, so repointing `data_dir`
afterwards blinds every later CLI process without moving the graph the daemon
serves. `graviton list` and `health` stay local by decision, which makes them
both the control (blind, they must see nothing) and, once unblinded, the reader
of whatever the daemon actually persisted.
"""

import re
import sys

import pytest

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="unix sockets only")

SEED = "documentation, manuals and runbooks\nhow to operate the system"
BLIND = ".kern/blind"

_KINDS = re.compile(r"^claim kinds: (\d+)$", re.M)


def claim_kind_count(project):
	stdout, stderr = project.run("health")
	m = _KINDS.search(stdout)
	assert m, f"health printed no claim-kind count: out={stdout} err={stderr}"
	return int(m.group(1))


def test_graviton_add_lands_in_the_serving_daemon_and_survives_its_persist(project):
	project.write_config(intake_enabled=False)
	baseline_kinds = claim_kind_count(project)
	project.start_daemon()
	# From here the CLI's own store is an empty directory the daemon never opened.
	project.write_config(data_dir=BLIND, intake_enabled=False)

	stdout, stderr = project.run("graviton", "add", "docs", SEED)
	assert "graviton added: docs" in stdout, f"out={stdout} err={stderr}"

	# Control: the blind store never got it, so the write did not happen here.
	stdout, _ = project.run("graviton", "list")
	assert "docs" not in stdout, f"the CLI wrote the graviton locally after all: {stdout}"

	# A second routed write makes the daemon flush its whole graph again. That
	# flush is what used to drop a graviton only the CLI's disk ever held.
	stdout, stderr = project.run("claim-kind", "add", "runbook", "operational runbooks")
	assert "claim kind added: runbook" in stdout, f"out={stdout} err={stderr}"

	# Unblind: read back what the daemon itself persisted.
	project.write_config(intake_enabled=False)
	stdout, stderr = project.run("graviton", "list")
	assert "docs" in stdout, (
		f"the daemon's persist dropped the routed graviton: out={stdout} err={stderr}"
	)
	assert claim_kind_count(project) == baseline_kinds + 1, (
		"the routed claim kind never reached the daemon's graph"
	)


def test_graviton_and_claim_kind_still_write_locally_with_no_daemon(project):
	"""The NoDaemon fallback: nothing serving, the CLI owns the write."""
	baseline_kinds = claim_kind_count(project)

	stdout, stderr = project.run("graviton", "add", "docs", SEED)
	assert "graviton added: docs" in stdout, f"out={stdout} err={stderr}"
	stdout, _ = project.run("graviton", "list")
	assert "docs" in stdout, f"the local add must reach the local store: {stdout}"

	stdout, stderr = project.run("claim-kind", "add", "runbook", "operational runbooks")
	assert "claim kind added: runbook" in stdout, f"out={stdout} err={stderr}"
	assert claim_kind_count(project) == baseline_kinds + 1, "local claim-kind add lost"

	stdout, stderr = project.run("graviton", "remove", "docs")
	assert "graviton removed: docs" in stdout, f"out={stdout} err={stderr}"
	stdout, _ = project.run("graviton", "list")
	assert "docs" not in stdout, f"the local remove never landed: {stdout}"

	stdout, stderr = project.run("claim-kind", "rm", "runbook")
	assert "claim kind removed: runbook" in stdout, f"out={stdout} err={stderr}"
	assert claim_kind_count(project) == baseline_kinds, "local claim-kind rm lost"
