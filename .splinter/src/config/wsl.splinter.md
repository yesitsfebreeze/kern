# splinter: src/config/wsl.rs

Why this module exists (measured): under WSL2 default NAT networking, Ollama runs as a Windows host process; the loopback default http://localhost:11434 resolves inside the WSL VM where nothing listens. Every embed call fails as a transient connect error, ingest leaves the job spooled forever, and the failure is invisible (no crash, no surfaced error) — just a graph that stays empty. Measured on this machine: 13 daemons, weeks of uptime, zero thoughts each.

Design constraints (kept tight in source):
- Rewrite the DEFAULT loopback URL only; an explicitly configured URL is the user's decision, never second-guessed.
- Must NOT be a blanket "on WSL rewrite to gateway" rule: mirrored-mode WSL2 and a Linux box running its own Ollama DO reach loopback. Hence probe loopback FIRST and only fall back to the host gateway when loopback is genuinely dead.

Implementation notes:
- PROBE_TIMEOUT 300ms: both ends are local (VM loopback, host across a virtual switch), so a live Ollama answers in single-digit ms; the timeout only bounds the dead case.
- split_host_port is deliberately tiny (no URL-parser dependency) because it only ever sees kern's own loopback defaults.
- in_wsl reads /proc/version, not WSL_DISTRO_NAME (kept in source rationale).
- host_gateway uses the route table, not resolv.conf (kept in source rationale).
