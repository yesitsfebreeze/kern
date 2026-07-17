# src/base/time.rs — commentary

Placement: lives in `base` rather than `mcp` because it has no MCP coupling and
any transport/CLI layer needs the same parse.

Second-pass migration (comment -> note):
- Why the parse is deliberately partial: only the fixed-offset
  `YYYY-MM-DDTHH:MM:SS` prefix (bytes 0..19) is read. The timezone suffix (`Z` /
  `±hh:mm`) and the sub-second fraction are ignored because callers only need
  second-granularity wall-clock instants for filter bounds.
- Why every malformed input is `Err(())` rather than a panic: the input is
  reachable from untrusted MCP `since` / `before` / `valid_at` arguments. The
  rejected cases are short-after-trim, non-ASCII/multi-byte inside the fixed slice
  region, non-numeric fields, and a pre-epoch result.
- The length-and-ASCII guard is the whole trap: the fixed slices are taken AFTER
  `trim()`, so the check must also run after the trim, and the bytes must be ASCII
  or a `str` slice would land on a non-char boundary and panic. Tests pin both
  (`   2026   ` trims short despite being >=20 bytes untrimmed; `é` is 2 bytes).
- `days_from_civil` is the standard Howard Hinnant civil-from-days algorithm,
  era-based so it handles negative years; kept inline to stay dependency-free.
