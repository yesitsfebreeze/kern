# src/trnsprt/src/typed/adapter.rs — commentary

- `AsyncStdioAdapter::split` / `WriterWithChild`: the child handle is deliberately moved into the writer side via a struct owning both — cleaner than dropping it on the floor; when the writer drops, so does the child, killing any orphan (kill-on-drop rationale also documented on the struct itself, kept inline as load-bearing).
- `inproc_reader_drains_leftover_across_small_reads` (test): one 5-byte write read back two bytes at a time so the reader's `leftover` buffer is exercised (chunk larger than the read buffer).

Second-pass migration: `AsyncStdioAdapter` doc compressed — it is distinct from the legacy synchronous `ChildStdio`. No `Drop` on the adapter itself: `split` moves its fields out, which a `Drop` type forbids; the adapter is always split immediately in practice. Kill-on-drop stays inline (load-bearing invariant).
Design notes (moved from source comments during comment sweep):
- Adapter (TCP, async stdio, in-proc) delivers raw bytes, splitting into AsyncRead/AsyncWrite halves so a Channel frames each direction.
- TcpAdapter::bind is the server-side counterpart to connect; pair with accept.
- AsyncStdioAdapter wraps a tokio::process::Child's stdin/stdout. HAZARD (kept tightened in source): on split the Child moves into the writer, whose Drop calls start_kill — tokio's Child detaches on drop, so without this the dropped writer would orphan the MCP subprocess.
- InprocAdapter::pair is a pair of in-process byte channels for tests: bytes written to `a` arrive when `b` reads, and vice versa.
