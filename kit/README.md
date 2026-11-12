# rounds in this project

This directory holds the review threads between the writer agent and its reviewer agents (by
default Claude Code writes and Codex reviews; `reviewer` below changes the reviewer, `WRITER.md`
serves any writer), the tool that moves them, and its settings. Git ignores it (through `.git/info/exclude`),
so nothing here reaches a commit unless you decide otherwise.

## The loop

1. The writer opens a thread and starts its reviewer, from the checkout or worktree it works in:
   `.review/bin/review start <thread> --title "..." < ask.md`.
   The ask says what changed, why, and what to check. The tool records the checkout, its revision
   and its dirty tree in the request, and starts a reviewer session for this thread alone.
2. The reviewer takes the round (`review next --thread <thread> --wait`), reviews, runs the checks,
   and answers: `review respond <thread> --verdict accept|changes|human < response.md`.
3. The writer waits for the answer: `review wait <thread> --timeout 0` in the background (no time
   limit), or `review wait <thread>` in the foreground (590 seconds, then exit 3).
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

Git ignores `.review` (through `.git/info/exclude`), so editors and file pickers that hide ignored
files do not show it. `review open <thread>` prints a thread's folder and its newest file;
`review status` lists the threads, newest activity first, with the time of that activity and the
time the reviewer spent on them. A suspend of the machine does not count when a wait (the writer's
`review wait` or the reviewer's `review next --wait`) was running at the time: the waits note
each suspend in a `.suspends` file, in `threads/` or in the thread's folder.

`review checks` runs the configured `checks` (or `review checks -- <command>`) under one lock for the
whole kit, a file in /tmp that both sandboxes can write: with several reviewers and writers at work,
the full checks run one at a time.

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

## The reviewer agent

`reviewer` in `config.json` picks the agent, its model and extra arguments:

    "reviewer": { "agent": "codex", "model": null, "command": null, "args": [] }

- `agent`: a preset, `codex` (the default) or `claude`, or a name defined under `agents`.
- `model`: passed to the agent (`--model` in the presets); `null` leaves the agent's default.
- `command`: another executable for the agent (a path, a wrapper script).
- `args`: extra arguments, where the template says `{args}`.

`agents` defines an agent, or overrides a preset's fields:

    "agents": {
      "mine": {
        "command": "my-agent",
        "interactive": ["{command}", "--model", "{model}", "{args}", "{prompt}"],
        "headless": ["{command}", "run", "{prompt}"],
        "sandbox": "wrap",
        "writable": ["~/.my-agent"],
        "herdr_kind": null
      }
    }

- `interactive`: the command line in a terminal (a tab, a window); `headless`: without one (the
  `headless` launcher; the interactive one when missing). The agent must start working on
  `{prompt}`, its first message, and keep running its tools until the thread is over.
- Template elements: `{command}`, `{prompt}`, `{args}` (spliced), `{model}` (without a model, it
  goes, and so does the option just before it), `{codex_sandbox}` (Codex's profile arguments), and
  `{thread}`, `{thread_dir}`, `{cwd}` inside any element.
- `sandbox`: `codex`, `wrap` or `none` (below).
- `writable`: the agent's own state outside the project (its config, sessions, credentials
  refresh), writable inside the `wrap` sandbox.
- `herdr_kind`: the agent kind Herdr knows, to start it as a named agent there (only when not
  wrapped).

## The sandbox

The project is read-only for the reviewer; it may write only its thread directory, the paths in
`sandbox.writable_in_project` (build outputs the checks need, such as `target` or
`node_modules/.cache`), `sandbox.writable_outside` (caches), and the temporary directories. Two
ways to hold it there:

- `codex`: Codex's own sandbox, from a permission profile on its command line (nothing changes in
  `~/.codex/config.toml`), approvals off (`-a never`): a refused command comes back to Codex as an
  error. `sandbox.network` opens or closes its network.
