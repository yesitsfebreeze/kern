# src/trnsprt/src/kern_rpc/mod.rs — commentary

Layout: `dto` wire types (several re-exported from `search` so the two services share a wire vocabulary); `svc` the `service!` invocation emitting `KernRpc`/`KernRpcClient`/`serve_kern_rpc`; `mock` in-memory `MockKernServer`; `client_local` convenience constructor dialing the per-user `kern.sock` endpoint.

Forks are deliberately NOT part of `KernRpc` — routing agent session forks through kern would force it to know about agent sessions, which it doesn't.
