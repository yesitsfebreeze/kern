# src/trnsprt/src/inproc.rs — commentary

Second-pass migration:

- `InProcTransport` doc 8 lines -> 2 (purpose + the "tests/local-dev only" trap). Moved here:
  - It is a synchronous loopback: writing a newline-delimited JSON-RPC request dispatches it immediately against the owned `McpServer` and buffers the reply for the next read. No socket, no async, no separate process.
  - Why tests/local-dev ONLY: single-threaded; the request/response buffers are unbounded and grow with traffic; no backpressure, no cancellation. Use the HTTP or local-socket transports for real deployments.
- `Read::read` bulk-copy comment compressed to one line. The full point: it copies out of the ring buffer rather than popping byte-by-byte, and because the `VecDeque` may straddle its internal wrap, it must copy each contiguous slice (`as_slices` head then tail) in turn before draining the consumed prefix. `tiny_reads_reassemble_the_full_frame_across_the_ring` guards this.
- Test `EchoServer` is a local mock rather than `test_utils::AdderServer` because of trnsprt's dev-dep cycle (`trnsprt -> test-utils -> trnsprt`) — see the fuller writeup in the `http.rs` note.
