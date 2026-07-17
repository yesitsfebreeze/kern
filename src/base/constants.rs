use std::time::Duration;

pub const DEFAULT_WEIGHT_CONTENT: f64 = 0.5;
pub const DEFAULT_WEIGHT_REASON: f64 = 0.3;
pub const DEFAULT_WEIGHT_SCORE: f64 = 0.2;

pub const DEFAULT_SEED_K: usize = 5;
pub const DEFAULT_DECAY: f64 = 0.45;

pub const QBST_ACCESS_WEIGHT: f64 = 0.02;
pub const QBST_RECENCY_WEIGHT: f64 = 0.05;
pub const QBST_RECENCY_HALF_LIFE: Duration = Duration::from_secs(24 * 60 * 60);
pub const QBST_CAP: f64 = 0.1;

pub const DEFAULT_DEDUP_THRESHOLD: f64 = 0.92;

/// Compaction rewrites the whole file (O(total)), so it is gated on size.
pub const COLD_COMPACT_MIN_BYTES: u64 = 256 * 1024;

/// Cap on entities retained in the cold store; compaction keeps the newest by
/// creation time.
pub const COLD_MAX_ENTRIES: usize = 50_000;

/// HNSW `ef` for ingest dedup's k=1 probe — `ef=1` is greedy single-path and
/// misses true nearest neighbours, letting duplicates through.
pub const DEDUP_EF: usize = 64;

/// Duplicate floor for a freshly-ingested vector. Shared by the runtime
/// `ingest::Config` and `config::IngestConfig` so the defaults cannot drift.
pub const INGEST_DEDUP_THRESHOLD: f64 = 0.95;

/// Max entities sampled when clustering a kern for auto-naming / child-spawn.
pub const TICK_MAX_CLUSTER_SAMPLE: usize = 200;
pub const TICK_QUEUE_CAPACITY: usize = 512;
/// `0` disables the driver.
pub const TICK_INTERVAL_SECS: u64 = 60;

/// Sentinel for `GraphConfig::max_kerns` / `GraphGnn::max_loaded_kerns` meaning
/// "no kern-eviction cap". A finite cap is currently unsafe (see note).
pub const KERN_CAP_DISABLED: usize = usize::MAX;

pub const KERN_INNER_RADIUS: f64 = 0.15;
pub const KERN_OUTER_RADIUS: f64 = 0.35;

/// Acceptance floor for routing into a named anchor; below it the entity falls
/// through to the `generic` catch-all.
pub const ACCEPT_FLOOR: f64 = 0.5;

/// Root's permanent catch-all child. Carries an empty `anchor_vec`, so
/// similarity routing never matches it — it is the fallback only.
pub const GENERIC_ANCHOR: &str = "generic";

/// Cosine floor above which two anchor vectors are the SAME concept, blocking
/// promotion of a near-duplicate root anchor. Re-tune if the embed model changes.
pub const ANCHOR_DEDUP_THRESHOLD: f64 = 0.85;
pub const KERN_COHESION_THRESHOLD: f64 = 0.60;
pub const KERN_MIN_CLUSTER_SIZE: usize = 10;
pub const KERN_NAMING_COHESION_THRESHOLD: f64 = 0.50;
pub const KERN_NAMING_MIN_CLUSTER_SIZE: usize = 5;

pub const PULSE_DECAY: f64 = 0.5;
pub const PULSE_THRESHOLD: f64 = 0.05;

pub const REFINE_TRAVERSAL_WEIGHT: f64 = 0.01;
pub const REFINE_BOOST_CAP: f64 = 0.1;
pub const REFINE_INTERVAL: u32 = 10;

pub const IMPORTANT_MIN_COSINE: f64 = 0.20;
pub const IMPORTANT_ACCESS_THRESHOLD: i32 = 3;

/// Semantic query cache defaults.
pub const QUERY_CACHE_DEFAULT_CAP: usize = 256;
/// Cosine floor for a semantic cache hit — only paraphrases collide, not
/// topical neighbours.
pub const QUERY_CACHE_DEFAULT_THETA: f64 = 0.97;

pub const FACT_SCORE_BOOST: f64 = 0.3;

pub const DEFAULT_CONFIDENCE: f64 = 0.5;
pub const MAX_AI_CONFIDENCE: f64 = 0.95;
pub const FACT_CONFIDENCE: f64 = 1.0;

