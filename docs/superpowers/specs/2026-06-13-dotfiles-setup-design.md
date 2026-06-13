# Dotfiles Setup — Design Spec

**Date:** 2026-06-13
**Status:** Approved (pending written-spec review)
**Location:** `E:\dev\kern\setup\` (chezmoi source root)

## Goal

A single folder that fully provisions a development environment across
**Windows, macOS, and Linux** ("roam Win + Mac/Linux"). Clone the repo, run one
bootstrap command, and the machine has a configured terminal, editor, shell, and
a curated set of CLI tools — all theme-matched and reproducible. Re-running the
sync command brings any machine up to date.

## Scope

In scope:

- **Terminal:** WezTerm — one Lua config, all three OSes; provides tabs/splits/multiplexing.
- **Multiplexer:** tmux — Unix only, scoped to **remote/SSH session persistence**
  (NOT the primary source of splits; WezTerm covers that, and is the multiplexer
  on Windows where tmux has no native build).
- **Editor:** Neovim — from-scratch Lua config with `lazy.nvim` plugin manager.
- **Shell:** Nushell — the single interactive shell on all three OSes.
- **CLI tools:** ripgrep, fd, fzf, bat, eza, zoxide, starship, git + delta,
  lazygit, gh, jq.
- **Deploy mechanism:** chezmoi — per-OS templating, native Win + Unix binary,
  single `chezmoi apply` everywhere.
- **Unified theme:** Catppuccin Mocha across WezTerm, nvim, starship, bat,
  lazygit, and delta.

Explicitly out of scope (deferred):

- `~/.claude` configuration management (settings, CLAUDE.md, agents, skills, hooks).
- kern binary install / MCP registration.

These were in the original ask but the user chose "pure setup for now." They can be
folded in later as additional chezmoi-managed trees without reworking the structure.

## Approach

**Approach B — Fully declarative chezmoi.** `setup/` is the chezmoi source root. A
per-OS package list (`.chezmoidata/packages.yaml`) drives `run_onchange_` scripts
that install everything via the native package manager. A tiny `bootstrap` handles
only the chicken-and-egg problem (installing chezmoi itself), then delegates to
`chezmoi apply` for everything else.

Rejected alternatives:

- **A — config-only chezmoi + separate bootstrap scripts:** two systems (imperative
  install scripts + chezmoi) to keep in sync. More drift risk.
- **C — chezmoi + Nix:** maximal reproducibility but steep ramp and poor Windows
  support; wrong fit for a roaming Win+Unix user.

## Repository Layout

```
setup/
  .chezmoiroot                       # contains "home" -> chezmoi treats home/ as source
  bootstrap.sh                       # Unix: install chezmoi, then chezmoi init --apply
  bootstrap.ps1                      # Windows: same, via winget/scoop fallback
  README.md                          # what this is, how to run it, how to sync
  home/
    .chezmoi.toml.tmpl               # one-time prompt: name + email; derives per-OS vars
    .chezmoiignore                   # templated: excludes tmux.conf on Windows
    .chezmoidata/
      packages.yaml                  # tool -> {winget, scoop, brew, apt, pacman} name map
    .chezmoitemplates/
      catppuccin-mocha.tmpl          # shared palette (hex values) reused by configs
    dot_config/
      wezterm/
        wezterm.lua                  # OS-conditional font/shell; Catppuccin Mocha
      nvim/
        init.lua
        lua/
          config/                    # options, keymaps, autocmds, lazy bootstrap
          plugins/                   # one file per plugin spec
      nushell/
        config.nu                    # aliases, tool init (zoxide, starship, fzf)
        env.nu                       # PATH, env, starship hook
      starship.toml                  # Catppuccin Mocha prompt
      bat/config                     # Catppuccin Mocha theme
      lazygit/config.yml             # Catppuccin Mocha + keys
      gh/config.yml
    dot_config/tmux/
      tmux.conf                      # Unix only (gated by run/template); session persistence
    dot_gitconfig.tmpl               # aliases, delta pager, name/email templated
    run_onchange_install-packages.sh.tmpl   # Unix: read packages.yaml, install via brew/apt/pacman
    run_onchange_install-packages.ps1.tmpl  # Windows: install via winget/scoop
