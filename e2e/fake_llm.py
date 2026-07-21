"""Deterministic fake Ollama server for e2e runs — no GPU, no model.

Serves the native API kern speaks to a localhost URL without /v1:
- POST /api/embed  -> feature-hashed bag-of-words vectors; token overlap
  yields real cosine similarity, so retrieval ranking is meaningful.
- POST /api/chat   -> echoes the last user message back as the completion,
  so a test can assert what reached any chat-completion prompt. The one
  exception is the intake distill prompt, which the echo cannot satisfy (see
  `distilled`).
"""

import hashlib
import json
import math
import re
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DIM = 384


def embed(text):
	vec = [0.0] * DIM
	for tok in re.findall(r"[a-z0-9]+", text.lower()):
		h = hashlib.blake2b(tok.encode(), digest_size=8).digest()
		idx = int.from_bytes(h[:4], "little") % DIM
		vec[idx] += 1.0 if h[4] & 1 else -1.0
	norm = math.sqrt(sum(v * v for v in vec)) or 1.0
	return [v / norm for v in vec]


_DISTILL = "Output ONLY a JSON array"


def distilled(prompt):
	"""Answer the intake distill prompt in the shape it asks for.

	The echo cannot: src/ingest/distill.rs::parse_claims spans the first '[' to
	the last ']', and the prompt's own "If nothing is worth keeping, output []"
	puts prose inside that span, so an echoed prompt always parses as garbage.
	Every `assistant:` line of the conversation becomes one claim.
	"""
	body = prompt.split("CONVERSATION:", 1)[-1]
	claims = []
	for line in body.splitlines():
		if not line.startswith("assistant:"):
			continue
		text = line.split(":", 1)[1].strip()
		if text:
			claims.append({"text": text, "kind": "fact"})
	return json.dumps(claims)


class _Handler(BaseHTTPRequestHandler):
	def log_message(self, *args):
		pass

	def _reply(self, payload):
		body = json.dumps(payload).encode()
		self.send_response(200)
		self.send_header("Content-Type", "application/json")
		self.send_header("Content-Length", str(len(body)))
		self.end_headers()
		self.wfile.write(body)

	def do_POST(self):
		length = int(self.headers.get("Content-Length", 0))
		req = json.loads(self.rfile.read(length) or b"{}")
		if self.path == "/api/embed":
			inp = req.get("input", "")
			texts = inp if isinstance(inp, list) else [inp]
			self._reply({"embeddings": [embed(t) for t in texts]})
		elif self.path == "/api/chat":
			last = ""
			for msg in req.get("messages", []):
				if msg.get("role") == "user":
					last = msg.get("content", "")
			reply = distilled(last) if _DISTILL in last else last
			self._reply({"message": {"role": "assistant", "content": reply}, "done": True})
		else:
			self.send_error(404)


class FakeLlm:
	def __init__(self):
		self.server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
		self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
		self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
		self.thread.start()

	def close(self):
		self.server.shutdown()
		self.server.server_close()
