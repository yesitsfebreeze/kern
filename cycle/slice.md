# Reconcile — cycle/1, [store]

item 76 [store]: watchdog force-exit attempts a bounded guarded flush before process::exit(101), logs which of the two happened (flush landed vs blocked).

Success criteria:
1. spawn_watchdog takes save_fn and is spawned after save_fn is available.
2. On stall, watchdog calls watchdog_flush_attempt(&save_fn, FLUSH_DEADLINE) -> WatchdogFlush::{Flushed,Blocked}.
3. Exit log says which outcome before process::exit(101).
4. Unit test: fast save_fn -> Flushed (ran it); sleeping save_fn -> Blocked (did not complete in window).
5. cargo test --workspace green; docs_check exit 0.
