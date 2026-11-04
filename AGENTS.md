# rounds

If the user pointed you at this repository to set up rounds in their project, follow
[INSTALL.md](INSTALL.md). The reference below helps you answer their questions.

## Reference

### Requirements and platforms

`install.sh` downloads one static binary on Linux (x86_64 or arm64) and checks its checksum. On
macOS it builds the binary with cargo, so it needs [Rust](https://rustup.rs) there (release
binaries for macOS come later). With Rust installed, `cargo install --git
https://github.com/oscillatingworks/rounds` and `rounds install` work too.

The user needs git, and the CLIs of the agents they use, logged in to their accounts. With the
defaults: [Claude Code](https://claude.com/claude-code) as the writer and the
[Codex CLI](https://github.com/openai/codex) as the reviewer. A reviewer other than Codex needs
`bwrap` (the `bubblewrap` package) on Linux.

### Personal by default

Setting up rounds changes nothing in the repository. The kit lives in `.review/`, which git
ignores through `.git/info/exclude` (a local file that is never pushed). The writer's skill lives
in the user's home folder. Teammates see no new files, and the reviews stay on the user's machine.
Each person who wants reviews sets it up for themselves. rounds never posts to a pull request.

To share the setup with a team instead, install with `--commit` and commit `.review` (add
`.review/threads/` to `.gitignore` if the threads should stay personal).

### A review, round by round

```
writer: review start fix-login-timeout < ask.md     reviewer: takes round 1, runs review checks
writer: review wait fix-login-timeout --timeout 0   reviewer: responds: changes (2 findings)
writer: fixes both, review request ...              reviewer: takes round 2
writer: review wait ... --timeout 0                 reviewer: responds: accept, and its session ends
```

The writer drives its own thread, from the main checkout or any worktree: it opens it, waits,
fixes, opens the next round, and stops for the user on `human` (a contract change, a cost, a risk,
a disagreement the reviewer cannot settle). The reviewer waits for the answer, which the writer
brings in the next round. The protocol is in [kit/README.md](kit/README.md).

- **Waits.** `review wait <thread> --timeout 0` waits until the verdict, with a progress line every
  ten minutes; run it in the background. Without `--timeout`, it stops after 590 seconds with exit
  3 (for a shell tool with a time limit), and the writer runs it again.
- **Checks, one run at a time.** `review checks` runs the configured `checks` (or
  `review checks -- <command>`) under one lock for the whole kit, a file in /tmp that both
  sandboxes can write. With several reviewers and writers at work, heavy test suites do not run at
  the same time; the output says how long the run waited.
- **Finding the threads.** `review status` lists every thread, newest activity first, with the time
  of that activity, its age, and the review time (each round from request to response; it is clock
  time, so a laptop suspend counts). `review open <thread>` prints a thread's folder and its newest
  file, because editors hide `.review`. `review show <thread>` prints the whole thread.
- **The first Codex reviewer in a project.** Codex asks once per project whether to trust the
  folder, and the reviewer waits on that question in its tab until the user answers it.
  `review start` prints a `note:` line about it (the writer shows it to the user), and
  `review doctor` a `warn` line, until Codex trusts the project.
- **Independent or adversarial.** The default reviewer is independent: a separate agent in its
  own session, with an open mind. `review start <thread> --stance adversarial` (or
  `"stance": "adversarial"` in `.review/config.json` for every thread) makes it a skeptic: it
  assumes the change is wrong, tries to break every claim, and gives each finding a concrete
  failure scenario. A thread keeps its stance for every reviewer it gets; `review doctor` shows
  the configured one.
- **A new reviewer on an old thread.** A round on an accepted thread starts a new reviewer; its
  prompt tells it to read the earlier rounds in the thread folder first and keep their settled
  points.

### Agents and models

The two roles are separate, and either can be any agent with any model:

| Role | Default | What it needs |
|---|---|---|
| Writer | Claude Code | To run shell commands. It follows `.review/WRITER.md`, installed as a skill for Claude Code and for Codex. |
| Reviewer | Codex | A CLI that takes a first message. rounds starts it, sandboxes it, and stops it. |

**Another writer.** Any agent that can run commands can write: Codex (it gets the skill too),
Gemini CLI, opencode, or another. Point it at `.review/WRITER.md` (for example, one line in the
project's agent instructions).

**Another reviewer.** Set it in `.review/config.json`:

```json
"reviewer": { "agent": "claude", "model": "opus" }
```

`agent` is a preset (`codex` or `claude`) or one the user defines; `model` is passed to the agent;
`args` adds arguments. Any other agent is a command and two templates:

```json
"reviewer": { "agent": "gemini", "model": "gemini-2.5-pro" },
"agents": {
  "gemini": {
    "command": "gemini",
    "interactive": ["{command}", "--yolo", "--model", "{model}", "{args}", "--prompt-interactive", "{prompt}"],
    "headless": ["{command}", "--yolo", "--model", "{model}", "{args}", "--prompt", "{prompt}"],
    "sandbox": "wrap",
    "writable": ["~/.gemini"]
  }
}
```

Every setting, the template elements and the environment overrides are in
[kit/README.md](kit/README.md).

### The sandbox

The reviewer cannot change the project. The project is read-only; writable are only its thread
directory, the build outputs the checks need (`target/`, `node_modules/.cache`, ...), caches, the
temporary directories, and the agent's own state.

- Codex confines itself with a permission profile rounds passes on its command line
  (`"sandbox": "codex"`).
- Every other agent runs inside an operating-system sandbox rounds builds around it
  (`"sandbox": "wrap"`: bubblewrap on Linux, sandbox-exec on macOS). Inside it, the agent may run
  its tools without asking, because it cannot change the project.

`review doctor` proves the configured reviewer's sandbox on the machine; `review selftest` proves
it with a real reviewer.

### Where the reviewer runs

rounds works with any agent manager, and knows some of them first-hand:

| The writer runs in | The reviewer appears as |
|---|---|
| the [Herdr](https://herdr.dev) app | a tab `review <thread>` in the same space |
| Herdr (the terminal multiplexer) | a tab `review <thread>` |
| tmux | a window `review-<thread>` |
| anything else | a process in the background (the agent's headless mode), its log in the thread |

Any other manager (Zellij, WezTerm, kitty, screen, a web dashboard) plugs in through three shell
templates in `.review/config.json`: start, alive, stop. See [kit/README.md](kit/README.md).

### What it guarantees

- **The reviewer cannot change the project** (above).
- **One reviewer per review.** No shared queue, no reviewer juggling five subjects. A thread is one
  subject; its reviewer keeps the standard from round to round.
- **Nothing leaves the machine** beyond what the agents already send to their providers. The
  threads stay in the project, ignored by git.
- **Worktrees work.** A review starts from any worktree; the reviewer runs in that worktree, the
  threads stay in one store in the main checkout.
- **The human decides what is the human's.** A `human` verdict stops the loop until the user
  answers.

## Working on rounds itself

One Rust binary. `rounds install` copies itself into a project as `.review/bin/review`,
beside the kit texts it carries, and the same binary then runs the review commands there.

| Path | What it is |
|---|---|
| `src/commands.rs` | The review commands: rounds, waits, verdicts, the reviewer's lifecycle, `status`, `open`, `checks` (and its lock), `doctor`, `selftest`. |
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
| `assets/match.svg` | The README diagram: a review as an 8-bit tennis match, in the logo's palette, 440×328 pixels as SVG rectangles grouped by color; the text is a 5×7 pixel font drawn as rectangles too, so it looks the same everywhere. |

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
- rounds is for personal use: it does not post to pull requests or other shared places.
- The README stays short (what it is, the diagram, install, use, remove); details go here or in
  `kit/README.md`.
- A release is its own commit on main, `chore(release): vX.Y.Z`: it bumps `Cargo.toml`, and its
  message lists every change since the last release, grouped (features, fixes, docs), so the commit
  and the release page on GitHub show them in one place. Tag that commit `vX.Y.Z`;
  `.github/workflows/release.yml` builds the binaries `install.sh` downloads and uses the commit
  message as the release notes.
- Keep the documents product-neutral: no company, product or project names other than the tools
  rounds drives.
