//! The day-to-day commands: a wait without a limit, status times, open, the checks lock, and the
//! context a new reviewer gets on a thread with earlier rounds.

mod common;

use std::time::{Duration, Instant};

use common::Env;
use serde_json::json;

#[test]
fn a_wait_with_timeout_0_has_no_limit_and_ends_on_the_verdict() {
    let env = Env::new();
    env.request("alpha", "x");
    std::thread::scope(|scope| {
        let waiting = scope.spawn(|| env.run(&["wait", "alpha", "--timeout", "0"]));
        std::thread::sleep(Duration::from_millis(1500));
        assert!(
            !waiting.is_finished(),
            "timeout 0 is no limit, not an immediate timeout"
        );
        env.respond("alpha", "changes", "fix it");
        let out = waiting.join().unwrap();
        assert_eq!(out.code, 10, "{}", out.text);
    });
}

#[test]
fn status_lists_the_newest_activity_first_with_its_time_and_the_review_time() {
    let env = Env::new();
    env.request("older", "x");
    env.respond("older", "accept", "fine");
    std::thread::sleep(Duration::from_millis(1100));
    env.request("newer", "x");
    let out = env.run(&["status"]);
    assert_eq!(out.code, 0, "{}", out.text);
    let lines: Vec<&str> = out.text.lines().collect();
    assert!(lines[0].starts_with("newer"), "{}", out.text);
    assert!(lines[1].starts_with("older"), "{}", out.text);
    assert!(
        lines[0].contains("ago)") && lines[0].contains("review ") && lines[0].contains(", open"),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].contains("review ") && !lines[1].contains(", open"),
        "{}",
        lines[1]
    );
    assert!(lines[0].contains('Z'), "the time is UTC: {}", lines[0]);
}

#[test]
fn open_prints_the_thread_folder_and_its_newest_file() {
    let env = Env::new();
    env.request("alpha", "x");
    env.respond("alpha", "changes", "fix it");
    let out = env.run(&["open", "alpha"]);
    assert_eq!(out.code, 0, "{}", out.text);
    let lines: Vec<&str> = out.text.lines().collect();
    assert_eq!(lines[0], env.thread_path("alpha").to_str().unwrap());
    assert!(lines[1].ends_with("01-response.md"), "{}", out.text);
}

