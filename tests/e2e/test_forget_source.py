"""ROADMAP item 19: `kern forget --source <scheme>://<object_id>` removes every
thought a source put in the graph, and `--force` is the one thing that reaches
its local Facts.

Two sources are in play because the CLI can only mint one of each kind:

- `file://` comes from the intake queue — anything that is not a `.txt`
  transcript is ingested whole as a Document (`ingest::intake::drain_document`),
  so a two-paragraph note lands as several non-Fact entities under one
  `Source::File { path: "<filename>" }`. That is the cascade half.
- `inline://` comes from `kern ingest`, which ingests at user confidence 1.0 and
  therefore mints **Facts** under `Source::Inline { hash: sha256(text) }`
  (`ingest_cmd.rs`, `clamp_confidence`). That is the `--force` half. There is no
  CLI path that mints a local Fact under a `file://` source today, so the guard
  is proven where a Fact actually exists.

The routed test blinds the CLI the same way test_daemon_reads does: a daemon
reads its config once at startup and then holds its store open, so repointing
`data_dir` afterwards blinds every later CLI process without moving the graph
the daemon serves. `kern list` stays local by decision and is the control — with
the config repointed it must see nothing while the routed `query` still answers,
which is what makes "the forget landed in the daemon" the only reading.
"""

import hashlib
import sys

import pytest

from ranking import hits

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="unix sockets only")

# Two paragraphs -> two chunks (`split::paragraph_split` cuts on a blank line),
# with disjoint vocabulary so the 0.95 dedup threshold cannot fold them into one.
NOTE = (
	"Ada keeps her bicycle in the garden shed behind the house\n"
	"\n"
	"The deploy pipeline runs on Jenkins every night at two\n"
)
BIKE = "bicycle"
DEPLOY = "Jenkins"

FACT = "Marrow the cat refuses to eat anything except salmon"

BLIND = ".kern/blind"


def inline_source(text):
	"""What `kern ingest` stamps: `Source::Inline { hash: content_hash(text) }`."""
	return "inline://" + hashlib.sha256(text.encode()).hexdigest()


def queue(project, name="notes.md", body=NOTE):
	intake = project.cwd / ".kern" / "intake"
	intake.mkdir(parents=True, exist_ok=True)
	(intake / name).write_text(body)
	return intake


def texts(project, probe):
	stdout, stderr = project.run("query", probe)
	return " ".join(hit.text for hit in hits(stdout)), stdout, stderr


def test_forget_source_removes_every_chunk_of_one_file(project):
	"""The NoDaemon half: nothing serving, the CLI owns the graph."""
	queue(project)
	stdout, stderr = project.run("intake", "drain")
	assert "drained 1 of 1 pending" in stdout, f"out={stdout} err={stderr}"

	found, stdout, stderr = texts(project, "where does ada keep her bicycle")
	assert BIKE in found, f"the note never landed: out={stdout} err={stderr}"
	found, stdout, stderr = texts(project, "what runs the deploy pipeline")
	assert DEPLOY in found, f"second chunk never landed: out={stdout} err={stderr}"

	stdout, stderr = project.run("forget", "--source", "file://notes.md")
	assert "forgot" in stdout, f"forget --source failed: out={stdout} err={stderr}"
	assert "from file://notes.md" in stdout, f"wrong printer output: {stdout}"

	found, stdout, stderr = texts(project, "where does ada keep her bicycle")
	assert BIKE not in found, f"first chunk survived the source forget: {stdout}"
	found, stdout, stderr = texts(project, "what runs the deploy pipeline")
	assert DEPLOY not in found, f"second chunk survived the source forget: {stdout}"


def test_a_fact_in_the_source_needs_force(project):
	"""`--force` is the only bypass of the Fact guard, and never the default."""
	stdout, stderr = project.run("ingest", FACT)
	assert "status=committed" in stdout, f"ingest failed: out={stdout} err={stderr}"
	source = inline_source(FACT)

	stdout, stderr = project.run("forget", "--source", source)
	assert "kept" in stdout and "--force" in stdout, (
		f"a refused Fact must say why and what to do: out={stdout} err={stderr}"
	)
	found, stdout, stderr = texts(project, "what does marrow the cat eat")
	assert "salmon" in found, f"the Fact must survive without --force: {stdout}"

	stdout, stderr = project.run("forget", "--source", source, "--force")
	assert "kept" not in stdout, f"--force must take the Fact: out={stdout} err={stderr}"
	found, stdout, stderr = texts(project, "what does marrow the cat eat")
	assert "salmon" not in found, f"--force did not actually remove the Fact: {stdout}"


def test_a_bad_source_selector_is_refused_not_guessed_at(project):
	for bad in ["notes.md", "ftp://notes.md", "file://"]:
		stdout, stderr = project.run("forget", "--source", bad)
		assert not stdout.strip(), f"{bad} must remove nothing: {stdout}"
		assert "source" in stderr.lower(), f"{bad} must say what was wrong: {stderr}"


def test_force_without_source_is_refused_rather_than_ignored(project):
	"""clap's `requires` does not fire for a SetTrue flag, so this is the guard.

	A `--force` the per-id path quietly drops is the worst of both: the caller
	asked to punch through the Fact guard and gets a refusal that reads like the
	thought was never there.
	"""
	stdout, stderr = project.run("ingest", FACT)
	assert "status=committed" in stdout, f"ingest failed: out={stdout} err={stderr}"

	stdout, stderr = project.run("forget", "--force", "deadbeef")
	assert "--source" in stderr, f"--force with an ID must be refused: {stderr}"
	assert not stdout.strip(), f"nothing may be printed as done: {stdout}"

	found, stdout, stderr = texts(project, "what does marrow the cat eat")
	assert "salmon" in found, f"the Fact must be untouched: {stdout}"


def test_forget_source_lands_in_the_serving_daemon_not_on_the_clis_disk(project):
	project.write_config(intake_enabled=False)
	project.start_daemon()
	# From here the CLI's own store is an empty directory the daemon never opened.
	project.write_config(data_dir=BLIND, intake_enabled=False)

	queue(project)
	stdout, stderr = project.run("intake", "drain")
	assert "drained 1 of 1 pending" in stdout, f"out={stdout} err={stderr}"

	# Only the daemon's live graph can answer this — the CLI's data_dir is empty.
	found, stdout, stderr = texts(project, "where does ada keep her bicycle")
	assert BIKE in found, f"the note never reached the daemon: out={stdout} err={stderr}"

	# Control: the CLI's own disk view never held it, so anything the forget
	# removes below it cannot have removed from there.
	stdout, _ = project.run("list")
	assert BIKE not in stdout, f"the CLI drained locally after all: {stdout}"

	stdout, stderr = project.run("forget", "--source", "file://notes.md")
	assert "from file://notes.md" in stdout, f"routed forget: out={stdout} err={stderr}"

	found, stdout, stderr = texts(project, "where does ada keep her bicycle")
	assert BIKE not in found, f"the daemon's graph kept the source: {stdout}"
	found, stdout, stderr = texts(project, "what runs the deploy pipeline")
	assert DEPLOY not in found, f"the daemon's graph kept the source: {stdout}"
