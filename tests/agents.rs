//! Reviewer agents: the presets, the model, custom agents, and the two sandboxes.

mod common;

use common::Env;
use serde_json::json;

const APP: &[(&str, &str)] = &[("HC_TERMINAL_ID", "t99")];

fn with_command(env: &Env, name: &str) -> String {
    env.fakes.join(name).to_string_lossy().to_string()
}

#[test]
fn claude_reviews_inside_the_wrap_sandbox_with_its_tools_allowed() {
    let env = Env::new();
    env.config(|c| c["reviewer"] = json!({ "agent": "claude", "model": "opus" }));
    let claude = with_command(&env, "claude");
    let out = env.run_with(
        &["start", "delta"],
        Some("please review"),
        &[
            ("HC_TERMINAL_ID", "t99"),
            ("ROUNDS_REVIEWER_COMMAND", &claude),
        ],
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    let run = env.call(&["hc", "run"]);
    let argv: Vec<&str> = run.iter().map(String::as_str).collect();
    let sandbox = argv
        .iter()
        .position(|a| a.ends_with("/bwrap"))
        .expect("the agent runs wrapped");
    let inner = argv.iter().position(|a| *a == "--").unwrap() + 1;
    assert!(sandbox < inner);
    let sandbox_args = &argv[sandbox..inner];
    let project = env.project.to_str().unwrap();
    let thread = env.thread_path("delta");
    let ro = sandbox_args
        .windows(3)
        .position(|w| w == ["--ro-bind", project, project])
        .expect("the project is read-only");
    let rw = sandbox_args
        .windows(3)
        .position(|w| w[0] == "--bind-try" && w[1] == thread.to_str().unwrap())
        .expect("the thread directory is writable");
    assert!(
        ro < rw,
        "the thread directory is mounted over the read-only project"
    );
    assert_eq!(argv[inner], claude);
    assert_eq!(
        &argv[inner + 1..inner + 3],
        ["--allowedTools", "Bash,Read,Grep,Glob,Write,Edit"]
    );
    assert_eq!(&argv[inner + 3..inner + 5], ["--model", "opus"]);
    assert!(
        !argv.iter().any(|a| a.contains("dangerously")),
        "no permission bypass"
    );
    assert!(argv.last().unwrap().contains("THREAD=delta"));
}

#[test]
fn the_model_comes_from_the_config_and_goes_away_without_one() {
    let env = Env::new();
    env.start("one", APP);
    let first = env.call(&["hc", "run"]);
    assert!(
        !first.contains(&"--model".to_string()),
        "no model: no --model"
    );
    env.config(|c| c["reviewer"]["model"] = json!("some-model"));
    env.start("two", APP);
    let second = env
        .calls()
        .into_iter()
        .filter(|c| c[..2] == ["hc", "run"])
        .nth(1)
        .unwrap();
    let i = second.iter().position(|a| a == "--model").unwrap();
    assert_eq!(second[i + 1], "some-model");
}

#[test]
fn a_custom_agent_follows_its_templates() {
    let env = Env::new();
    let agent = with_command(&env, "my-agent");
    env.config(|c| {
        c["reviewer"] = json!({ "agent": "mine", "model": "m1", "args": ["--quiet"] });
        c["agents"] = json!({ "mine": {
            "command": agent,
            "interactive": ["{command}", "chat", "--model", "{model}", "{args}", "--dir", "{cwd}", "{prompt}"],
            "sandbox": "none"
        }});
    });
    let out = env.run_with(
        &["start", "delta"],
        Some("x"),
        &[("HC_TERMINAL_ID", "t99"), ("ROUNDS_REVIEWER_COMMAND", "")],
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    let run = env.call(&["hc", "run"]);
    let i = run.iter().position(|a| a == &agent).unwrap();
    let project = env.project.to_str().unwrap().to_string();
    assert_eq!(
        run[i + 1..i + 7],
        ["chat", "--model", "m1", "--quiet", "--dir", &project].map(String::from)
    );
    assert!(run.last().unwrap().contains("THREAD=delta"));
    let doctor = env.run_with(
        &["doctor"],
        None,
        &[("ROUNDS_REVIEWER_COMMAND", "")],
        &env.project,
    );
    assert!(
        doctor.text.contains("warn  sandbox: none"),
        "{}",
        doctor.text
    );
}

#[test]
fn an_unknown_agent_is_refused() {
    let env = Env::new();
    env.config(|c| c["reviewer"]["agent"] = json!("nobody"));
    let out = env.start("delta", APP);
    assert_eq!(out.code, 1);
    assert!(
        out.text.contains("unknown reviewer agent nobody"),
        "{}",
        out.text
    );
}

#[test]
fn herdr_runs_a_wrapped_agent_in_the_tab_pane() {
    let env = Env::new();
    env.config(|c| c["reviewer"]["agent"] = json!("claude"));
    let claude = with_command(&env, "claude");
    let out = env.run_with(
        &["start", "delta"],
        Some("x"),
        &[("HERDR_ENV", "1"), ("ROUNDS_REVIEWER_COMMAND", &claude)],
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(
        !env.calls()
            .iter()
            .any(|c| c[..3] == ["herdr", "agent", "start"]),
        "no named agent for a wrapped command"
    );
    let run = env.call(&["herdr", "pane", "run"]);
    assert!(
        run[4].contains("bwrap") && run[4].contains("THREAD=delta"),
        "{run:?}"
    );
    assert_eq!(env.state("delta")["reviewer"]["mode"], "pane");
}

#[test]
fn the_doctor_checks_the_wrap_sandbox_of_the_reviewer() {
    let env = Env::new();
    env.config(|c| c["reviewer"]["agent"] = json!("claude"));
    let claude = with_command(&env, "claude");
    let out = env.run_with(
        &["doctor"],
        None,
        &[("ROUNDS_REVIEWER_COMMAND", &claude)],
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(out.text.contains("reviewer: claude"), "{}", out.text);
    assert!(
        out.text
            .contains("sandbox (wrap): the project is read-only"),
        "{}",
        out.text
    );
    assert!(
        !out.text.contains("codex login"),
        "no Codex login check for another agent"
    );
    let out = env.run_with(
        &["doctor"],
        None,
        &[
            ("ROUNDS_REVIEWER_COMMAND", &claude),
            ("FAKE_SANDBOX_OPEN", "1"),
        ],
        &env.project,
    );
    assert_eq!(out.code, 1, "{}", out.text);
}
