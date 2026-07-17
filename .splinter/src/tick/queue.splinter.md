# src/tick/queue.rs — commentary

- `TaskKind::SeedQuestions`: replaced the long-dead no-op `Split` variant.

## Second-pass migration:
- TaskKind doc comments trimmed to purpose + `extra`/`kern_id` payload; the "enqueued by X hook, dispatched in process_task to Y" plumbing was dropped (process_task's match is the dispatch map).
- `enqueue` send-failure path rolls back BOTH the inflight counter and the pending marker: otherwise a full-channel try_send failure leaves the key flagged forever and dedup blocks every future re-enqueue. Guard: `full_channel_send_failure_rolls_back_pending`.
- `task_commit_access`: entity ids are content hashes / doc ids and never contain a newline, so the newline-join round-trips.
