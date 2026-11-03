# Reviews with rounds (the writer)

You are the writer: you change code and documents, and you ask for reviews. Each review thread
gets its own reviewer agent (Codex unless `reviewer.agent` in `.review/config.json` names another),
which reads, runs the project's checks, and answers with a verdict. It never changes code.

The tool lives in the project's main checkout, also when you work in a worktree:

```sh
R="$(git rev-parse --path-format=absolute --git-common-dir)/../.review/bin/review"
```

If `$R` does not exist, rounds is not set up in this project: say so in one line (it is set up by
pointing an agent at https://github.com/oscillatingworks/rounds) and stop. The protocol is in
`.review/README.md` of the main checkout.

## Open a review

1. Pick a thread name: lower-case words and digits joined by dashes, one subject per thread
   (`fix-login-timeout`, `plan-cache-layer`).
2. Write the ask to a file: what changed and why, the files and commits to look at, the claims the
   reviewer should verify, and the checks to run. Be specific: the reviewer knows only what you
   write and what the repository shows.
3. From the checkout or worktree you work in, run `"$R" start <thread> --title "<short title>" < ask.md`.
   It prints where the reviewer runs (a tab, a window, or a background process). Tell the user in
   one line.
4. Wait: `"$R" wait <thread> --timeout 3000`. Run it in the background when your harness can (its
   exit wakes you), and keep working on something else; otherwise run it in the foreground.

## Act on the verdict

- Exit 0, `accept`: the review is done. Tell the user in one line.
- Exit 10, `changes`: fix what the response lists. Then open the next round with
  `"$R" request <thread> < answer.md`, answering every finding: fixed (how), or not fixed (why).
  Do not reopen a settled point. Wait again.
- Exit 20, `human`: stop work on this thread. Show the question to the user word for word, with
  your recommendation, and wait for the answer. Then open the next round with the answer.
- Exit 3: the wait timed out; run it again.

After six rounds on one thread (`max_rounds` in `.review/config.json`), escalate with
`"$R" escalate <thread> "the question"` and show it to the user.

## Other commands

- `"$R" status`: every thread and its reviewer.
- `"$R" show <thread>`: the whole thread.
- `"$R" reviewer <thread>`: restart a reviewer that is gone; `--stop` ends it.
- `"$R" close <thread> "reason"`: close a thread whose subject moved elsewhere.
- `"$R" doctor`: check the setup when a reviewer does not start.
