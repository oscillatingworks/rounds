# reviewright

Review rounds between two coding agents, on any git project: Claude Code writes, and a Codex
session reviews. Each review gets its own Codex session, which reads the change, runs the
project's checks and answers with a verdict: `accept`, `changes`, or `human`. It stays open
round after round and ends when it accepts. It cannot change your code: its sandbox lets it write
only its own review notes.

The two agents talk through plain files in the project (`.review/threads/`), so every review
is a readable record: what was asked, what was found, what was fixed, who decided.

## Install

Point your agent at this repository. In Claude Code, inside your project:

> Set up reviewright in this project: https://github.com/oscillatingworks/reviewright

The agent follows [INSTALL.md](INSTALL.md): it installs the kit, sets the project's checks, runs a
health check, and runs one real review end to end. Then it tells you what it did.

To do it by hand, from your project:

```sh
curl -fsSL https://raw.githubusercontent.com/oscillatingworks/reviewright/main/install.sh | sh
```

On Linux (x86_64 or arm64) it downloads one static binary, checks its checksum, and installs the
kit. On macOS it builds the binary with cargo, so it needs [Rust](https://rustup.rs) there (release
binaries for macOS come later). You need git, [Claude Code](https://claude.com/claude-code) and the
[Codex CLI](https://github.com/openai/codex), logged in to your accounts. With Rust installed,
`cargo install --git https://github.com/oscillatingworks/reviewright` and `reviewright install`
work too.

## Personal by default

Setting up reviewright changes nothing in your repository. The kit lives in `.review/`, which git
ignores through `.git/info/exclude` (a local file that is never pushed). The writer's skill lives
in your home folder. Your teammates see no new files, and your reviews stay on your machine. Each
person who wants reviews sets it up for themselves.

To share the setup with the team instead, install with `--commit` and commit `.review` (add
`.review/threads/` to `.gitignore` if the threads should stay personal).

## Use

Ask Claude for a review when a change is ready ("get this reviewed"), or make it a project rule.
Claude opens a thread and a Codex reviewer starts beside it. Then:

```
Claude: review start fix-login-timeout < ask.md     Codex: takes round 1, runs the checks
Claude: review wait fix-login-timeout               Codex: responds: changes (2 findings)
Claude: fixes both, review request ...              Codex: takes round 2
Claude: review wait ...                             Codex: responds: accept, and its session ends
```

When the reviewer says `human` (a contract change, a cost, a risk, a disagreement it cannot
settle), Claude stops and shows you the question. The reviewer waits for your answer, which Claude
brings in the next round.

`.review/bin/review status` lists every thread; `review show <thread>` prints one.

## Where the reviewer runs

reviewright works with any agent manager, and knows some of them first-hand:

| You run Claude in | The reviewer appears as |
|---|---|
| the [Herdr](https://herdr.dev) app | a tab `review <thread>` in the same space |
| Herdr (the terminal multiplexer) | a tab `review <thread>` with a named agent |
| tmux | a window `review-<thread>` |
| anything else | a process in the background (`codex exec`), its log in the thread |

Any other manager (Zellij, WezTerm, kitty, screen, a web dashboard) plugs in through three shell
templates in `.review/config.json`: start, alive, stop. See the kit's
[README](kit/README.md).

## What it guarantees

- **The reviewer cannot change your project.** Codex runs under a permission profile passed on its
  command line: the project is read-only, writable are only its thread directory, the build
  outputs your checks need (`target/`, `node_modules/.cache`, ...), caches and temporary
  directories. `review doctor` proves it on your machine; `review selftest` proves it with a real
  reviewer.
- **One reviewer per review.** No shared queue, no reviewer juggling five subjects. A thread is one
  subject; its reviewer keeps the standard from round to round.
- **Nothing leaves your machine** beyond what Claude and Codex already send to their providers.
  The threads stay in your project, ignored by git, unless you decide to commit them.
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
