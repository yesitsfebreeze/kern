# splinter: src/bin/locomo_eval.rs


# src/bin/locomo_eval.rs — commentary

Thin CLI over `bench_support::locomo_run::run_eval`; all scoring and driving logic lives there.

Second-pass migration (from the `//!` and item docs):
- What the harness reports (moved out of the `//!`): per-category quality — F1 / ROUGE-L / LLM-judge, plus abstention on the adversarial category — retrieved-context size, and query latency. The `//!` keeps the pipeline shape and the CC BY-NC 4.0 "never bundled" trap.
- `resolve_dataset`: doc compressed to the precedence rule (`--dataset` > `$KERN_LOCOMO_PATH` > `eval/locomo10.json`). It is a pure function specifically so that precedence is unit-testable without touching the filesystem — guarded by `dataset_resolution_precedence`.
- The pre-flight `Path::exists` check exists to fail loudly with actionable guidance instead of a bare "file not found" surfacing deep in the loader; its comment was deleted because the two `eprintln!` lines beneath it say exactly that.
- clap `///` arg docs are `--help` output and were left intact.
