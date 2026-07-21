use std::time::Duration;

// The balanced hybrid mix: ModeWeights::default() and RetrievalConfig.weights_hybrid.
pub const DEFAULT_WEIGHT_CONTENT: f64 = 0.5;
pub const DEFAULT_WEIGHT_REASON: f64 = 0.3;
pub const DEFAULT_WEIGHT_EDGE: f64 = 0.2;

pub const DEFAULT_SEED_K: usize = 5;

pub const QBST_ACCESS_WEIGHT: f64 = 0.02;
pub const QBST_RECENCY_WEIGHT: f64 = 0.05;
pub const QBST_RECENCY_HALF_LIFE: Duration = Duration::from_secs(24 * 60 * 60);
pub const QBST_CAP: f64 = 0.1;

pub const COLD_COMPACT_MIN_BYTES: u64 = 256 * 1024;

pub const COLD_MAX_ENTRIES: usize = 50_000;

// ef=1 is greedy single-path and misses true NN, letting duplicates through.
pub const DEDUP_EF: usize = 64;

pub const INGEST_DEDUP_THRESHOLD: f64 = 0.95;

pub const TICK_MAX_CLUSTER_SAMPLE: usize = 200;
pub const TICK_QUEUE_CAPACITY: usize = 512;
// 0 disables the driver.
pub const TICK_INTERVAL_SECS: u64 = 60;

// "no kern-eviction cap" sentinel. A finite cap is currently unsafe.
pub const KERN_CAP_DISABLED: usize = usize::MAX;

pub const KERN_INNER_RADIUS: f64 = 0.15;
pub const KERN_OUTER_RADIUS: f64 = 0.35;

pub const ACCEPT_FLOOR: f64 = 0.5;

// Empty graviton_vec, so similarity routing never matches it — fallback only.
pub const GENERIC_GRAVITON: &str = "generic";

// Two gravitons above this cosine are the SAME concept. Re-tune if the embed model changes.
pub const GRAVITON_DEDUP_THRESHOLD: f64 = 0.85;
pub const KERN_COHESION_THRESHOLD: f64 = 0.60;
pub const KERN_MIN_CLUSTER_SIZE: usize = 10;
pub const KERN_NAMING_COHESION_THRESHOLD: f64 = 0.50;
pub const KERN_NAMING_MIN_CLUSTER_SIZE: usize = 5;

pub const PULSE_DECAY: f64 = 0.5;
pub const PULSE_THRESHOLD: f64 = 0.05;

pub const REFINE_TRAVERSAL_WEIGHT: f64 = 0.01;
pub const REFINE_BOOST_CAP: f64 = 0.1;

pub const IMPORTANT_MIN_COSINE: f64 = 0.20;
pub const IMPORTANT_ACCESS_THRESHOLD: i32 = 3;

pub const QUERY_CACHE_DEFAULT_CAP: usize = 256;
pub const QUERY_CACHE_DEFAULT_THETA: f64 = 0.97;

pub const FACT_SCORE_BOOST: f64 = 0.3;

pub const DEFAULT_CONFIDENCE: f64 = 0.5;
pub const MAX_AI_CONFIDENCE: f64 = 0.95;
pub const FACT_CONFIDENCE: f64 = 1.0;

pub const PROVENANCE_SCORE: f64 = 0.85;

pub const ANSWER_MAX_CHAINS: usize = 5;
pub const ANSWER_MAX_THOUGHTS: usize = 5;

pub const DEGRADE_DECAY_BASE: f64 = 0.15;
pub const DEGRADE_DECAY_POW: f64 = 0.75;
pub const DEGRADE_MIN_THRESHOLD: f64 = 0.05;

pub const QUESTION_RESOLVE_THRESHOLD: f64 = 0.80;

pub const GOSSIP_FANOUT: usize = 3;
pub const GOSSIP_SEEN_SET_CAP: usize = 10_000;
pub const GOSSIP_SEEN_TTL: Duration = Duration::from_secs(60);
pub const GOSSIP_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
pub const GOSSIP_DISCOVERY_INTERVAL: Duration = Duration::from_secs(10);
pub const GOSSIP_DISCOVERY_MULTICAST: &str = "239.77.75.68";
pub const GOSSIP_MAX_PEERS: usize = 50;
pub const GOSSIP_SEED_ADDR: &str = "seed.kern.dev:7946";
pub const GOSSIP_DIAL_TIMEOUT: Duration = Duration::from_secs(2);
pub const GOSSIP_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
// Reject a larger declared length before reading any body bytes — bounds per-connection memory.
pub const GOSSIP_MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

// At the cap new remote ids are dropped; existing ids still CRDT-merge (known updates never lost).
pub const GOSSIP_REMOTE_KERN_ENTITY_CAP: usize = 50_000;
// Coarsely bounds a peer pinning a per-replica slot toward u64::MAX.
pub const GOSSIP_CRDT_DELTA_MAX: u64 = 1_000_000;

pub const LEDGER_THOUGHT_TTL: Duration = Duration::from_secs(72 * 60 * 60);
pub const LEDGER_ROUTING_TTL: Duration = Duration::from_secs(5 * 60);

// Residency, not forgetting: an unloaded kern is persisted first and reloads on next `get`.
pub const KERN_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const KERN_IDLE_SWEEP_EVERY: Duration = Duration::from_secs(60);

pub const COLD_HEAT_THRESHOLD: f64 = 0.01;

pub const COLD_GC_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub const STIGMERGY_GC_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub const DISK_CONSOLIDATE_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub const DISK_CONSOLIDATE_MIN_DELTA: usize = 10_000;

pub const USER_SOURCE: &str = "user";
pub const AGENT_SOURCE: &str = "agent";
// "agent" as a source is canonically AGENT_SOURCE — do NOT add a second const
// with the same value (it silently collides in the source map).