```

Notes on chezmoi conventions:

- `dot_` prefix becomes a leading `.` at the destination (`dot_config` -> `~/.config`).
- `.tmpl` suffix enables Go-template processing (OS branching, injected name/email).
- `run_onchange_` scripts re-run only when their rendered content changes — so editing
  `packages.yaml` triggers a re-install pass; otherwise they are skipped.
- `.chezmoiroot` lets the repo keep `bootstrap`/`README` at top level while chezmoi
  uses `home/` as the actual source tree.

## Cross-OS Handling

- Templates branch on `.chezmoi.os` (`windows` / `darwin` / `linux`).
- WezTerm: conditional `default_prog` (Nushell exe path differs per OS) and font
  fallback; otherwise shared.
- Nushell: shared `config.nu`; `env.nu` sets OS-appropriate PATH entries.
- tmux: `tmux.conf` deploys only on Unix. A templated `.chezmoiignore` lists
  `.config/tmux/tmux.conf` when `.chezmoi.os` is `windows`, so chezmoi skips it there
  and WezTerm multiplexing stands in.
- Package install scripts: separate `.sh` (Unix) and `.ps1` (Windows); each reads the
  same `packages.yaml` and maps each tool to that platform's package name. A tool with
  no entry for the current OS is skipped with a logged note (no silent drop).

## Bootstrap Flow

1. User clones the repo (or runs the one-line bootstrap that clones it).
2. `bootstrap.sh` / `bootstrap.ps1`:
   - Detect OS / package manager.
   - Install chezmoi if absent (official installer on Unix; winget/scoop on Windows).
   - Run `chezmoi init --apply --source <repo>/setup`.
3. chezmoi prompts once for name + email (`.chezmoi.toml.tmpl`), persisted to local
   chezmoi config (not committed).
4. chezmoi renders templates to real destination paths.
5. `run_onchange_install-packages.*` installs/updates the CLI tools via the native
   package manager.
6. First `nvim` launch (or a headless sync step) installs plugins via lazy.nvim.

**Sync any machine:** `chezmoi apply` (or `chezmoi update` to pull + apply).

## Theme

Catppuccin Mocha is the single source of palette truth. Where an upstream Catppuccin
config exists (WezTerm builtin, bat theme, starship preset, lazygit, delta), use it.
The shared `catppuccin-mocha.tmpl` holds raw hex values for any config that needs
manual color references, so the palette is defined once.

Swapping themes later means changing the referenced Catppuccin flavor (and the shared
template), not touching each config.

## Acceptance Criteria

A machine is correctly provisioned when, after running bootstrap:

1. `wezterm --version`, `nvim --version`, `nu --version`, and every tool in the package
   list resolve on PATH.
2. `chezmoi apply --dry-run` reports no pending changes (idempotent).
3. `nvim --headless "+Lazy! sync" +qa` exits 0 with all plugins installed.
4. `nu -c "version"` loads `config.nu`/`env.nu` without error.
5. Starship prompt renders in Nushell; zoxide/fzf integrations are active.
6. WezTerm opens with Nushell as the default program and Catppuccin Mocha applied.
7. On Unix: `tmux.conf` is present at the expected path; on Windows: it is absent and
   WezTerm multiplexing is the documented substitute.
8. `git config --get core.pager` returns delta; theme matches.
9. Re-running bootstrap on an already-provisioned machine is a no-op (no errors, no
   duplicate work beyond chezmoi's change detection).

## Testing Strategy

- **Dry-run smoke:** `chezmoi apply --dry-run` per OS confirms template rendering and
  idempotency without mutating the machine.
- **Headless nvim:** plugin-sync verification as in acceptance criterion 3.
- **Nushell load:** `nu -c` config-load check (criterion 4).
- **Package-map lint:** a check that every tool in scope has at least one OS mapping in
  `packages.yaml`.
- Manual cross-OS verification on at least one Windows and one Unix machine before the
  setup is considered done.

## Open Questions / Future

- Folding `~/.claude` and kern back in (deferred by user choice).
- Secrets handling (chezmoi supports age/templated secrets) — not needed for current
  tool set; revisit if claude/kern API keys enter scope.
