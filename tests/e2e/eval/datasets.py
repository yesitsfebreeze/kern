"""Fetch the benchmark datasets into /eval/ (gitignored, never committed).

LoCoMo is CC BY-NC 4.0 and LongMemEval is research data — both are downloaded
to the user's machine on demand and stay out of the repo.
"""

import argparse
import sys
import urllib.request
from pathlib import Path

from common import DATA_DIR

LOCOMO_URL = (
	"https://raw.githubusercontent.com/snap-research/locomo/main/data/locomo10.json"
)
LOCOMO_PATH = DATA_DIR / "locomo10.json"

LONGMEMEVAL_REPO = "xiaowu0162/longmemeval"
LONGMEMEVAL_FILE = "longmemeval_s.json"
LONGMEMEVAL_PATH = DATA_DIR / LONGMEMEVAL_FILE

NOTICE = """\
Datasets are for local evaluation only:
- LoCoMo (snap-research/locomo): CC BY-NC 4.0 — non-commercial, no redistribution.
- LongMemEval (xiaowu0162/longmemeval): research benchmark, see its license.
Both live under /eval/, which is gitignored; do not commit them."""


def fetch_locomo():
	if LOCOMO_PATH.exists():
		print(f"locomo: already at {LOCOMO_PATH}")
		return
	DATA_DIR.mkdir(exist_ok=True)
	print(f"locomo: downloading {LOCOMO_URL}")
	urllib.request.urlretrieve(LOCOMO_URL, LOCOMO_PATH)
	print(f"locomo: {LOCOMO_PATH} ({LOCOMO_PATH.stat().st_size:,} bytes)")


def fetch_longmemeval():
	if LONGMEMEVAL_PATH.exists():
		print(f"longmemeval: already at {LONGMEMEVAL_PATH}")
		return
	try:
		from huggingface_hub import hf_hub_download
	except ImportError:
		sys.exit("longmemeval needs huggingface-hub: just e2e-install")
	DATA_DIR.mkdir(exist_ok=True)
	print(f"longmemeval: downloading {LONGMEMEVAL_FILE} from {LONGMEMEVAL_REPO}")
	got = hf_hub_download(
		repo_id=LONGMEMEVAL_REPO,
		filename=LONGMEMEVAL_FILE,
		repo_type="dataset",
		local_dir=DATA_DIR,
	)
	print(f"longmemeval: {got} ({Path(got).stat().st_size:,} bytes)")


def main():
	parser = argparse.ArgumentParser(description=__doc__)
	parser.add_argument(
		"which", nargs="?", default="all", choices=["locomo", "longmemeval", "all"]
	)
	which = parser.parse_args().which
	print(NOTICE)
	if which in ("locomo", "all"):
		fetch_locomo()
	if which in ("longmemeval", "all"):
		fetch_longmemeval()


if __name__ == "__main__":
	main()
