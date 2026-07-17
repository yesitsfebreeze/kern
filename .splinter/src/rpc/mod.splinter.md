# src/rpc/mod.rs — commentary

- The `kern_rpc` surface is the typed read+write API consumed by sub-agents and external MCP clients. This module is the SERVER half of the boundary (kern implements the `KernRpc` trait); clients dial in with `trnsprt::kern_rpc::KernRpcClient`, and the `kern.sock` endpoint is bound via `trnsprt::typed::LocalListener`.