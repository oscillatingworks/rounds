//! Golden reviews: the frontier model at a high effort, until the thread is accepted.

mod common;

use common::Env;
use serde_json::json;

const APP: &[(&str, &str)] = &[("HC_TERMINAL_ID", "t99")];

fn launches(env: &Env) -> Vec<Vec<String>> {
    env.calls()
        .into_iter()
        .filter(|c| c[..2] == ["hc", "run"])
        .collect()
}

fn after<'a>(argv: &'a [String], option: &str) -> Option<&'a str> {
    let i = argv.iter().position(|a| a == option)?;
    argv.get(i + 1).map(String::as_str)
}

/// Codex's effort: the `-c` that sets `model_reasoning_effort` (its sandbox uses `-c` too).
fn codex_effort(argv: &[String]) -> Option<&str> {
    argv.windows(2)
        .find(|w| w[0] == "-c" && w[1].starts_with("model_reasoning_effort="))
        .map(|w| w[1].as_str())
}

fn start(env: &Env, args: &[&str]) -> common::Out {
    env.run_with(args, Some("please review"), APP, &env.project)
}

fn status_line(env: &Env, id: &str) -> String {
    env.run(&["status"])
        .text
        .lines()
        .find(|l| l.starts_with(&format!("{id} ")))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_golden_review_runs_the_frontier_model_at_xhigh_until_the_thread_is_accepted() {
    let env = Env::new();
    let out = start(&env, &["start", "deep", "--golden"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(
        out.text
            .contains("(golden review, gpt-6-astra at xhigh effort)"),
        "{}",
        out.text
    );
    let first = &launches(&env)[0];
    assert_eq!(after(first, "--model"), Some("gpt-6-astra"));
    assert_eq!(
        codex_effort(first),
        Some("model_reasoning_effort=\"xhigh\"")
    );
    assert!(first.last().unwrap().contains("A golden review"));
    assert!(status_line(&env, "deep").contains("[golden]"));

    env.run(&["next", "--thread", "deep"]);
    env.respond("deep", "accept", "fine");
    let out = env.run_with(&["request", "deep"], Some("round two"), APP, &env.project);
    assert_eq!(out.code, 0, "{}", out.text);
    let second = &launches(&env)[1];
    assert_eq!(after(second, "--model"), None, "{second:?}");
    assert!(
        !second.iter().any(|a| a.contains("effort")),
        "the next review is a normal one: {second:?}"
    );
    assert!(!second.last().unwrap().contains("A golden review"));
    assert!(!status_line(&env, "deep").contains("golden"));

    let out = start(&env, &["start", "plain"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(!out.text.contains("golden"), "{}", out.text);
    assert!(!launches(&env)[2].iter().any(|a| a.contains("effort")));
}

#[test]
fn a_golden_review_keeps_its_reviewer_through_changes_and_human_verdicts() {
    let env = Env::new();
    start(&env, &["start", "deep", "--golden"]);
    for verdict in ["changes", "human"] {
        env.run(&["next", "--thread", "deep"]);
        env.respond("deep", verdict, "more");
        // Lose the reviewer, so the next round starts a new one.
        env.run(&["reviewer", "deep", "--stop"]);
        let out = env.run_with(&["request", "deep"], Some("again"), APP, &env.project);
        assert_eq!(out.code, 0, "{}", out.text);
    }
    let all = launches(&env);
    assert_eq!(all.len(), 3);
    for argv in &all {
        assert_eq!(after(argv, "--model"), Some("gpt-6-astra"), "{argv:?}");
    }
}

#[test]
fn a_thread_can_ask_for_a_golden_review_later_and_its_reviewer_gives_way() {
    let env = Env::new();
    start(&env, &["start", "later"]);
    env.run(&["next", "--thread", "later"]);
    env.respond("later", "changes", "more");
    let out = env.run_with(
        &["request", "later", "--golden", "--effort", "high"],
        Some("go deep"),
        APP,
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    let all = launches(&env);
    assert_eq!(all.len(), 2, "a golden reviewer replaces the normal one");
    assert_eq!(after(&all[1], "--model"), Some("gpt-6-astra"));
    assert_eq!(
        codex_effort(&all[1]),
        Some("model_reasoning_effort=\"high\"")
    );
}

#[test]
fn max_effort_only_when_asked_and_effort_only_with_golden() {
    let env = Env::new();
    let out = start(&env, &["start", "most", "--golden", "--effort", "max"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert_eq!(
        codex_effort(&launches(&env)[0]),
        Some("model_reasoning_effort=\"max\"")
    );

    let out = start(&env, &["start", "odd", "--effort", "max"]);
    assert_ne!(out.code, 0);
    assert!(
        out.text.contains("--effort goes with --golden"),
        "{}",
        out.text
    );

    let out = start(&env, &["start", "odd", "--golden", "--effort", "low"]);
    assert_ne!(out.code, 0);
    assert!(out.text.contains("unknown effort"), "{}", out.text);
    assert!(!env.thread_path("odd").join("state.json").exists());
}

#[test]
fn claude_reviews_golden_with_the_latest_fable_and_the_config_can_change_it() {
    let env = Env::new();
    env.config(|c| c["reviewer"] = json!({ "agent": "claude", "model": "opus" }));
    let claude = env.fakes.join("claude").to_string_lossy().to_string();
    let with = [
        ("HC_TERMINAL_ID", "t99"),
        ("ROUNDS_REVIEWER_COMMAND", claude.as_str()),
    ];
    let out = env.run_with(
        &["start", "deep", "--golden"],
        Some("x"),
        &with,
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    let argv = &launches(&env)[0];
    assert_eq!(after(argv, "--model"), Some("fable"));
    assert_eq!(after(argv, "--effort"), Some("xhigh"));
    assert!(argv.iter().any(|a| a.ends_with("/bwrap")), "still wrapped");
    assert!(!argv.iter().any(|a| a.contains("dangerously")));

    env.config(|c| c["golden"] = json!({ "model": "some-frontier", "effort": "high" }));
    env.run_with(
        &["start", "two", "--golden"],
        Some("x"),
        &with,
        &env.project,
    );
    let argv = &launches(&env)[1];
    assert_eq!(after(argv, "--model"), Some("some-frontier"));
    assert_eq!(after(argv, "--effort"), Some("high"));
    assert!(
        env.run(&["doctor"])
            .text
            .contains("golden: claude with some-frontier at high effort"),
    );
}

#[test]
fn another_agent_can_run_the_golden_reviews() {
    let env = Env::new();
    env.config(|c| c["golden"] = json!({ "agent": "claude" }));
    let out = start(&env, &["start", "deep", "--golden"]);
    assert_eq!(out.code, 0, "{}", out.text);
    let argv = &launches(&env)[0];
    assert!(
        argv.iter().any(|a| a.ends_with("/bwrap")),
        "claude runs wrapped: {argv:?}"
    );
    assert_eq!(after(argv, "--model"), Some("fable"));
    let doctor = env.run(&["doctor"]).text;
    assert!(doctor.contains("reviewer: codex"), "{doctor}");
    assert!(
        doctor.contains("golden: claude with fable at xhigh effort"),
        "{doctor}"
    );
}

#[test]
fn a_shared_thread_cannot_ask_for_a_golden_review() {
    let env = Env::new();
    let out = env.run_with(
        &["request", "shared", "--golden"],
        Some("x"),
        &[],
        &env.project,
    );
    assert_ne!(out.code, 0);
    assert!(
        out.text.contains("needs a thread with its own reviewer"),
        "{}",
        out.text
    );
    assert!(out.text.contains("(no thread shared yet)"), "{}", out.text);
    assert!(
        env.run(&["status"]).text.contains("no threads"),
        "nothing is recorded"
    );

    let out = env.run_with(&["request", "shared"], Some("x"), &[], &env.project);
    assert_eq!(out.code, 0, "{}", out.text);
    env.run(&["next"]);
    env.respond("shared", "changes", "more");
    let out = env.run_with(
        &["request", "shared", "--golden"],
        Some("y"),
        &[],
        &env.project,
    );
    assert_ne!(out.code, 0);
    assert!(
        out.text.contains("needs a thread with its own reviewer"),
        "{}",
        out.text
    );
    assert_eq!(env.state("shared")["round"], 1, "no round opened");
    assert!(env.state("shared").get("golden").is_none());
}

#[test]
fn the_old_reviewer_gives_way_before_the_golden_round_is_published() {
    // The old reviewer tries to take and answer the thread at the moment rounds checks it is
    // alive (the command launcher's `alive` hook). It must find no open round.
    let env = Env::new();
    let takeover = env.root.join("takeover");
    let starts = env.root.join("starts");
    let (t, s) = (takeover.display(), starts.display());
    env.config(|c| {
        c["command_launcher"] = json!({
            "start": format!("echo {{command}} >> {s}"),
            "stop": "true",
            "alive": format!(
                "if test -f {t}; then rm {t}; .review/bin/review next --thread later > {t}.taken; \
                 echo old | .review/bin/review respond later --verdict changes > {t}.answered 2>&1; fi; true"
            )
        });
    });
    env.run_with(&["start", "later"], Some("first"), &[], &env.project);
    env.run(&["next", "--thread", "later"]);
    env.respond("later", "changes", "fix");
    std::fs::write(&takeover, "").unwrap();
    let out = env.run_with(
        &["request", "later", "--golden"],
        Some("go deeper"),
        &[],
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(!takeover.exists(), "the alive hook ran");
    let state = env.state("later");
    assert_eq!(state["status"], "requested");
    assert_eq!(state["round"], 2);
    assert!(!env.thread_path("later").join("02-response.md").exists());
    let started = std::fs::read_to_string(&starts).unwrap();
    assert_eq!(started.lines().count(), 2, "{started}");
    assert!(
        started.lines().nth(1).unwrap().contains("gpt-6-astra"),
        "{started}"
    );
}
