import json
import os
import subprocess
import time
from pathlib import Path

import pytest

from fake_llm import FakeLlm

ROOT = Path(__file__).resolve().parent.parent


@pytest.fixture(scope="session")
def kern_bin():
	subprocess.run(["cargo", "build", "--bin", "kern"], cwd=ROOT, check=True)
	# Asked, not assumed: a worktree can point build.target-dir at a shared cache,
	# and `<repo>/target` is then a directory that never gets written.
	meta = subprocess.run(
		["cargo", "metadata", "--format-version", "1", "--no-deps"],
		cwd=ROOT,
		capture_output=True,
		text=True,
		check=True,
	)
	return Path(json.loads(meta.stdout)["target_directory"]) / "debug" / "kern"


@pytest.fixture(scope="session")
def fake_llm():
	srv = FakeLlm()
	yield srv
	srv.close()


class KernProject:
	"""One isolated kern project: private cwd, runtime dir, config dir.

	XDG_RUNTIME_DIR isolates the hub socket from any user hub;
	XDG_CONFIG_HOME keeps the user's real kern.toml out of the layered load.
	"""

	def __init__(self, kern_bin, tmp_path, llm_url):
		self.bin = str(kern_bin)
		self.cwd = tmp_path / "proj"
		self.runtime = tmp_path / "run"
		self.llm_url = llm_url
		config_home = tmp_path / "config"
		for d in (self.cwd / ".kern", self.runtime, config_home):
			d.mkdir(parents=True)
		self.write_config()
		self.env = os.environ | {
			"XDG_RUNTIME_DIR": str(self.runtime),
			"XDG_CONFIG_HOME": str(config_home),
		}
		self._children = []

	def write_config(
		self,
		data_dir=None,
		intake_enabled=True,
		intake_retention_secs=0,
		watcher_enabled=False,
		watcher_roots=None,
	):
		"""(Re)write the project kern.toml. `data_dir` is cwd-relative.

		Config is read once per process, at startup — so rewriting this while a
		daemon runs repoints the *next* CLI invocation without moving the store
		the daemon already holds open.

		`intake_enabled=False` stops the daemon spawning its own intake poll
		loop; `kern intake drain` ignores the flag, being an explicit request.

		`intake_retention_secs` is the standing per-source TTL for everything
		that queue ingests. It is omitted when 0 so the default config text —
		what every other test loads — stays exactly what it was.

		`watcher_enabled` / `watcher_roots` turn on the file watcher, whose
		`roots` are cwd-relative and default to the whole cwd. The section is
		emitted only when enabled, for the same reason: every other test's
		config text stays byte-identical.
		"""
		head = f'data_dir = "{data_dir}"\n\n' if data_dir else ""
		ttl = (
			f"retention_secs = {intake_retention_secs}\n" if intake_retention_secs else ""
		)
		watcher = ""
		if watcher_enabled:
			roots = f"roots = {json.dumps(list(watcher_roots))}\n" if watcher_roots else ""
			watcher = f"\n[watcher]\nenabled = true\n{roots}"
		(self.cwd / ".kern" / "kern.toml").write_text(
			f"{head}"
			f'[embed]\nurl = "{self.llm_url}"\nmodel = "fake-embed"\n\n'
			f'[reason]\nurl = "{self.llm_url}"\nmodel = "fake-reason"\n\n'
			f"[intake]\nenabled = {str(intake_enabled).lower()}\n{ttl}"
			f"{watcher}\n"
		)

	def run(self, *args, timeout=120):
		out = subprocess.run(
			[self.bin, *args],
			cwd=self.cwd,
			env=self.env,
			capture_output=True,
			text=True,
			timeout=timeout,
		)
		return out.stdout, out.stderr

	def spawn(self, *args, **popen_kw):
		popen_kw.setdefault("stdin", subprocess.DEVNULL)
		popen_kw.setdefault("stdout", subprocess.DEVNULL)
		popen_kw.setdefault("stderr", subprocess.DEVNULL)
		return subprocess.Popen(
			[self.bin, *args], cwd=self.cwd, env=self.env, **popen_kw
		)

	def start_hub(self, *extra):
		child = self.spawn("hub", *extra)
		self._children.append(child)
		sock = self.runtime / "kern-hub.sock"
		wait_until(lambda: sock.exists(), 10, f"hub never bound {sock}")
		return child

	def node_sockets(self):
		"""Every per-project daemon socket in this project's private runtime dir.

		The name carries an FNV tag of the daemon's cwd (trnsprt::typed::local),
		which the test has no way to recompute — so it matches on the shape.
		"""
		return [
			p
			for p in self.runtime.iterdir()
			if p.name.startswith("kern-") and p.name != "kern-hub.sock"
		]

	def start_daemon(self):
		child = self.spawn("--daemon")
		self._children.append(child)
		wait_until(
			lambda: bool(self.node_sockets()),
			60,
			f"daemon never bound a socket under {self.runtime}",
		)
		return child

	def stop(self, child):
		"""Kill a child and reap it.

		A killed daemon leaves its socket *file* behind — nothing unlinks it. That
		is exactly the shape the NoDaemon fallback has to survive in the field, so
		the test does not clean it up: connect-refused must read as "no daemon".
		"""
		child.kill()
		child.wait()
		if child in self._children:
			self._children.remove(child)

	def kill_all(self):
		for child in self._children:
			child.kill()
			child.wait()


def wait_until(cond, secs, msg):
	deadline = time.monotonic() + secs
	while not cond():
		assert time.monotonic() < deadline, msg
		time.sleep(0.1)


@pytest.fixture
def project(kern_bin, fake_llm, tmp_path):
	p = KernProject(kern_bin, tmp_path, fake_llm.url)
	yield p
	p.kill_all()
