<p align="center">
  <img src="assets/logo.svg" width="160" height="160" alt="rounds: an 8-bit tennis racket and ball">
</p>

<h1 align="center">rounds</h1>

Review rounds between two coding agents, on any git project: one agent writes, another reviews.
Each review gets its own reviewer session, which reads the change, runs the project's checks and
answers with a verdict: `accept`, `changes`, or `human`. It stays open round after round and ends
when it accepts. It cannot change your code: its sandbox lets it write only its own review notes.

The two agents talk through plain files in the project (`.review/threads/`), so every review is a
readable record: what was asked, what was found, what was fixed, who decided. Any agent and any
model can take either role; see [Agents](#agents).

## Install

Point your agent at this repository, from inside your project:

> Set up rounds in this project: https://github.com/oscillatingworks/rounds

The agent follows [INSTALL.md](INSTALL.md): it installs the kit, sets the project's checks, runs a
health check, and runs one real review end to end. Then it tells you what it did.

To do it by hand, from your project:

```sh
curl -fsSL https://raw.githubusercontent.com/oscillatingworks/rounds/main/install.sh | sh
```

On Linux (x86_64 or arm64) it downloads one static binary, checks its checksum, and installs the
kit. On macOS it builds the binary with cargo, so it needs [Rust](https://rustup.rs) there (release
binaries for macOS come later). With Rust installed, `cargo install --git
https://github.com/oscillatingworks/rounds` and `rounds install` work too.

You need git, and the CLIs of the agents you use, logged in to your accounts. With the defaults:
[Claude Code](https://claude.com/claude-code) as the writer and the
[Codex CLI](https://github.com/openai/codex) as the reviewer.

## Personal by default

Setting up rounds changes nothing in your repository. The kit lives in `.review/`, which git
ignores through `.git/info/exclude` (a local file that is never pushed). The writer's skill lives
in your home folder. Your teammates see no new files, and your reviews stay on your machine. Each
person who wants reviews sets it up for themselves.

To share the setup with the team instead, install with `--commit` and commit `.review` (add
`.review/threads/` to `.gitignore` if the threads should stay personal).

## Use

Ask your agent for a review when a change is ready ("get this reviewed"), or make it a project
rule. The writer opens a thread and a reviewer starts beside it. Then:

```
writer: review start fix-login-timeout < ask.md     reviewer: takes round 1, runs the checks
writer: review wait fix-login-timeout               reviewer: responds: changes (2 findings)
writer: fixes both, review request ...              reviewer: takes round 2
writer: review wait ...                             reviewer: responds: accept, and its session ends
```

When the reviewer says `human` (a contract change, a cost, a risk, a disagreement it cannot
settle), the writer stops and shows you the question. The reviewer waits for your answer, which the
writer brings in the next round.

`.review/bin/review status` lists every thread; `review show <thread>` prints one.

## Agents

The two roles are separate, and either can be any agent with any model. The defaults:

| Role | Default | What it needs |
|---|---|---|
| Writer | Claude Code | To run shell commands. It follows `.review/WRITER.md`, installed as a skill for Claude Code and for Codex. |
| Reviewer | Codex | A CLI that takes a first message. rounds starts it, sandboxes it, and stops it. |

**Another writer.** Any agent that can run commands can write: Codex (it gets the skill too),
Gemini CLI, opencode, or your own. Point it at `.review/WRITER.md` (for example, one line in the
project's agent instructions).

**Another reviewer.** Set it in `.review/config.json`:

```json
"reviewer": { "agent": "claude", "model": "opus" }
```

`agent` is a preset (`codex` or `claude`) or one you define; `model` is passed to the agent;
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

**The sandbox follows the agent.** Codex confines itself with a permission profile rounds passes
on its command line (`"sandbox": "codex"`). Every other agent runs inside an operating-system
sandbox rounds builds around it (`"sandbox": "wrap"`: bubblewrap on Linux, sandbox-exec on macOS),
with the same rules: the project read-only, writable only the thread directory, the build outputs
the checks need, the caches, the temporary directories, and the agent's own state (`writable`).
Inside it, the agent may run its tools without asking, because it cannot change the project.
`review doctor` proves the sandbox of the configured reviewer. The kit's
[README](kit/README.md) has every setting.

## Where the reviewer runs

rounds works with any agent manager, and knows some of them first-hand:

| You run the writer in | The reviewer appears as |
|---|---|
| the [Herdr](https://herdr.dev) app | a tab `review <thread>` in the same space |
| Herdr (the terminal multiplexer) | a tab `review <thread>` |
| tmux | a window `review-<thread>` |
| anything else | a process in the background (the agent's headless mode), its log in the thread |

Any other manager (Zellij, WezTerm, kitty, screen, a web dashboard) plugs in through three shell
templates in `.review/config.json`: start, alive, stop. See the kit's [README](kit/README.md).

## What it guarantees

- **The reviewer cannot change your project.** It runs in a sandbox (the agent's own, or one
  rounds builds around it): the project is read-only, writable are only its thread directory, the
  build outputs your checks need (`target/`, `node_modules/.cache`, ...), caches and temporary
  directories. `review doctor` proves it on your machine; `review selftest` proves it with a real
  reviewer.
- **One reviewer per review.** No shared queue, no reviewer juggling five subjects. A thread is one
  subject; its reviewer keeps the standard from round to round.
- **Nothing leaves your machine** beyond what your agents already send to their providers. The
  threads stay in your project, ignored by git, unless you decide to commit them.
- **Worktrees work.** Start a review from any worktree; the reviewer runs in that worktree, the
  threads stay in one store in the main checkout.
- **The human decides what is the human's.** A `human` verdict stops the loop until you answer.

## The method

Why one reviewer per thread, why files, why a second model: [docs/method.md](docs/method.md).

## Remove

```sh
.review/bin/review install --uninstall           # keeps the threads
.review/bin/review install --uninstall --purge   # removes them too
```

## License

MIT. See [LICENSE](LICENSE).
