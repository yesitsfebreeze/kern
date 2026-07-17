# src/config-io/src/lib.rs — commentary

- `read_value` / `read_value_parses_leading_section_header`: regression history — `read_value` originally used `parse::<toml::Value>()`, which misread a leading `[section]` header as an array literal and failed on any real config file (every project `.kern/*.toml`), silently disabling project-scope config. Fixed by parsing into `toml::Table`; the inline comment on `read_value` carries the load-bearing gotcha.
Second-pass migration:
- `merge_sections` rationale (compressed inline to 2 lines): the wholesale section replacement is intentional — a project either owns a section or it does not — but it surprises callers expecting a deep merge; a field the user set but the project omits is LOST, not inherited. Top-level keys present in only one scope are kept as-is (covered by load_layered_keeps_sections_present_in_only_one_scope).
- File header trimmed: section-level merge summary ("project TOML overrides whole sections; missing sections fall through to user, then defaults") now lives only on `merge_sections`.
- Deleted as duplicates: the test doc on read_value_parses_leading_section_header (restated the inline `read_value` comment, which now points at the test) and the setup comment in save_then_load_round_trips_and_creates_parent_dirs (restated the test name and assert message).
