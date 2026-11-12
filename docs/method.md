# The method

The rounds tool puts a review step between "the agent says it is done" and "the change is in". It is a
small set of rules, a file layout, and a tool that enforces both. This page says why each rule is
there.

## Two agents, two models

The writer (Claude Code by default) holds the context of the work: the plan, the conversation with
the human, the code it changed. The reviewer (Codex by default) holds none of it. It sees what the writer wrote in the
ask, and what the repository shows. That gap is the point: a reviewer that shares the writer's
context shares its blind spots. A second model adds a second set of habits: what one model
overlooks, the other often catches. Any two agents and models can take the two roles; a different
model in each role keeps that second set of habits.

## One reviewer per thread

A thread is one subject: one fix, one feature, one plan. Each thread gets its own reviewer session,
started when the thread opens and ended when the reviewer accepts.

- The reviewer's context holds one subject only. Round 3 is judged against rounds 1 and 2 of the
  same subject, not against whatever the previous thread was about.
- Reviews run in parallel. Five writers in five worktrees get five reviewers, none waiting for
  another.
- A reviewer that crashes or hangs costs one thread. `review request` starts a new one.
- The human sees each reviewer as its own tab or window and can watch it, stop it, or talk to it.

## Rounds and verdicts

A round is one request and one response. The response ends in one of three verdicts:

- `accept`: nothing blocking remains. The thread is done.
- `changes`: something must change. The writer fixes it and opens the next round, answering every
  finding: fixed, or not fixed and why. A settled point stays settled unless new code reopens it.
- `human`: the decision belongs to a person. The loop stops until the human answers.

The reviewer puts findings first, most severe first, each with a file and line, the defect in one
sentence, and how to verify or fix it. Then what it checked and found fine, so the writer knows
what was covered. No padding.

After six rounds on one thread the writer escalates: a loop that does not converge needs a person,
not a seventh round.

## Independent by default, adversarial on request

The reviewer is independent: another agent, in its own session, that knows only the ask and the
repository, runs the real checks, and cannot fix what it finds. It is not adversarial by default.
A reviewer that assumes every change is wrong finds more on a risky change, but on an everyday one
it raises doubts it cannot make concrete, and the loop takes more rounds for the same result. So
the default reviewer judges with an open mind and accepts when nothing blocking remains.

A thread can ask for the other stance (`--stance adversarial`, or `stance` in the config): the
reviewer assumes the change is wrong until the evidence shows otherwise, tries to break each claim,
and gives every finding a concrete failure scenario. Use it where a missed defect costs more than a
few extra rounds: data migrations, security boundaries, concurrency, anything hard to roll back.

A security review is a focus, not a stance (`--focus security`, or `focus` in the config). A
general reviewer asks whether the change does what it claims; a security reviewer asks what else
it lets someone do. It starts from the trust boundaries, goes through a fixed list (input,
injection, access, secrets, exposure, limits, dependencies), and states each finding as an attack
with a severity. The focus says what to look at and the stance how hard to try, so they combine:
a security review of a new login flow is worth the adversarial stance too.

## The human's decisions

The agents settle what the code and the project's rules can settle. They stop for the human on
what they cannot: a change to a contract or an interface others depend on, a cost, a risk, a change
to how something is measured or released, and a disagreement between writer and reviewer that a
response cannot settle. The question goes in the first line, so the human reads it first.

Nobody watches the reviewer's session. A question asked there would wait forever; a question is a
`human` verdict, which the writer shows to the human.

## The reviewer cannot change the project

The reviewer reviews; it does not fix. Its sandbox makes that a fact, not a request: the project is
read-only, and it can write only its own thread directory, plus the build outputs and caches the
checks need. A reviewer that could fix would blur who wrote what, and a review would stop being a
review. The writer fixes; the reviewer checks the fix.

The reviewer runs the checks itself (the tests, the linters, the builds), on the revision the
request records. "The tests pass" in a review means the reviewer saw them pass.

## Files as the protocol

Requests, responses, notes and state are files in the project. Either side can read the whole
history; a human can too, with any editor. The format survives a crashed session, a restarted
machine, a change of agent manager. State changes are published by rename, so no reader sees half
a state. Git ignores the files by default: the review record is local unless the team decides to
keep it.

## Someone else's change

An external review uses the same loop for a different reader. The reviewer finds; the writer, which
cannot change the author's code, checks every finding against it and disputes what does not hold.
Only the findings both agents settle reach the draft, so the author does not pay for an agent's
false alarm. The comment goes out under the user's name, so the user reads it first and posts it:
rounds never posts by itself, never approves, and signs what it helped write.

## What the writer puts in an ask

The reviewer knows only the ask and the repository. A good ask:

- says what changed and why, in a few lines;
- names the files, the commits, or the diff range;
- lists the claims the reviewer should verify ("the retry stops after three attempts", "no query
  runs without the tenant filter");
- says which checks to run, and anything they need (a service to start, a fixture to load);
- in a later round, answers every finding of the previous one.
