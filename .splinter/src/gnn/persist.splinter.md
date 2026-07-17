# splinter: src/gnn/persist.rs

Second-pass migration:
- `unmarshal_rejects_a_future_version_before_checking_params`: deleted narration. Contract recorded here — the empty `params` vec in that fixture is ALSO a count mismatch, so the test only proves anything because `unmarshal_weights` checks `version` before `params.len()`; keep that ordering if the function is refactored.
- `unmarshal_rejects_a_corrupt_data_length_without_panicking`: deleted narration (test name + assert message carry it). Fixture is right-version/right-count/right-shape with param 0's data vec one element short.
- Kept inline: the `WEIGHT_FILE_VERSION` bump contract, and the `copy_from_slice` PANIC trap (shape and data are independent serde fields, so a corrupt file can match shape yet carry a wrong-length data vec).
