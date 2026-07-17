# src/wire.rs — commentary

- `is_zero`: generic (`Default + PartialEq`) so one serde skip predicate serves every numeric wire field; replaced a former per-type `is_zero` / `is_zero_i64` pair.


Second-pass migration:
- Module/`WireError` docs compressed. Design rationale moved here: validation never silently saturates or coerces — bad inputs surface as structured errors so client bugs are loud, not hidden.
- `WireError` variant docs deleted (duplicated the `#[error]` messages). The old `InternalKindOnWire` doc named "`Document` and `Superseded`" as internal-only — stale: `Superseded` is not an `EntityKind` handled here; the internal-only set is Document/Question/Answer/Conclusion (see `validate_wire_kind`).
- `validate_wire_conf` doc compressed; acceptable range is inclusive `[WIRE_CONF_MIN, WIRE_CONF_MAX]` = [0.0, 1.0], NaN rejected.
- `validate_fact_source`: MCP entrypoint always pins source to `AGENT_SOURCE`; the guard exists to backstop future caller paths.
- `is_zero` doc deleted (restated the code); see existing note line above for why it is generic.
- `conf_inclusive_bounds_accepted` comment compressed to the one-line oracle.
