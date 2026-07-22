"""ROADMAP item 100: `kern health` reports the DAEMON's degradations, not its own.

Eight of the numbers `kern health` prints are scoped to the process that reads
them — the seven fail-open counters summed into `degraded:` are `AtomicU64`
statics, and `evicted:` reads a `Store` field every `Store::open` zeroes. The
`kern health` path opens its own store and then runs no search, no scoring, no
tick, no ingest and no merge, so this process's copies can only ever be zero.
Whatever a serving daemon actually degraded was invisible.

The driver is a chunk a live daemon could not embed: the fake LLM refuses any
embed carrying `FAIL_MARKER` with a permanent 400, so the routed `kern intake
drain` distills a claim, fails to embed it, drops it, and counts the drop — in
the daemon's process.

The blinding is test_intake_drain's: a daemon reads its config once at startup
and holds its store open, so repointing `data_dir` afterwards blinds every later
CLI process without moving the graph or the counters the daemon serves. The
drain is routed, so the refusal can only have been counted over there; a blinded
`kern health` that prints a nonzero count can only have read it off the wire.
"""

import re
import sys

import pytest

from fake_llm import FAIL_MARKER

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="unix sockets only")

BLIND = ".kern/blind"

# One `assistant:` line is one distilled claim, and none of these can be embedded.
# The count is asserted *exactly*, and three rather than one: a surface that
# printed a constant, or summed the wrong source, or double-counted a retry, all
# pass ">= 1" and none of them pass "== 3".
UNEMBEDDABLE_CLAIMS = 3
TRANSCRIPT = "user: where does ada keep her things\n" + "".join(
	f"assistant: Ada keeps her {thing} in the garden shed {FAIL_MARKER}\n"
	for thing in ("bicycle", "helmet", "pump")
)

_DROPPED = re.compile(r"^degraded: +.*?(\d+) chunks lost to embedding", re.M)


def test_health_reports_the_serving_daemons_degradation_counts(project):
	project.write_config(intake_enabled=False)
	project.start_daemon()
	# From here the CLI's own store is an empty directory the daemon never opened,
	# and its own counters are a fresh process's zeros.
	project.write_config(data_dir=BLIND, intake_enabled=False)

	# Before: the daemon is serving and has degraded nothing, so the line is absent.
	# The same blinded CLI printing a count only *after* the daemon drops something
	# is what separates a number read off the wire from a constant in the format.
	stdout, stderr = project.run("health")
	assert not _DROPPED.search(stdout), f"degraded before anything did: {stdout}"

	intake = project.cwd / ".kern" / "intake"
	intake.mkdir(parents=True, exist_ok=True)
	(intake / "sess-1.txt").write_text(TRANSCRIPT)

	# Routed: the drain, the distill and the failing embed all happen in the daemon.
	stdout, stderr = project.run("intake", "drain")
	assert "drained" in stdout, f"out={stdout} err={stderr}"

	stdout, stderr = project.run("health")
	m = _DROPPED.search(stdout)
	assert m, f"no degraded line at all — health printed its own zeros: {stdout}"
	assert int(m.group(1)) == UNEMBEDDABLE_CLAIMS, (
		f"not the count the daemon holds: {stdout}"
	)


def test_health_with_no_daemon_still_reports_this_processs_own_counts(project):
	"""The no-daemon path is untouched: local numbers, and the tick line says so."""
	stdout, stderr = project.run("health")
	assert "tick:        (no daemon serving this directory)" in stdout, (
		f"out={stdout} err={stderr}"
	)
	assert "evicted:     0 cold rows dropped" in stdout, f"out={stdout} err={stderr}"
	# Nothing degraded in this process, so the line stays absent — a healthy kern
	# is quiet, and preferring the daemon must not start printing a row of zeros.
	assert "off-model queries dropped" not in stdout, f"out={stdout} err={stderr}"