- `wrap`: an operating-system sandbox around the agent, for any agent: bubblewrap on Linux
  (`bwrap`, in every distribution's packages), sandbox-exec on macOS. The disk is read-only, then
  the writable paths above and the agent's `writable` are mounted writable, the project read-only
  again (it may sit under /tmp), the thread directory and the build outputs writable over it. The
  network stays open (the agent needs its provider). Inside, the agent runs its tools without
  asking (the `claude` preset allows Bash, Read, Grep, Glob, Write and Edit); it cannot change the
  project.
- `none`: no sandbox; `review doctor` warns.

`review doctor` runs the configured reviewer's sandbox around a probe and checks both rules.

Codex asks once per project whether to trust the folder. The first interactive Codex reviewer
waits on that question in its tab until the user answers it; `review start` prints a `note:` line
and `review doctor` a `warn` line while Codex has not trusted the project (in its `config.toml`).
The `headless` launcher (`codex exec`) does not ask.

Claude Code asks once whether to trust a folder it has not seen. An interactive `claude` reviewer
in a new checkout (a new worktree) waits on that question in its tab: answer it there, or use the
`headless` launcher, which does not ask.

## External reviews

`review start <thread> --pr <ref>` reviews someone else's change (a pull request, a merge request).
The writer runs it from a worktree with the change checked out. The reviewer reviews the change
against its base, the writer checks each finding against the code and answers it in the next
round, and the reviewer accepts when the findings are settled. Then:

- `review draft <thread> < text` stores the comment for the author as `draft.md` in the thread,
  with `reviewed by 🎾rounds` at the bottom, and prints it as it will be posted.
- `review post <thread>` posts the draft through the platform's command line, as the user's
  account: a plain comment review, never an approval or a request for changes. It posts once per
  thread (`--again` posts it once more) and logs each post in `threads/.posts`. The writer runs it
  only when the user says so. The reviewer cannot: `review post` must write the log in the store,
  and the reviewer's sandbox keeps the store read-only.

`review status` marks external threads `[pr <ref>]`, then `[pr <ref>, draft]` and
`[pr <ref>, posted]`.

The platform is `platform` in `config.json`: `github` (the default, `gh pr review <ref> --comment
--body-file <draft>`) or `gitlab` (`glab mr note <ref> --message <text>`). Any other is a command
and a `post` template in `platforms`, with the elements `{command}`, `{pr}`, `{body_file}` and
`{body}`:

```json
"platform": "forge",
"platforms": {
  "forge": { "command": "forge", "post": ["{command}", "comment", "{pr}", "--file", "{body_file}"] }
}
```

A thread keeps the platform it started with. `review doctor` shows the platform and whether its
command is installed.

## Settings

`config.json`:

- `launcher`: `auto`, or one of the launchers above.
- `reviewer`, `agents`: the reviewer agent (above).
- `read_first`: files the reviewer reads before the first round (the project's agent instructions).
- `checks`: the commands the reviewer runs when code changes.
- `sandbox`: see above.
- `max_rounds`: the round after which the writer escalates.
- `platform`, `platforms`: where external reviews are posted (above).
- `stance`: `independent` (the default) or `adversarial`. An adversarial reviewer assumes the
  change is wrong, tries to break every claim, and gives each finding a concrete failure scenario
  (the section "An adversarial review" in `REVIEWER.md`). `review start <thread> --stance
  adversarial` sets it for one thread. A thread keeps the stance it started with, for every
  reviewer it gets.
- `focus`: `general` (the default) or `security`. A security reviewer looks for what an attacker
  can do with the change, and gives each finding the attacker, the input, the result and a
  severity (the section "A security review" in `REVIEWER.md`). `review start <thread> --focus
  security` sets it for one thread. The focus goes with either stance, and a thread keeps it for
  every reviewer it gets.

Environment overrides: `ROUNDS_STANCE`, `ROUNDS_FOCUS`, `ROUNDS_REVIEWER` (the agent), `ROUNDS_REVIEWER_MODEL`,
`ROUNDS_REVIEWER_COMMAND`, `ROUNDS_REVIEWER_ARGS`, `ROUNDS_LAUNCHER`, `ROUNDS_SPACE` (the Herdr app
space, from `herdr-app ls`), `ROUNDS_SANDBOX` (another bwrap or sandbox-exec), `ROUNDS_THREADS`,
`ROUNDS_POLL` (poll every five seconds instead of waking on file events), `ROUNDS_PLATFORM`,
`ROUNDS_PLATFORM_COMMAND` (another `gh` or `glab`).

## Commands

    review start <thread> [--title T] [--files a,b] < ask     open a thread with its own reviewer
                 [--stance adversarial] [--focus security] [--pr REF]
    review request <thread> [--title T] [--files a,b] < ask   the next round
    review wait <thread> [--timeout S]                        the writer's wait (0: no limit)
    review next --thread <thread> [--wait] [--timeout S]      the reviewer's wait
    review respond <thread> --verdict accept|changes|human < body
    review escalate <thread> "question"
    review close <thread> "reason"
    review draft <thread> < text                              an external review's comment, signed
    review post <thread> [--again]                            post it (when the user says so)
    review reviewer <thread> [--stop]                         start, restart or stop a reviewer
    review status                                             every thread
    review show <thread>                                      one thread, rounds in order
    review open <thread>                                      its folder and newest file
    review checks [-- command...]                             the checks, one run at a time
    review doctor                                             check the setup and the sandbox
    review selftest                                           one real round, end to end
    review uninstall [--purge]                                remove the kit (the threads stay unless --purge)

`review request` without `start` sends a thread to a shared reviewer instead: one long reviewer
session started with `SHARED.md`, which takes rounds with `review next --wait`. Use it when you
want one reviewer for many small threads.
