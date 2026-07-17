# src/base/locks.rs — commentary

The `*_recovered` names are historical: they predate the parking_lot swap, when
the std locks could poison and these wrappers recovered the `PoisonError` (so a
worker panic could never turn into a daemon crash via an `unwrap()` on a guard).
The many call sites kept the names as a single acquisition point; the wrappers
now simply forward.

Second-pass migration (comment -> note):
- The `//!` doc's explanation of what "no poisoning" buys — a thread that panics
  while holding a guard leaves the lock immediately usable by the next acquirer,
  so there is no `PoisonError` to recover and no `unwrap()` on a guard that could
  turn a worker panic into a daemon crash — is already recorded above; the doc now
  states only the fact and that the `_recovered` names are historical.
