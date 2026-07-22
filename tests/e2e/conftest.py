import json
import subprocess
from pathlib import Path

import pytest

from fake_llm import FakeLlm
from harness import KernProject, wait_until  # noqa: F401 — tests import both from here

ROOT = Path(__file__).resolve().parents[2]


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


@pytest.fixture
def project(kern_bin, fake_llm, tmp_path):
	p = KernProject(kern_bin, tmp_path, fake_llm.url)
	yield p
	p.kill_all()
