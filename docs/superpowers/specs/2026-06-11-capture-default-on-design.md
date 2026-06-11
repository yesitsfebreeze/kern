# Capture Default-On Design

**Date:** 2026-06-11
**Status:** Approved

## Goal

Make capture on by default. The whole point of kern is capturing sessions — opt-out
should be the exception, not opt-in the rule. When the system can't fully operate
(no `[reason]` LLM), it degrades gracefully with a single actionable warning rather
than silently doing nothing.

## Changes

### `src/config/capture.rs`

- `enabled: false` → `enabled: true` in `Default`.
- Doc comment updated from *"OFF by default. Opt in via…"* to
  *"ON by default. Disable via `[capture] enabled = false` in kern.toml."*
- Test renamed: `defaults_are_off_with_sane_tunables` →
  `defaults_are_on_with_sane_tunables`; the `assert!(!c.enabled)` assertion
  flipped to `assert!(c.enabled)`.
- The existing `validate_rejects_zero_intervals_only_when_enabled` test passes
  unchanged (it constructs its own `CaptureConfig` values explicitly).

### `src/commands.rs`

- The block-level comment above `spawn_capture` currently reads
  *"off unless `[capture] enabled = true`"* — update to
  *"on by default; disable via `[capture] enabled = false`"*.
- The `warn!` emitted when `enabled` but no reason LLM is present becomes:

  ```
  "capture: spool drain inactive — add a [reason] section to kern.toml to \
   enable distillation; deltas will accumulate in .kern/capture/ and will be \
   processed once the daemon restarts with a reason LLM configured"
  ```

  The message fires once at daemon startup and is silent thereafter (current
  behaviour). No other logic changes.

### `.kern/kern.toml`

Remove the now-redundant `enabled = true` line under `[capture]`.  The section
comment (`# Self-learning: capture Claude Code sessions -> distill -> graph ->
digest.`) is kept.

## Invariants preserved

- Digest writer runs regardless of whether the spool drain is active (no LLM
  required; already the case).
- Deltas that arrive while no LLM is configured accumulate in `.kern/capture/`
  and are processed on the next daemon start that has a `[reason]` LLM — no
  data loss (already the case via the retry-on-failure spool design).
- `validate()` behaviour is unchanged: zero intervals are only rejected when
  `enabled = true`, which is now the default, but the default intervals are
  non-zero so fresh configs still pass.

## Out of scope

- Runtime LLM-availability probing (start spool drain mid-run when Ollama comes
  online) — future work.
- `kern status` degraded-mode signal — future work.