pub const PROVENANCE_SCORE: f64 = 0.85;

pub const ANSWER_MAX_CHAINS: usize = 5;
pub const ANSWER_MAX_THOUGHTS: usize = 5;

pub const MIN_DELIVER_SCORE: f64 = 0.40;
pub const MAX_DELIVER_RESULTS: usize = 10;

pub const DEGRADE_DECAY_BASE: f64 = 0.15;
pub const DEGRADE_DECAY_POW: f64 = 0.75;
pub const DEGRADE_MIN_THRESHOLD: f64 = 0.05;

pub const QUESTION_RESOLVE_THRESHOLD: f64 = 0.80;

pub const MCP_VERSION: &str = "2024-11-05";

pub const GOSSIP_FANOUT: usize = 3;
pub const GOSSIP_SEEN_SET_CAP: usize = 10_000;
pub const GOSSIP_SEEN_TTL: Duration = Duration::from_secs(60);
pub const GOSSIP_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
pub const GOSSIP_DISCOVERY_INTERVAL: Duration = Duration::from_secs(10);
pub const GOSSIP_DISCOVERY_MULTICAST: &str = "239.77.75.68";
pub const GOSSIP_MAX_PEERS: usize = 50;
pub const GOSSIP_SEED_ADDR: &str = "seed.kern.dev:7946";
/// Connect timeout when dialing a peer for a one-way send or a fetch.
pub const GOSSIP_DIAL_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a fetch waits for the peer's reply frame before giving up.
pub const GOSSIP_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// Hard cap on a single length-prefixed gossip frame; a larger declared length
/// is rejected before any body bytes are read, bounding per-connection memory.
pub const GOSSIP_MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Max entities a `remote-*` phantom kern may hold. At the cap new remote ids
/// are dropped; existing ids still CRDT-merge, so known updates are never lost.
pub const GOSSIP_REMOTE_KERN_ENTITY_CAP: usize = 50_000;
/// Upper bound on an inbound CRDT delta's per-replica value — coarsely bounds a
/// peer pinning a slot toward `u64::MAX`.
pub const GOSSIP_CRDT_DELTA_MAX: u64 = 1_000_000;

pub const LEDGER_THOUGHT_TTL: Duration = Duration::from_secs(72 * 60 * 60);
pub const LEDGER_ROUTING_TTL: Duration = Duration::from_secs(5 * 60);

pub const KERN_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const KERN_IDLE_SWEEP_EVERY: Duration = Duration::from_secs(60);

/// Stigmergy cold-path GC: heat below which a thought is fully evaporated.
pub const COLD_HEAT_THRESHOLD: f64 = 0.01;

/// Stigmergy cold-path GC: minimum age since last access before a cold thought
/// is eligible for `forget()`.
pub const COLD_GC_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Stigmergy cold-path GC: sweep period. The `COLD_GC_AGE` gate dominates, not
/// this frequency.
pub const STIGMERGY_GC_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Disk-index consolidation period (fold the in-RAM delta into a fresh DiskANN
/// snapshot). Fires only when disk-backed AND past [`DISK_CONSOLIDATE_MIN_DELTA`].
pub const DISK_CONSOLIDATE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Buffered post-snapshot writes needed before consolidation is worth its
/// rebuild cost.
pub const DISK_CONSOLIDATE_MIN_DELTA: usize = 10_000;

pub const USER_SOURCE: &str = "user";
pub const AGENT_SOURCE: &str = "agent";
pub const SOURCE_CHAT: &str = "chat";
pub const SOURCE_REQUEST: &str = "request";
pub const SOURCE_DECISION: &str = "decision";
pub const SOURCE_IDEA: &str = "idea";
pub const SOURCE_FILE: &str = "file";
pub const SOURCE_CODE: &str = "code";
pub const SOURCE_DIFF: &str = "diff";
pub const SOURCE_ERROR: &str = "error";
pub const SOURCE_DOC: &str = "doc";
pub const SOURCE_TEST: &str = "test";
pub const SOURCE_CONFIG: &str = "config";
pub const SOURCE_LOG: &str = "log";
pub const SOURCE_SCHEMA: &str = "schema";
pub const SOURCE_DEP: &str = "dep";
// "agent" as a source is canonically AGENT_SOURCE — do NOT add a second const
// with the same value (it silently collides in the descriptor map).
