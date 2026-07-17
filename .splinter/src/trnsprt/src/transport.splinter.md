# src/trnsprt/src/transport.rs — commentary

- `classify_child_log_level`: matching is deliberately case-insensitive — a fixed bug: previously only upper-case "ERROR" / lower-case "error:" matched, so a child logging "Error: boom" slipped through as Info. Guarded by `classify_child_log_level_is_case_insensitive_with_error_priority`.

Second-pass migration:

- `child_stdio_round_trips_a_line_through_an_echo_child`: deleted the 3-line narration of the echo child. What it said, for the record — the test spawns a trivial "read one line from stdin, write it to stdout, exit" child (`cat` on unix, a one-line PowerShell `[Console]::In.ReadLine()` on windows) purely to verify the spawn -> write -> read -> kill cycle against a real subprocess. The `#[cfg]`'d spawn calls show this.
