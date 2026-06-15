set windows-shell := ["pwsh", "-NoLogo", "-NoProfile", "-Command"]

compose    := "docker compose -f docker/docker-compose.yml"
hunt_outdir := justfile_directory() / "docker" / "out"

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
    cargo test --workspace

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

clean:
    cargo clean

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

docker-build:
    {{compose}} build

docker:
    {{compose}} run --rm --remove-orphans dev

hunt-run SECS="300":
    HUNT_SECS={{SECS}} {{compose}} run --rm dev bash docker/hunt.sh

hunt-print:
    {{compose}} run --rm dev bash -c 'heaptrack_print $(ls -t docker/out/heaptrack.kern.*.gz | head -1) | less -R'

hunt-leaks:
    {{compose}} run --rm dev bash -c 'heaptrack_print --print-leaks $(ls -t docker/out/heaptrack.kern.*.gz | head -1) | head -60'

[windows]
hunt-clean:
    -Remove-Item -Recurse -Force "{{hunt_outdir}}"

[unix]
hunt-clean:
    rm -rf "{{hunt_outdir}}"
