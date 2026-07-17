# splinter: src/bin/locomo_eval.rs


# src/bin/locomo_eval.rs — commentary

Thin CLI over `bench_support::locomo_run::run_eval`; all scoring and driving logic lives there.

Second-pass migration (from the `//!` and item docs):
- What the harness reports (moved out of the `//!`): per-category quality — F1 / ROUGE-L / LLM-judge, plus abstention on the adversarial category — retrieved-context size, and query latency. The `//!` keeps the pipeline shape and the CC BY-NC 4.0 "never bundled" trap.
- `resolve_dataset`: doc compressed to the precedence rule (`--dataset` > `$KERN_LOCOMO_PATH` > `eval/locomo10.json`). It is a pure function specifically so that precedence is unit-testable without touching the filesystem — guarded by `dataset_resolution_precedence`.
- The pre-flight `Path::exists` check exists to fail loudly with actionable guidance instead of a bare "file not found" surfacing deep in the loader; its comment was deleted because the two `eprintln!` lines beneath it say exactly that.
- clap `///` arg docs are `--help` output and were left intact.
# src/bin/locomo_eval.rs — commentary (migrated from CLI doc comments)

- Binary: LoCoMo eval harness (#36). Drives capture→distill→retrieve over the LoCoMo corpus with live ollama. Dataset is CC BY-NC 4.0 — supplied via a path, never bundled.
- Dataset resolution precedence (`resolve_dataset`): explicit `--dataset`, then `$KERN_LOCOMO_PATH`, then the default `eval/locomo10.json`.
- CLI args (help text removed from source): `--dataset` path to locomo10.json; `--url` ollama base URL; `--embed-model` embedding model tag; `--answer-model` answerer tag (kern's `reason` endpoint glues retrieved context); `--judge-model` LLM-judge tag (default `qwen2.5:7b`); `--samples` limit to first N dialogues (default all 10); `--max-qa` limit to first N QA probes per dialogue (default all); `--dedup` dedup cosine threshold at ingest (default 0.95); `--seed` sampling seed forwarded to ollama, vary across runs for error bars (default 0); `--json` emit machine-readable / CI-diffable report instead of the human table; `--output` write report to a file instead of stdout.
