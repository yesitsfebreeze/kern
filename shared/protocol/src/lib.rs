//! Wire types shared between the `agnt` daemon and its clients (the agent strip
//! and drill UI). Pure serde DTOs plus the `AgntRpc` service definition — no
//! behaviour — so both sides agree on the surface without depending on each
//! other's internals.

pub mod agent;

// Re-export the agent wire types and the generated `AgntRpc` service surface at
// the crate root, so consumers write `protocol::ForkSnapshot` /
// `protocol::AgntRpcClient` instead of reaching through `protocol::agent::`.
pub use agent::{
    AgentLifecycle, AgntRpc, AgntRpcClient, ForkKind, ForkSnapshot, ForkStateLite, OutputEvent,
    PluginSummary, SlashOutcome, TurnResult, UsageSummary, serve_agnt_rpc,
};
