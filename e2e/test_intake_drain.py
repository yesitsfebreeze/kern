"""The intake half of ROADMAP item 9: `kern intake drain` goes through the
daemon rather than around it.

`drain_once` reads the queue directory and archives each entry it commits, so a
CLI drain running in its own process distills the same transcript the daemon's
poll loop is about to distill (two LLM calls) and races it for the archive move.
Routing the command through the daemon's `intake_drain` tool leaves exactly one
drainer.

The proof uses the same blinding as test_daemon_reads: a daemon reads its config
once at startup and then holds its store open, so repointing `data_dir`
afterwards blinds every later CLI process without moving the graph the daemon
serves. `kern list` stays local by decision, which makes it the control — the
drained claim has to be absent there and present through the routed `kern
query`.

The daemon's own poll loop is off in the first test, so the *only* thing that
can put the transcript into the graph it serves is the routed drain.
"""

import sys

import pytest

from ranking import hits

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="unix sockets only")

TRANSCRIPT = (
	"user: where does ada put the bike\n"
	"assistant: Ada keeps her bicycle in the garden shed behind the house\n"
)

BLIND = ".kern/blind"


def queue(project, name="sess-1.txt", body=TRANSCRIPT):
	intake = project.cwd / ".kern" / "intake"
	intake.mkdir(parents=True, exist_ok=True)
	(intake / name).write_text(body)
	return intake


def test_intake_drain_lands_in_the_serving_daemon_not_on_the_clis_disk(project):
	project.write_config(intake_enabled=False)
	project.start_daemon()
	# From here the CLI's own store is an empty directory the daemon never opened.
	project.write_config(data_dir=BLIND, intake_enabled=False)

	intake = queue(project)
	stdout, stderr = project.run("intake", "drain")
	assert "drained 1 of 1 pending" in stdout, f"out={stdout} err={stderr}"
	assert not (intake / "sess-1.txt").exists(), f"transcript not consumed: {stdout}"
	assert (intake / "done" / "sess-1.txt").exists(), f"transcript not archived: {stdout}"

	# Only the daemon's live graph can answer this — the CLI's data_dir is empty.
	stdout, stderr = project.run("query", "where does ada keep her bicycle")
	ranked = hits(stdout)
	assert any("bicycle" in hit.text for hit in ranked), (
		f"the drained claim never reached the daemon: out={stdout} err={stderr}"
	)

	# Control: the CLI's own disk view never held it, so the drain cannot have
	# run in this process.
	stdout, _ = project.run("list")
	assert "bicycle" not in stdout, f"the CLI drained locally after all: {stdout}"


def test_intake_drain_still_drains_in_process_with_no_daemon(project):
	"""The NoDaemon fallback: nothing serving, the CLI owns the queue."""
	intake = queue(project)

	stdout, stderr = project.run("intake", "drain")
	assert "drained 1 of 1 pending" in stdout, f"out={stdout} err={stderr}"
	assert (intake / "done" / "sess-1.txt").exists(), f"not archived: {stdout}"

	stdout, _ = project.run("list")
	assert "bicycle" in stdout, f"the local drain must reach the local store: {stdout}"

	stdout, stderr = project.run("query", "where does ada keep her bicycle")
	assert any("bicycle" in hit.text for hit in hits(stdout)), (
		f"local drain query: out={stdout} err={stderr}"
	)
