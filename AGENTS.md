# rounds

If the user pointed you at this repository to set up rounds in their project, follow
[INSTALL.md](INSTALL.md) and nothing below.

## Working on rounds itself

One Rust binary. `rounds install` copies itself into a project as `.review/bin/review`,
beside the kit texts it carries, and the same binary then runs the review commands there.

| Path | What it is |
|---|---|
| `src/commands.rs` | The review commands: rounds, waits, verdicts, the reviewer's lifecycle, `doctor`, `selftest`. |
| `src/agent.rs` | The reviewer agent: the presets (`codex`, `claude`), the command templates, the model, Codex's permission profile and the `wrap` sandbox (bubblewrap, sandbox-exec). |
| `src/launcher.rs` | The launchers (Herdr app, Herdr, tmux, command templates, headless, manual). |
| `src/install.rs` | `rounds install` (and upgrade), and `review uninstall`. |
| `src/store.rs`, `src/watch.rs`, `src/config.rs` | The thread store, the file watch that wakes waits, the settings. |
| `kit/` | The texts the installer writes into a project: the protocol (`README.md`) and the reviewer prompts. |
| `kit/WRITER.md`, `skill/header.md` | The writer's instructions, for any agent; the installer adds the header to make the skill for Claude Code and Codex. |
| `install.sh` | The one-step installer: downloads the release binary for the platform and runs `rounds install`. |
| `examples/fake.rs` | The test double for the agents, the sandbox and the managers; the tests link it as `codex`, `claude`, `my-agent`, `bwrap`, `hc`, `herdr-app`, `herdr`, `tmux`. |
| `tests/` | Black-box tests: each one installs the real binary into a fresh git project and drives it. |
| `docs/method.md` | Why the method works the way it does. |
| `assets/logo.svg` | The logo: an 8-bit tennis racket and ball, 32×32 pixels as SVG rectangles (`shape-rendering="crispEdges"`). |

Rules:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass before a
  change is done; CI runs them on Linux.
- No `unsafe` (the crate forbids it). Few dependencies: `serde_json`, `shlex`, `notify`.
- A launcher implements `Launcher` (`start`, `alive`, `stop`) in `src/launcher.rs` and is named in
  `NAMES` and `launcher_for`; teach `examples/fake.rs` its manager and add tests in
  `tests/reviewers.rs`.
- A reviewer agent preset is an entry in `presets()` in `src/agent.rs` (a command, two templates,
  a sandbox mode); add tests in `tests/agents.rs`. Any agent other than Codex runs `wrap`: never
  give an agent a permission bypass flag outside a sandbox.
- The reviewer never changes the project: a change to Codex's permission profile or to the `wrap`
  sandbox keeps the project read-only, and `review doctor` checks it.
- A release is a tag `vX.Y.Z` on main (bump `Cargo.toml` first); `.github/workflows/release.yml`
  builds and publishes the binaries `install.sh` downloads.
- Keep the documents product-neutral: no company, product or project names other than the tools
  rounds drives.
