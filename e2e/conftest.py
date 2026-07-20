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
	return ROOT / "target" / "debug" / "kern"


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
		config_home = tmp_path / "config"
		for d in (self.cwd / ".kern", self.runtime, config_home):
			d.mkdir(parents=True)
		(self.cwd / ".kern" / "kern.toml").write_text(
			f'[embed]\nurl = "{llm_url}"\nmodel = "fake-embed"\n\n'
			f'[reason]\nurl = "{llm_url}"\nmodel = "fake-reason"\n\n'
			f'[answer]\nurl = "{llm_url}"\nmodel = "fake-answer"\n'
		)
		self.env = os.environ | {
			"XDG_RUNTIME_DIR": str(self.runtime),
			"XDG_CONFIG_HOME": str(config_home),
		}
		self._hubs = []

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
		self._hubs.append(child)
		sock = self.runtime / "kern-hub.sock"
		wait_until(lambda: sock.exists(), 10, f"hub never bound {sock}")
		return child

	def kill_all(self):
		for child in self._hubs:
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
