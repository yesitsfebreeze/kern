set windows-shell := ["pwsh", "-NoLogo", "-NoProfile", "-Command"]

# Shared debug/test image — sibling repo: https://github.com/yesitsfebreeze/rustest
rustest_repo := "https://github.com/yesitsfebreeze/rustest"
rustest_dir  := justfile_directory() / ".." / "rustest"
image        := "rustest:latest"

default:
    @just --list

check:
    cargo check --workspace

build:
    cargo build

release:
    cargo build --release

# Launch the mux: spawns the agent pane, becomes the cwd singleton daemon
# (engine in-process), and registers/serves the kern MCP — all in one process.
run:
    cargo run --bin kern

# Headless daemon only (no TUI/panes) — for servers or background use.
daemon:
    cargo run --bin kern -- --daemon

test:
    cargo nextest run --workspace
    cargo test --doc --workspace

fmt:
    cargo fmt --all -- --check

fmt-fix:
    cargo fmt --all

clippy:
    cargo clippy --all-targets -- -D warnings

install: release
    cargo install --path . --force

uninstall:
    -cargo uninstall kern

[unix]
clean:
    cargo clean
    rm -rf .relay .mesh .git-fs .machine
    rm -rf .kern/bin .kern/capture .kern/data .kern/digest.md .kern/*.log
    rm -rf docs/book/book

[windows]
clean:
    cargo clean
    -Remove-Item -Recurse -Force .relay, .mesh, .git-fs, .machine
    -Remove-Item -Recurse -Force ".kern\bin", ".kern\capture", ".kern\data", ".kern\digest.md"
    -Remove-Item -Force ".kern\*.log"
    -Remove-Item -Recurse -Force "docs\book\book"

[windows]
kill:
    -taskkill /IM kern.exe /F 2>$null

[unix]
kill:
    -pkill -f kern

docs:
    cargo run --manifest-path {{justfile_directory()}}/../shared/Cargo.toml -p doc-gen -- --workspace {{justfile_directory()}} --out {{justfile_directory()}}/docs/book/src
    mdbook build docs/book

docs-watch:
    cargo run --manifest-path {{justfile_directory()}}/../shared/Cargo.toml -p doc-gen -- --workspace {{justfile_directory()}} --out {{justfile_directory()}}/docs/book/src --watch

docs-serve:
    mdbook serve docs/book

docs-check:
    mdbook test docs/book

# Build the shared rustest image (clone the sibling repo if it's missing).
[unix]
docker-build:
    test -d "{{rustest_dir}}" || git clone {{rustest_repo}} "{{rustest_dir}}"
    cd "{{rustest_dir}}" && just build

[windows]
docker-build:
    if (-not (Test-Path "{{rustest_dir}}")) { git clone {{rustest_repo}} "{{rustest_dir}}" }
    cd "{{rustest_dir}}"; just build

# Interactive shell in the debug/test container (this repo mounted at /work).
docker:
    docker run --rm -it --cap-add SYS_PTRACE --security-opt seccomp=unconfined -v "{{justfile_directory()}}":/work -w /work {{image}} /bin/bash

# Run the full test suite inside the container (installs nextest into the image's PATH if missing).
docker-test:
    docker run --rm --cap-add SYS_PTRACE --security-opt seccomp=unconfined -v "{{justfile_directory()}}":/work -w /work {{image}} sh -c 'command -v cargo-nextest >/dev/null || curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C /usr/local/bin; cargo nextest run --workspace && cargo test --doc --workspace'


# Tier-0 workload retrieval snapshot on the generated trace (recall@10, NDCG@10,
# latency p50/p95/p99, throughput, vector memory). Deterministic stub embedder —
# no Ollama needed. Quality numbers are reproducible; compare against
# docs/kern/bench-retrieval.md baselines before merging retrieval changes.
bench-workload: trace
    cargo run --release --features bench --bin retrieval_bench -- --trace traces/workload.json --all

# Measured repository snapshot: build, tests, code shape, oracle state.
# Every number comes from a run. --json for CI diffing, --skip-tests when cold.
insight *args:
    python3 scripts/insight.py {{args}}

# Regenerate the bench trace. Byte-identical for the same args, so it is
# generated rather than committed.
trace:
    @mkdir -p traces
    @test -f traces/workload.json || python3 scripts/gen_trace.py --docs 200 --queries 50 --name kern-ranking-fusion-v1 --out traces/workload.json
