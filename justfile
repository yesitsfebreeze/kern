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
    rm -rf .kern/bin .kern/intake .kern/data .kern/*.log
    rm -rf site traces

[windows]
clean:
    cargo clean
    -Remove-Item -Recurse -Force .relay, .mesh, .git-fs, .machine
    -Remove-Item -Recurse -Force ".kern\bin", ".kern\intake", ".kern\data"
    -Remove-Item -Force ".kern\*.log"
    -Remove-Item -Recurse -Force "site", "traces"

[windows]
kill:
    -taskkill /IM kern.exe /F 2>$null

[unix]
kill:
    -pkill -f kern

docs:
    mkdocs build --strict

docs-serve:
    mkdocs serve

docs-deploy:
    mkdocs gh-deploy --force

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




