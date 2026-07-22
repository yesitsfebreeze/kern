"""ROADMAP item 30: a watched file gets a durable backstop.

`notify` installs watches and replays nothing, and kern runs no startup scan —
so a record the sink handed to the in-RAM queue was gone for good if the daemon
died before the distill/embed leg committed it. Nothing re-offers it. The fix
routes the sink through the same durable intake `tool_ingest` writes: the record
is parked as `<intake>/direct/<content-hash>.json` first, and the poll loop (or
the next daemon) drains it.

Both halves are subprocess-visible, which is why this is an e2e and not a unit
test: `[watcher] enabled` is ordinary `kern.toml` config and the artifact is a
file on disk.

The claim carries `STALL_MARKER`, so the fake embedder holds it for STALL_SECS.
That is not decoration, and it was measured rather than assumed: the worker
persists the graph after *every* committed job, so against an instant fake the
in-flight window this test aims at is microseconds wide. Half (b) run alone
against a pre-change build **passes** twice in a row with `STALL_SECS = 0` (4.6s,
3.8s) and **fails** with the stall in place (90s of retries). The stall is what
makes "killed with the record still in flight" the deterministic case; without it
this file would be green on the bug.

Half (b) cannot be isolated by this test on a pre-change build — a daemon that
parks nothing dies at (a) first. It was isolated with a scratch probe that drops
the on-disk assertions and takes the kill on a timer; that probe is what the
numbers above come from.
"""

import sys
import time

import pytest

from conftest import wait_until
from fake_llm import STALL_MARKER
from ranking import hits

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="unix sockets only")

CLAIM = f"Ada keeps her bicycle in the garden shed behind the house {STALL_MARKER}"


def test_a_watched_file_survives_a_kill_taken_before_its_distillation_lands(project):
	notes = project.cwd / "notes"
	notes.mkdir()
	# A root narrower than the cwd, so this test measures the backstop and nothing
	# else. The cwd-wide case is the test below.
	project.write_config(watcher_enabled=True, watcher_roots=["notes"])
	daemon = project.start_daemon()

	direct = project.cwd / ".kern" / "intake" / "direct"

	# The bound socket says the daemon is up, not that `notify` has finished
	# installing the watch — so the edit is repeated until it is seen. Same bytes
	# every time, and the payload is named by content hash, so this is one record.
	def parked():
		(notes / "bike.md").write_text(CLAIM)
		return list(direct.glob("*.json"))

	wait_until(parked, 60, f"the watcher parked nothing under {direct}")

	# (a) the record is durable on disk BEFORE anything distilled it — the embed
	# call for this text is still hanging in the fake LLM right now.
	payload = parked()[0].read_text()
	assert STALL_MARKER in payload, f"the parked payload is not the watched file: {payload}"
	assert '"source_tag":"file"' in payload, (
		f"the channel must survive the durable hop — an 'agent' tag here is the "
		f"relabel item 95 closed: {payload}"
	)

	# (b) SIGKILL at that instant (Popen.kill is SIGKILL on posix), restart, and
	# the claim is still reachable. On the pre-item-30 daemon the record lived
	# only in the worker's channel, so this is where it was lost.
	project.stop(daemon)
	project.start_daemon()

	wait_until(
		lambda: list((direct / "done").glob("*.json")),
		90,
		f"the restarted daemon never drained the parked payload in {direct}",
	)

	stdout, stderr = project.run("query", "where does ada keep her bicycle")
	assert any("bicycle" in hit.text for hit in hits(stdout)), (
		f"the watched file did not survive the kill: out={stdout} err={stderr}"
	)


def test_the_default_cwd_root_does_not_ingest_the_intake_it_just_parked(project):
	"""The backstop's own feedback edge, with the config a host actually gets.

	`[watcher] roots` is optional and defaults to the whole cwd, and `intake.dir`
	defaults to `.kern/intake` *inside* that cwd. So parking a record durably puts
	a file in the tree that produced it: the watcher reads it back, parks a
	payload wrapping that payload, and repeats. Measured before the fix, from this
	exact config and one seed edit: 283 payloads in 60 seconds, the largest
	1.77 MB — each one an embed call and a graph write. `spawn_file_watcher` now
	hands `IgnoreRules` the resolved `intake.dir` and `data_dir` as denied
	prefixes.

	The bound is what makes this a test and not a demo: one edit is one payload,
	so anything above a handful is the loop running again.
	"""
	project.write_config(watcher_enabled=True)  # no roots -> the whole cwd
	project.start_daemon()

	direct = project.cwd / ".kern" / "intake" / "direct"
	(project.cwd / "seed.md").write_text("Ada keeps her bicycle in the garden shed")

	def payloads():
		live = list(direct.glob("*.json")) if direct.is_dir() else []
		done = list((direct / "done").glob("*.json")) if (direct / "done").is_dir() else []
		return live + done

	wait_until(lambda: payloads(), 60, f"the watcher parked nothing under {direct}")

	# Two intake poll intervals (5s each) past the first payload: enough for the
	# drain to archive it into `done/` and for a re-ingest of that archived file
	# to have parked its own payload, which is where the loop showed itself.
	time.sleep(15)
	got = payloads()
	assert len(got) <= 2, (
		f"the watcher is ingesting its own intake — one seed edit left {len(got)} "
		f"payloads under {direct}: {sorted(p.name for p in got)}"
	)
