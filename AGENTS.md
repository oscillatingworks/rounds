# reviewright

If the user pointed you at this repository to set up reviewright in their project, follow
[INSTALL.md](INSTALL.md) and nothing below.

## Working on reviewright itself

One Rust binary. `reviewright install` copies itself into a project as `.review/bin/review`,
beside the kit texts it carries, and the same binary then runs the review commands there.

| Path | What it is |
|---|---|
| `src/commands.rs` | The review commands: rounds, waits, verdicts, the reviewer's lifecycle, `doctor`, `selftest`. |
| `src/launcher.rs` | The Codex command line and its permission profile; the launchers (Herdr app, Herdr, tmux, command templates, headless, manual). |
| `src/install.rs` | `reviewright install`, upgrade and `--uninstall`. |
| `src/store.rs`, `src/watch.rs`, `src/config.rs` | The thread store, the file watch that wakes waits, the settings. |
| `kit/` | The texts the installer writes into a project: the protocol (`README.md`) and the reviewer prompts. |
| `skill/SKILL.md` | The writer's Claude Code skill. |
| `install.sh` | The one-step installer: downloads the release binary for the platform and runs `reviewright install`. |
| `examples/fake.rs` | The test double for Codex and the managers; the tests link it as `codex`, `hc`, `herdr-app`, `herdr`, `tmux`. |
| `tests/` | Black-box tests: each one installs the real binary into a fresh git project and drives it. |
| `docs/method.md` | Why the method works the way it does. |

Rules:

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass before a
  change is done; CI runs them on Linux and macOS.
- No `unsafe` (the crate forbids it). Few dependencies: `serde_json`, `shlex`, `notify`.
- A launcher implements `Launcher` (`start`, `alive`, `stop`) in `src/launcher.rs` and is named in
  `NAMES` and `launcher_for`; teach `examples/fake.rs` its manager and add tests in
  `tests/reviewers.rs`.
- The reviewer never changes the project: a change to the permission profile keeps the project
  read-only, and `review doctor` checks it.
- A release is a tag `vX.Y.Z` on main (bump `Cargo.toml` first); `.github/workflows/release.yml`
  builds and publishes the binaries `install.sh` downloads.
- Keep the documents product-neutral: no company, product or project names other than the tools
  reviewright drives.
