# kern dev container.
# Full toolchain to build, run, profile, and operate the kern daemon
# plus the Claude Code CLI.

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    TERM=xterm-256color \
    CARGO_TARGET_DIR=/work/target-linux \
    RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:/usr/local/bin:/usr/bin:/bin

# Build, debug, profile, and ops tooling.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl git build-essential pkg-config \
        libssl-dev cmake clang lld \
        gdb lldb strace ltrace \
        heaptrack valgrind linux-tools-generic \
        ripgrep fd-find vim tmux less jq \
        sqlite3 \
        gnupg xz-utils sudo \
    && rm -rf /var/lib/apt/lists/*

# Node 20 + Claude Code CLI.
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g @anthropic-ai/claude-code

# GitHub CLI.
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

# Rust stable.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain stable --profile minimal \
    && rustup component add rustfmt clippy rust-src \
    && cargo install --locked just

# perf binary lives under /usr/lib/linux-tools/<ver>/perf — symlink to PATH.
RUN ln -sf $(ls /usr/lib/linux-tools/*/perf 2>/dev/null | head -1) /usr/local/bin/perf || true

WORKDIR /work
CMD ["/bin/bash"]
