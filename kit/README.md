# rounds in this project

This directory holds the review threads between the writer agent (Claude Code) and its reviewers
(Codex), the tool that moves them, and its settings. Git ignores it (through `.git/info/exclude`),
so nothing here reaches a commit unless you decide otherwise.

## The loop

1. The writer opens a thread and starts its reviewer, from the checkout or worktree it works in:
   `.review/bin/review start <thread> --title "..." < ask.md`.
   The ask says what changed, why, and what to check. The tool records the checkout, its revision
   and its dirty tree in the request, and starts a Codex session for this thread alone.
2. The reviewer takes the round (`review next --thread <thread> --wait`), reviews, runs the checks,
   and answers: `review respond <thread> --verdict accept|changes|human < response.md`.
3. The writer waits for the answer: `review wait <thread>` (run it in the background).
   - exit 0, `accept`: done. The reviewer's session ends.
   - exit 10, `changes`: the writer fixes what the response lists, then opens the next round with
     `review request <thread>`, answering every finding (fixed, or not fixed and why).
   - exit 20, `human`: the loop stops. The writer shows the question to the human and waits. The
     reviewer stays open; the writer opens the next round after the human answers.
   - exit 3: the wait timed out; run it again.
4. Either side can stop the loop for the human: `review escalate <thread> "the question"`.
5. A thread whose subject moved elsewhere is closed by the writer: `review close <thread> "why"`.

After `max_rounds` rounds (config, default 6) on one thread, escalate: the loop is not converging.

## Files

A thread is `.review/threads/<YYYYMMDD>-<thread>/`: `01-request.md`, `01-response.md`,
`02-request.md`, ...; `NN-stop.md` for an escalation, `NN-closed.md` for a close,
`NN-notes-<topic>.md` for the reviewer's longer notes, `state.json` for the status (published by
rename, so a reader never sees a part of it). Commands take the bare thread name.

In a worktree, the tool links `.review` to the main checkout's directory, so every thread
lives in one store and `review status` lists them all.

## Where the reviewer runs

`launcher` in `config.json` picks it; `auto` takes the first that fits:

| Launcher | When | What you see |
|---|---|---|
| `herdr-app` | the writer runs in a terminal of the Herdr app (`HC_TERMINAL_ID` is set) | a tab `review <thread>` in the writer's space; `hc read rv-<thread>` |
| `herdr` | the writer runs in a Herdr pane (`HERDR_ENV=1`) | a tab `review <thread>` with the agent `rv-<thread>` |
| `tmux` | the writer runs inside tmux | a window `review-<thread>` |
| `command` | `command_launcher.start` is set | whatever your manager does with it |
| `headless` | always | no terminal: `codex exec` in the background, its log in the thread directory |
| `manual` | only when set | the tool prints the command; you run it in a terminal |

`command_launcher` templates take `{name}` (`rv-<thread>`), `{thread}`, `{cwd}`, `{label}` and
`{command}` (the whole reviewer command line, quoted). `start` must return at once; `alive` exits 0
while the reviewer runs; `stop` ends it. Example for Zellij:

    "command_launcher": {
      "start": "zellij run --name {name} --cwd {cwd} -- sh -c {command}",
      "stop": null,
      "alive": null
    }

## The sandbox

The reviewer runs Codex with a permission profile on its command line (nothing changes in
`~/.codex/config.toml`): the disk readable, the project read-only, writable only its thread
directory, the paths in `sandbox.writable_in_project` (build outputs the checks need, such as
`target` or `node_modules/.cache`), `sandbox.writable_outside` (caches), and the temporary
directories. `sandbox.network` opens the network (local services, Docker, package downloads).
Approvals are off (`-a never`): a refused command comes back to Codex as an error.

## Settings

`config.json`:

- `launcher`: `auto`, or one of the launchers above.
- `reviewer.command`, `reviewer.args`: the Codex executable and extra arguments (`["-m", "..."]`).
- `read_first`: files the reviewer reads before the first round (the project's agent instructions).
- `checks`: the commands the reviewer runs when code changes.
- `sandbox`: see above.
- `max_rounds`: the round after which the writer escalates.

Environment overrides: `ROUNDS_LAUNCHER`, `ROUNDS_SPACE` (the Herdr app space, from
`herdr-app ls`), `ROUNDS_CODEX`, `ROUNDS_CODEX_ARGS`, `ROUNDS_THREADS`,
`ROUNDS_POLL` (poll every five seconds instead of waking on file events).

## Commands

    review start <thread> [--title T] [--files a,b] < ask     open a thread with its own reviewer
    review request <thread> [--title T] [--files a,b] < ask   the next round
    review wait <thread> [--timeout S]                        the writer's wait
    review next --thread <thread> [--wait] [--timeout S]      the reviewer's wait
    review respond <thread> --verdict accept|changes|human < body
    review escalate <thread> "question"
    review close <thread> "reason"
    review reviewer <thread> [--stop]                         start, restart or stop a reviewer
    review status                                             every thread
    review show <thread>                                      one thread, rounds in order
    review doctor                                             check the setup and the sandbox
    review selftest                                           one real round, end to end

`review request` without `start` sends a thread to a shared reviewer instead: one long Codex
session started with `SHARED.md`, which takes rounds with `review next --wait`. Use it when you
want one reviewer for many small threads.
