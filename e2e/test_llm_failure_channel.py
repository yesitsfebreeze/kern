"""ROADMAP item 30: the distill leg's LLM failure now has a channel.

`Client::complete_func` used to end `.and_then(Result::ok).unwrap_or_default()`,
so a hung endpoint, a refused connection, an HTTP 500, an auth rejection and an
empty reply all reached `distill` as the same `""` — with no log line and no
counter anywhere. The product said as much in shipping text: `record_stuck`
writes "no parseable claims (prose reply, or endpoint unreachable)", naming two
causes and conceding it cannot say which.

The counter lives in the *daemon's* process, which is the whole difficulty: it
is an `AtomicU64` static, so a CLI that loads a graph and prints can only ever
read its own zero. `kern health` asks the serving daemon, so these tests drive
the failure through the daemon's intake poll loop and then read it back over the
socket.

The control matters as much as the assertion: a model that answers with prose is
not an endpoint fault, and the same run proves the counter stays put for it.
"""

import sys
import time

import pytest

import fake_llm

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="unix sockets only")

# Each mode, the word the health line must carry, and the ceiling the client
# needs to survive it. Only the hang needs one: against the 600s default the
# poll loop would still be parked when the test gave up, which is exactly the
# ceiling nobody chose that the config key exists to choose.
MODES = [
	pytest.param(fake_llm.CHAT_HANG_MARKER, "transient", 1, id="hang"),
	pytest.param(fake_llm.CHAT_ERROR_MARKER, "transient", None, id="http-500"),
	pytest.param(fake_llm.CHAT_EMPTY_MARKER, "permanent", None, id="empty-body"),
]

LLM_LINE = "failed completions"


def queue_transcript(project, marker):
	intake = project.cwd / ".kern" / "intake"
	intake.mkdir(parents=True, exist_ok=True)
	(intake / "sess-1.txt").write_text(
		"user: what did we settle on\n"
		f"assistant: we settled on {marker} for the rollout\n"
	)


def health_until(project, wants, secs, msg):
	"""Poll `kern health` until it says `wants`. Returns the whole output.

	One subprocess per attempt, so the cadence is seconds rather than the 0.1s
	`wait_until` uses — the intake poll is 5s wide and nothing here is faster.
	"""
	deadline = time.monotonic() + secs
	out = ""
	while True:
		out, err = project.run("health")
		if wants in out:
			return out
		assert time.monotonic() < deadline, f"{msg}: out={out} err={err}"
		time.sleep(1)


@pytest.mark.parametrize("marker,verdict,timeout_secs", MODES)
def test_a_failed_completion_is_counted_and_named_on_the_health_surface(
	project, marker, verdict, timeout_secs
):
	project.write_config(reason_timeout_secs=timeout_secs)
	project.start_daemon()

	# The control, taken before anything can have failed: the line is absent, so
	# a later hit cannot be something the surface always prints.
	before, _ = project.run("health")
	assert LLM_LINE not in before, f"a fresh daemon has no failures: {before}"

	queue_transcript(project, marker)
	out = health_until(
		project, LLM_LINE, 60, "the failed completion never reached kern health"
	)
	assert "last llm failure:" in out, f"counted but not named: {out}"
	assert verdict in out, f"the surface must say which failure it was: {out}"


def test_a_model_that_answers_never_raises_the_endpoint_counter(project):
	"""The case `record_stuck` could not tell from the ones above.

	The fake answers the distill prompt in the shape it asks for, so this is a
	working endpoint and a working model — and the counter that means "the
	endpoint is at fault" has to stay silent through a whole successful ingest.
	"""
	project.start_daemon()
	queue_transcript(project, "nothing unusual")

	# Wait for the claim to land, so this is a completed distill rather than one
	# the assertion outran.
	deadline = time.monotonic() + 60
	while True:
		stdout, stderr = project.run("query", "what did we settle on for the rollout")
		if "rollout" in stdout:
			break
		assert time.monotonic() < deadline, f"never distilled: out={stdout} err={stderr}"
		time.sleep(1)

	out, _ = project.run("health")
	assert LLM_LINE not in out, f"a working endpoint must stay quiet: {out}"
