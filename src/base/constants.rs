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

/// Cold-store compaction is skipped until `cold.jsonl` grows past this many
/// bytes. Compaction rewrites the whole file (O(total)), so gating on size
/// stops steady-state GC from rewriting the entire store every sweep for a
/// handful of victims — and because compaction shrinks the file, it
/// self-rate-limits. Reads stay correct meanwhile (latest-line-wins is applied
/// in memory). 256 KiB.
pub const COLD_COMPACT_MIN_BYTES: u64 = 256 * 1024;

/// Absolute cap on entities retained in the cold store. Compaction keeps the
/// newest `COLD_MAX_ENTRIES` by creation time and drops the oldest, so the
/// cold tier cannot grow without bound over the daemon's lifetime.
pub const COLD_MAX_ENTRIES: usize = 50_000;

/// HNSW `ef` (search beam width) used by ingest dedup's nearest-neighbour
/// probe. Dedup asks for the single closest entity (k=1); with `ef=1` the
/// search is greedy single-path and routinely misses the true nearest
/// neighbour, so genuine duplicates slip through and create divergent
/// content-hash entities. A wider beam restores recall at negligible cost for
/// a k=1 query.
pub const DEDUP_EF: usize = 64;

// ── Default ingest knobs ─────────────────────────────────────────────────────
// Shared by the runtime `ingest::Config` and the serde-deserialized
// `config::IngestConfig` so the two layers' defaults cannot silently drift.

/// Cosine-similarity floor above which a freshly-ingested vector is treated as a
/// duplicate of an existing entity (place::find_duplicate) and merged rather than
/// inserted. Higher than the anchor-path [`DEFAULT_DEDUP_THRESHOLD`] (0.92):
/// ingest dedup wants near-exact matches before collapsing two thoughts.
pub const INGEST_DEDUP_THRESHOLD: f64 = 0.95;
/// Nearest-neighbour count (`k`) for the ingest synthesis/rephrase HNSW probe.
pub const INGEST_HNSW_K: usize = 8;
/// HNSW search beam width (`ef`) for that probe; wider = better recall, more work.
pub const INGEST_HNSW_EF: usize = 32;
/// Lower edge of the rephrase similarity band: a candidate at or below this is
/// too dissimilar to merge, so it stays a distinct entity.
pub const INGEST_REPHRASE_LOWER: f64 = 0.85;
/// Upper edge of the rephrase band: at or above this the candidate is a
/// near-duplicate (dedup territory). Only entities STRICTLY between the two
/// bounds are rephrase/merge candidates.
pub const INGEST_REPHRASE_UPPER: f64 = 0.95;

// ── Default autonomous-maintenance tick knobs (config::TickConfig) ────────────
/// Max entities sampled when clustering a kern for auto-naming / child-spawn.
/// Caps clustering cost on large kerns (coarser sampling above this size).
pub const TICK_MAX_CLUSTER_SAMPLE: usize = 200;
/// Bounded capacity of the maintenance-tick task queue.
pub const TICK_QUEUE_CAPACITY: usize = 512;
/// Default seconds between autonomous maintenance ticks; `0` disables the driver.
pub const TICK_INTERVAL_SECS: u64 = 60;

/// Sentinel for `GraphConfig::max_kerns` / `GraphGnn::max_loaded_kerns` meaning
/// "no kern-eviction cap" (the shipped default). A finite cap is currently unsafe
/// — see the `GraphConfig::default` comment for the evict/persist consistency bug
/// it triggers — so this is the only value used. Named so the sentinel reads as
/// intent at every site instead of a bare `usize::MAX`.
pub const KERN_CAP_DISABLED: usize = usize::MAX;

pub const KERN_INNER_RADIUS: f64 = 0.15;
pub const KERN_OUTER_RADIUS: f64 = 0.35;

/// Minimum acceptance probability an entity must reach against a named anchor
/// (a named child of the dispatcher) to be routed into it. Below this, the
/// entity falls through to the `generic` catch-all anchor. Matches the
/// long-standing rejection cutoff used by the per-node acceptance gate.
pub const ACCEPT_FLOOR: f64 = 0.5;

/// Reserved anchor name for the root's permanent catch-all child. It carries an
/// empty `anchor_vec`, so similarity routing never matches it — it is reachable
/// only as the fallback when no named anchor clears `ACCEPT_FLOOR`.
pub const GENERIC_ANCHOR: &str = "generic";
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

/// Semantic query cache defaults. Live here (pure data) rather than in
/// `retrieval::cache` so `config` can default them WITHOUT a `config -> retrieval`
/// dependency cycle; the cache module reads them from here too.
pub const QUERY_CACHE_DEFAULT_CAP: usize = 256;
/// Cosine floor for a semantic cache hit — high enough that only paraphrases and
/// re-asks collide, not merely topical neighbours.
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

/// Maximum entities a per-network `remote-*` phantom kern may hold. Bounds
/// memory growth from a peer spamming forged `EntitySync` bodies: once the cap
/// is reached, brand-new remote ids are dropped while existing ids still
/// CRDT-merge (so legitimate updates to known entities are never lost).
pub const GOSSIP_REMOTE_KERN_ENTITY_CAP: usize = 50_000;
/// Upper bound on an inbound CRDT delta's per-replica value. The value is the
/// sender's absolute slot total, max-merged into the local GCounter; rejecting
/// values above this coarsely bounds a peer pinning a slot toward `u64::MAX`.
/// Realistic access/traversal tallies are far below this. (Full per-replica
/// ownership authentication is tracked separately.)
pub const GOSSIP_CRDT_DELTA_MAX: u64 = 1_000_000;

pub const LEDGER_THOUGHT_TTL: Duration = Duration::from_secs(72 * 60 * 60);
pub const LEDGER_ROUTING_TTL: Duration = Duration::from_secs(5 * 60);

pub const KERN_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const KERN_IDLE_SWEEP_EVERY: Duration = Duration::from_secs(60);

/// Stigmergy cold-path garbage collection: heat below which a thought is
/// considered fully evaporated. Heat half-life is ~36h (see
/// docs/kern/stigmergy-self-improving.md), so after 7 days an unaccessed
/// thought decays by ~2^(-7*24/36) ≈ 0.01, matching this threshold.
pub const COLD_HEAT_THRESHOLD: f64 = 0.01;

/// Stigmergy cold-path garbage collection: minimum age (since last access)
/// before a cold thought is eligible for `forget()`. Seven days gives the
/// heat half-life (~36h) roughly five half-lives to decay any transient
/// activity before we treat a thought as truly abandoned.
pub const COLD_GC_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Stigmergy cold-path garbage collection: how often the pulse driver
/// triggers a system-wide GC sweep. With heat half-life ~36h and an age
/// gate of 7 days, hourly is more than fast enough — the gate dominates,
/// not the sweep frequency. The TaskKey dedup map prevents duplicate
/// `StigmergyGc` tasks per kern while one is pending.
pub const STIGMERGY_GC_INTERVAL: Duration = Duration::from_secs(60 * 60);

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
// "agent" as a source is canonically AGENT_SOURCE (paired with USER_SOURCE for
// the Fact-tier gate); the default descriptor map (see `base::descriptors`)
// reuses it rather than defining a second const with the same value.
