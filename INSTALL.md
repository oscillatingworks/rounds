# Installing reviewright (for the agent)

The user pointed you at this repository to set up reviewright in their project. Do the whole setup
in one go, without stopping for confirmation, except where a step says to ask. At the end, report
what you did. Commands below assume the project is the current directory; use the project's path
when it is not.

## 1. Check the prerequisites

Run these and keep the results for the report:

```sh
git rev-parse --show-toplevel          # inside a git repository
codex --version                        # the Codex CLI
codex login status                     # logged in
```

- No git repository: stop and tell the user reviewright needs one.
- No Codex CLI: ask the user whether to install it (`npm install -g @openai/codex` or
  `brew install codex`), install it on a yes, then continue.
- Codex not logged in: ask the user to run `codex login` (it opens a browser), then continue.

## 2. Install it

From the project (any checkout or worktree of it; the kit goes to the main checkout):

```sh
curl -fsSL https://raw.githubusercontent.com/oscillatingworks/reviewright/main/install.sh | sh -s -- "$(pwd)"
```

The script downloads the `reviewright` binary for the platform, checks its SHA-256, and runs
`reviewright install` with the arguments after `--`. When no release binary fits the platform, it
builds one with cargo if cargo is installed; otherwise it says so, and you tell the user.

The defaults: the kit in `.reviewright/` of the main checkout, ignored by git through
`.git/info/exclude`; the writer's skill in `~/.claude/skills/reviewright/` (so every worktree has
it). Use `--skill project` when the user wants the skill inside the project, and `--commit` when
the user wants to commit `.reviewright` (then add `.reviewright/threads/` to `.gitignore` unless
the user wants the threads in history too). The installer ends with `review doctor`.

An existing install is upgraded in place: the tool and the prompts are replaced, `config.json`
and the threads are kept. `.reviewright/bin/review install` upgrades from the binary already
there; the script above upgrades to the latest release.

## 3. Set the project's checks

Edit `.reviewright/config.json` in the main checkout. The installer filled what it could detect;
you finish it by reading the project:

- `checks`: the commands that tell whether a change is good, in the order a reviewer should run
  them. Look at the CI configuration (`.github/workflows/`, `.gitlab-ci.yml`, ...), `Makefile`,
  `package.json` scripts, `bin/` and `script/` directories, and the project's agent instructions.
  Prefer the command CI runs. Example: `["bin/test", "npm run lint"]`.
- `read_first`: the project's agent instructions and contribution rules (`AGENTS.md`,
  `CLAUDE.md`, `CONTRIBUTING.md`, ...), the files a reviewer needs to judge a change.
- `sandbox.writable_in_project`: the paths inside the project the checks write to (build outputs,
  test caches, generated files), so the reviewer can run them. Keep it to outputs; never list
  source directories.
- `sandbox.writable_outside`: caches outside the project the checks write to (package manager and
  toolchain caches).
- `sandbox.network`: `true` (the default) lets checks reach local services, Docker and package
  registries. Set `false` only when the user asks.

## 4. Check and prove the setup

```sh
.reviewright/bin/review doctor      # every line ok
.reviewright/bin/review selftest    # one real review round; takes a minute or two
```

`selftest` starts a real Codex reviewer through the launcher `doctor` reported, has it try a write
to the project (which must fail), write a note in its thread, and accept. It removes its thread
afterwards. When a check fails, fix the cause (the doctor says what) and run it again. When the
`checks` you set need a build that writes outside the paths you listed, the reviewer's runs will
show it later: widen `writable_in_project` then.

## 5. Report

Tell the user, briefly:

- where the kit is, that git ignores it, and where the skill is;
- the launcher (`doctor` prints it) and where reviewers will appear;
- the checks you set;
- the result of `selftest`;
- how to use it: "ask for a review when a change is ready", or a line they can add to their
  project's agent instructions, for example: "Before a commit, get the change reviewed with
  reviewright and act on the verdict.";
- how to remove it: `.reviewright/bin/review install --uninstall`.

Do not commit anything unless the user asks.
