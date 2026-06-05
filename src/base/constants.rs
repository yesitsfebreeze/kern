use std::collections::HashMap;
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

/// HNSW `ef` (search beam width) used by ingest dedup's nearest-neighbour
/// probe. Dedup asks for the single closest entity (k=1); with `ef=1` the
/// search is greedy single-path and routinely misses the true nearest
/// neighbour, so genuine duplicates slip through and create divergent
/// content-hash entities. A wider beam restores recall at negligible cost for
/// a k=1 query.
pub const DEDUP_EF: usize = 64;

pub const KERN_INNER_RADIUS: f64 = 0.15;
pub const KERN_OUTER_RADIUS: f64 = 0.35;
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
pub const SOURCE_AGENT: &str = "agent";

pub fn default_descriptors() -> HashMap<String, String> {
	let pairs: &[(&str, &str)] = &[
		(SOURCE_CHAT, "A conversation turn between a user and an AI agent. Extract decisions made, questions asked, action items, and key information exchanged."),
		(SOURCE_REQUEST, "A user request or task description given to an AI agent. Extract the goal, constraints, acceptance criteria, and any referenced files or systems."),
		(SOURCE_DECISION, "An architectural or design decision. Extract the decision itself, the alternatives considered, the rationale, and any trade-offs noted."),
		(SOURCE_IDEA, "A brainstorm, hypothesis, or speculative note. Extract the core idea, any supporting reasoning, open questions, and connections to other concepts."),
		(SOURCE_FILE, "File content from the project filesystem. Extract the file's purpose, key exports or interfaces, dependencies, and structural patterns."),
		(SOURCE_CODE, "Source code from a programming language. Extract function signatures, type definitions, key algorithms, error handling patterns, and module boundaries."),
		(SOURCE_DIFF, "A git diff or patch showing changes to source files. Extract what was added, removed, or modified, the intent behind the change, and any files affected."),
		(SOURCE_ERROR, "A build error, test failure, or runtime exception. Extract the error type, message, location (file:line), likely cause, and any stack trace context."),
		(SOURCE_DOC, "Documentation such as a README, wiki page, or manual. Extract the subject, key concepts, usage instructions, API surface, and any warnings or caveats."),
		(SOURCE_TEST, "A test case or test output. Extract what is being tested, the expected vs actual behavior, assertion patterns, and pass/fail status."),
		(SOURCE_CONFIG, "A configuration file (YAML, TOML, JSON, .env). Extract key settings, their values, what they control, and any environment-specific overrides."),
		(SOURCE_LOG, "Log output or structured log entries. Extract timestamps, severity levels, error messages, request IDs, and any patterns or anomalies."),
		(SOURCE_SCHEMA, "A database schema, API schema, or interface definition. Extract entity names, field types, relationships, constraints, and versioning information."),
		(SOURCE_DEP, "Dependency information from a package manifest. Extract direct dependencies, version constraints, notable transitive dependencies, and any security notes."),
		(SOURCE_AGENT, "An AI agent-generated summary, plan, or reflection. Extract the key conclusions, next steps, open questions, and any referenced artifacts."),
	];
	pairs
		.iter()
		.map(|(k, v)| ((*k).to_string(), (*v).to_string()))
		.collect()
}

pub fn register_default_descriptors(descriptors: &mut HashMap<String, String>) -> usize {
	let defaults = default_descriptors();
	let mut n = 0;
	for (name, desc) in defaults {
		if let std::collections::hash_map::Entry::Vacant(e) = descriptors.entry(name) {
			e.insert(desc);
			n += 1;
		}
	}
	n
}