#[test]
fn checks_run_the_configured_commands_and_exit_with_the_first_failure() {
    let env = Env::new();
    env.config(|c| c["checks"] = json!(["echo first-check", "exit 3", "echo still-runs"]));
    let out = env.run(&["checks"]);
    assert_eq!(out.code, 3, "{}", out.text);
    assert!(out.text.contains("checks lock: waited"), "{}", out.text);
    assert!(
        out.text.contains("$ echo first-check\nfirst-check\nexit 0"),
        "{}",
        out.text
    );
    assert!(
        out.text.contains("still-runs"),
        "every check runs: {}",
        out.text
    );
    let out = env.run(&["checks", "--", "echo", "given command"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(out.text.contains("given command"), "{}", out.text);
}

#[test]
fn checks_run_one_at_a_time_across_callers() {
    let env = Env::new();
    let started = Instant::now();
    std::thread::scope(|scope| {
        let a = scope.spawn(|| env.run(&["checks", "--", "sleep", "1"]));
        let b = scope.spawn(|| env.run(&["checks", "--", "sleep", "1"]));
        assert_eq!(a.join().unwrap().code, 0);
        assert_eq!(b.join().unwrap().code, 0);
    });
    assert!(
        started.elapsed() >= Duration::from_millis(1900),
        "the two runs overlapped: {:?}",
        started.elapsed()
    );
}

#[test]
fn a_new_reviewer_on_a_thread_with_earlier_rounds_is_told_to_read_them() {
    let env = Env::new();
    let app = &[("HC_TERMINAL_ID", "t99")];
    env.start("delta", app);
    env.run(&["next", "--thread", "delta"]);
    env.respond("delta", "accept", "fine");
    let out = env.run_with(
        &["request", "delta"],
        Some("round two after the owner's decision"),
        app,
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    let runs: Vec<Vec<String>> = env
        .calls()
        .into_iter()
        .filter(|c| c[..2] == ["hc", "run"])
        .collect();
    assert_eq!(
        runs.len(),
        2,
        "the accepted thread's next round starts a new reviewer"
    );
    assert!(!runs[0].last().unwrap().contains("earlier round"));
    assert!(
        runs[1].last().unwrap().contains("1 earlier round(s)"),
        "{:?}",
        runs[1].last()
    );
}

#[test]
fn start_and_doctor_note_that_codex_will_ask_to_trust_a_new_project() {
    let env = Env::new();
    let app = &[("HC_TERMINAL_ID", "t99")];
    let out = env.start("alpha", app);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(
        out.text.contains("note: Codex has not trusted"),
        "{}",
        out.text
    );
    let doctor = env.run(&["doctor"]);
    assert!(
        doctor.text.contains("warn  Codex has not trusted"),
        "{}",
        doctor.text
    );
    let headless = env.start("beta", &[("ROUNDS_LAUNCHER", "headless")]);
    assert!(
        !headless.text.contains("trust"),
        "codex exec does not ask: {}",
        headless.text
    );

    std::fs::create_dir_all(env.home.join(".codex")).unwrap();
    std::fs::write(
        env.home.join(".codex/config.toml"),
        format!(
            "model = \"x\"\n\n[projects.\"{}\"]\ntrust_level = \"trusted\"\n",
            env.project.canonicalize().unwrap().display()
        ),
    )
    .unwrap();
    let out = env.start("gamma", app);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(!out.text.contains("trust"), "{}", out.text);
    assert!(!env.run(&["doctor"]).text.contains("trust"));
}

fn reviewer_prompts(env: &Env) -> Vec<String> {
    env.calls()
        .into_iter()
        .filter(|c| c[..2] == ["hc", "run"])
        .map(|c| c.last().unwrap().clone())
        .collect()
}

#[test]
fn the_stance_is_independent_unless_the_config_or_the_thread_asks_for_adversarial() {
    let env = Env::new();
    let app = &[("HC_TERMINAL_ID", "t99")];
    let out = env.start("plain", app);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(!out.text.contains("adversarial"), "{}", out.text);

    let out = env.run_with(
        &["start", "skeptic", "--stance", "adversarial"],
        Some("please review"),
        app,
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(out.text.contains("(adversarial review)"), "{}", out.text);

    env.config(|c| c["stance"] = json!("adversarial"));
    env.start("configured", app);

    let prompts = reviewer_prompts(&env);
    assert_eq!(prompts.len(), 3);
    assert!(!prompts[0].contains("adversarial"), "{}", prompts[0]);
    assert!(
        prompts[1].contains("An adversarial review"),
        "{}",
        prompts[1]
    );
    assert!(
        prompts[2].contains("An adversarial review"),
        "{}",
        prompts[2]
    );
    assert!(
        env.run(&["doctor"]).text.contains("stance: adversarial"),
        "the doctor shows the configured stance"
    );

    let out = env.run_with(
        &["start", "odd", "--stance", "hostile"],
        Some("please review"),
        app,
        &env.project,
    );
    assert_ne!(out.code, 0);
    assert!(out.text.contains("unknown stance"), "{}", out.text);
}

#[test]
fn a_thread_keeps_its_stance_for_every_reviewer() {
    let env = Env::new();
    let app = &[("HC_TERMINAL_ID", "t99")];
    env.run_with(
        &["start", "skeptic", "--stance", "adversarial"],
        Some("please review"),
        app,
        &env.project,
    );
    env.run(&["next", "--thread", "skeptic"]);
    env.respond("skeptic", "accept", "fine");
    let out = env.run_with(
        &["request", "skeptic"],
        Some("round two"),
        app,
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    let prompts = reviewer_prompts(&env);
    assert_eq!(
        prompts.len(),
        2,
        "the accepted thread starts a new reviewer"
    );
    assert!(
        prompts[1].contains("An adversarial review"),
        "{}",
        prompts[1]
    );
}
