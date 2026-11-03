//! Dedicated reviewers and their launchers. The fake stands in for Codex and every manager.

mod common;

use std::time::{Duration, Instant};

use common::{Env, git};
use serde_json::json;

const APP: &[(&str, &str)] = &[("HC_TERMINAL_ID", "t99")];

fn after<'a>(call: &'a [String], option: &str) -> &'a str {
    let i = call
        .iter()
        .position(|a| a == option)
        .unwrap_or_else(|| panic!("no {option} in {call:?}"));
    &call[i + 1]
}

// --- The Herdr app ---

#[test]
fn start_opens_a_thread_and_starts_codex_in_a_tab_beside_the_writer() {
    let env = Env::new();
    let out = env.start("delta", APP);
    assert_eq!(out.code, 0, "{}", out.text);
    let state = env.state("delta");
    assert_eq!(state["dedicated"], true);
    assert_eq!(state["dir"], env.project.to_str().unwrap());
    assert_eq!(state["writer_terminal"], "t99");
    let reviewer = &state["reviewer"];
    assert_eq!(
        (
            &reviewer["launcher"],
            &reviewer["agent"],
            &reviewer["terminal"],
            &reviewer["space"],
            &reviewer["tab"]
        ),
        (
            &json!("herdr-app"),
            &json!("rv-delta"),
            &json!("t1"),
            &json!("spW"),
            &json!("tb1")
        )
    );
    let run = env.call(&["hc", "run"]);
    assert_eq!(after(&run, "--cwd"), env.project.to_str().unwrap());
    assert!(run.contains(&"herdr.space=spW".to_string()));
    assert!(run.contains(&"--rm".to_string()));
    let codex = run.iter().position(|a| a.ends_with("/codex")).unwrap();
    assert_eq!(
        run[codex + 1..codex + 3],
        ["-a".to_string(), "never".to_string()]
    );
    let profile = run
        .iter()
        .find(|a| a.starts_with("permissions.reviewright="))
        .unwrap();
    let thread = env.thread_path("delta");
    assert!(
        profile.contains(&format!("{} = \"write\"", json!(thread.to_str().unwrap()))),
        "{profile}"
    );
    assert!(
        profile.contains(r#"":workspace_roots" = { "." = "read" }"#),
        "{profile}"
    );
    assert!(
        profile.contains("network = { enabled = true }"),
        "{profile}"
    );
    assert!(run.last().unwrap().contains("THREAD=delta"));
    assert_eq!(
        env.fake_state("terminals.json")["t1"]["tab_name"],
        "review delta"
    );
}

#[test]
fn the_sandbox_and_the_prompt_follow_the_config() {
    let env = Env::new();
    env.config(|c| {
        c["sandbox"] = json!({ "network": false, "writable_in_project": ["target"], "writable_outside": ["~/.cargo"] });
        c["checks"] = json!(["make test"]);
        c["read_first"] = json!(["AGENTS.md"]);
        c["reviewer"]["args"] = json!(["-m", "some-model"]);
    });
    assert_eq!(env.start("delta", APP).code, 0);
    let run = env.call(&["hc", "run"]);
    let profile = run
        .iter()
        .find(|a| a.starts_with("permissions.reviewright="))
        .unwrap();
    assert!(
        profile.contains(r#""." = "read", "target" = "write""#),
        "{profile}"
    );
    assert!(profile.contains(r#""~/.cargo" = "write""#));
    assert!(profile.contains("network = { enabled = false }"));
    assert_eq!(after(&run, "-m"), "some-model");
    assert!(run.last().unwrap().contains("`make test`"));
    assert!(run.last().unwrap().contains("Read first: AGENTS.md"));
    assert!(
        env.project.join("target").is_dir(),
        "a writable build output exists before the sandbox starts"
    );
}

#[test]
fn a_worktree_reaches_the_main_store_through_a_link() {
    let env = Env::new();
    let tree = env.root.join("tree");
    git(
        &env.project,
        &["worktree", "add", "-q", tree.to_str().unwrap()],
    );
    let tree = tree.canonicalize().unwrap();
    let out = env.run_with(&["start", "delta"], Some("from the worktree"), APP, &tree);
    assert_eq!(out.code, 0, "{}", out.text);
    assert_eq!(env.state("delta")["dir"], tree.to_str().unwrap());
    let link = tree.join(".review");
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        env.project.join(".review")
    );
    assert_eq!(
        after(&env.call(&["hc", "run"]), "--cwd"),
        tree.to_str().unwrap()
    );
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&tree)
        .args(["status", "--short"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&status.stdout),
        "",
        "the link is ignored by git"
    );
}

#[test]
fn reviewright_space_overrides_the_writers_space() {
    let env = Env::new();
    env.start(
        "delta",
        &[("HC_TERMINAL_ID", "t99"), ("REVIEWRIGHT_SPACE", "spX")],
    );
    assert!(
        env.call(&["hc", "run"])
            .contains(&"herdr.space=spX".to_string())
    );
}

#[test]
fn an_accept_ends_the_reviewer() {
    let env = Env::new();
    env.start("delta", APP);
    env.run(&["next", "--thread", "delta"]);
    let out = env.respond("delta", "accept", "fine");
    assert_eq!(out.code, 0, "{}", out.text);
    assert_eq!(env.state("delta")["reviewer"]["closed_reason"], "accepted");
    assert_eq!(env.fake_state("terminals.json"), json!({}));
}

#[test]
fn a_wait_removes_a_terminal_left_behind() {
    let env = Env::new();
    env.start("delta", APP);
    env.run(&["next", "--thread", "delta"]);
    let mut left = env.fake_state("terminals.json");
    env.respond("delta", "accept", "fine");
    left["t1"]["state"] = json!("finished");
    env.write_fake_state("terminals.json", &left);
    let out = env.run(&["wait", "delta", "--timeout", "5"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert_eq!(env.fake_state("terminals.json"), json!({}));
}

#[test]
fn a_lost_reviewer_is_restarted_and_a_closed_thread_ends_it() {
    let env = Env::new();
    env.start("delta", APP);
    env.run(&["next", "--thread", "delta"]);
    env.respond("delta", "changes", "fix");
    let mut left = env.fake_state("terminals.json");
    left["t1"]["state"] = json!("finished");
    env.write_fake_state("terminals.json", &left);
    let out = env.run_with(&["request", "delta"], Some("round two"), APP, &env.project);
    assert_eq!(out.code, 0, "{}", out.text);
    let terminals = env.fake_state("terminals.json");
    assert_eq!(
        terminals.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["t2"]
    );
    env.run(&["close", "delta", "moved", "elsewhere"]);
    assert_eq!(env.fake_state("terminals.json"), json!({}));
}

#[test]
fn a_terminal_under_another_name_is_never_ended() {
    let env = Env::new();
    env.start("delta", APP);
    let mut other = env.fake_state("terminals.json");
    other["t1"]["name"] = json!("someone-else");
    env.write_fake_state("terminals.json", &other);
    env.run(&["reviewer", "delta", "--stop"]);
    assert!(env.fake_state("terminals.json").get("t1").is_some());
}

// --- The thread lifecycle seen by the reviewer ---

#[test]
fn next_in_a_thread_waits_through_a_human_question_and_ends_on_accept() {
    let env = Env::new();
    env.start("delta", APP);
    env.request("alpha", "x");
    assert_eq!(env.run(&["next", "--thread", "delta"]).code, 0);
    assert_eq!(
        env.run(&["next", "--thread", "delta"]).code,
        0,
        "a restarted reviewer gets its open round back"
    );
    env.respond("delta", "human", "Which contract?");
    assert_eq!(
        env.run(&["next", "--thread", "delta", "--wait", "--timeout", "1"])
            .code,
        3
    );
    env.run_with(&["request", "delta"], Some("round two"), APP, &env.project);
    let out = env.run(&["next", "--thread", "delta", "--wait", "--timeout", "5"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(out.text.contains("round two"));
    env.respond("delta", "accept", "fine");
    assert_eq!(env.run(&["next", "--thread", "delta"]).code, 30);
    assert_eq!(env.state("alpha")["status"], "requested");
}

#[test]
fn the_shared_reviewer_never_takes_a_dedicated_thread() {
    let env = Env::new();
    env.start("delta", APP);
    env.request("alpha", "x");
    assert!(env.run(&["next"]).text.contains("reviewing alpha"));
    assert_eq!(env.run(&["next"]).code, 3);
    env.run(&["escalate", "delta", "a", "question"]);
    assert_eq!(
        env.run(&["next"]).code,
        3,
        "a dedicated thread's question does not stop the shared reviewer"
    );
}

#[test]
fn reviewer_gives_a_shared_thread_its_own_reviewer() {
    let env = Env::new();
    env.request("alpha", "x");
    let out = env.run_with(&["reviewer", "alpha"], None, APP, &env.project);
    assert_eq!(out.code, 0, "{}", out.text);
    assert_eq!(env.state("alpha")["dedicated"], true);
    assert!(
        env.run_with(&["reviewer", "alpha"], None, APP, &env.project)
            .text
            .contains("is running")
    );
}

#[test]
fn start_refuses_an_existing_thread() {
    let env = Env::new();
    env.request("alpha", "x");
    let out = env.start("alpha", APP);
    assert_eq!(out.code, 1);
    assert!(out.text.contains("continue it with `review request alpha`"));
}

// --- Other launchers ---

#[test]
fn herdr_starts_a_named_codex_agent_in_a_labeled_tab_and_closes_only_that_tab() {
    let env = Env::new();
    let out = env.start("delta", &[("HERDR_ENV", "1")]);
    assert_eq!(out.code, 0, "{}", out.text);
    let reviewer = env.state("delta")["reviewer"].clone();
    assert_eq!(reviewer["launcher"], "herdr");
    let agent = env.call(&["herdr", "agent", "start"]);
    assert_eq!(agent[3], "rv-delta");
    let dashes = agent.iter().position(|a| a == "--").unwrap();
    assert_eq!(
        agent[dashes + 1..dashes + 3],
        ["-a".to_string(), "never".to_string()]
    );
    env.call(&["herdr", "agent", "prompt"]);
    let tab = reviewer["tab"].as_str().unwrap();
    assert_eq!(
        env.fake_state("herdr.json")["tabs"][tab]["label"],
        "review delta"
    );
    env.run(&["next", "--thread", "delta"]);
    env.respond("delta", "accept", "fine");
    assert_eq!(env.fake_state("herdr.json")["tabs"], json!({}));
}

#[test]
fn tmux_opens_a_window_and_kills_it_on_accept() {
    let env = Env::new();
    let tmux = &[("TMUX", "/tmp/fake,1,0")];
    let out = env.start("delta", tmux);
    assert_eq!(out.code, 0, "{}", out.text);
    let reviewer = env.state("delta")["reviewer"].clone();
    assert_eq!(
        (&reviewer["launcher"], &reviewer["window"]),
        (&json!("tmux"), &json!("@1"))
    );
    let window = env.call(&["tmux", "new-window"]);
    assert_eq!(after(&window, "-n"), "review-delta");
    let words = shlex::split(window.last().unwrap()).unwrap();
    assert!(words.last().unwrap().contains("THREAD=delta"));
    assert!(
        env.run_with(&["reviewer", "delta"], None, tmux, &env.project)
            .text
            .contains("is running")
    );
    env.run(&["next", "--thread", "delta"]);
    env.respond("delta", "accept", "fine");
    assert_eq!(env.fake_state("tmux.json"), json!({}));
}

#[test]
fn the_command_launcher_fills_its_templates() {
    let env = Env::new();
    let marker = env.root.join("started");
    let m = marker.to_str().unwrap().to_string();
    env.config(|c| {
        c["command_launcher"] = json!({
            "start": format!("printf '%s|%s|%s\\n' {{name}} {{thread}} {{cwd}} > {m}; printf '%s' {{command}} > {m}.cmd"),
            "alive": format!("test -f {m}"),
            "stop": format!("rm -f {m}")
        });
    });
    let out = env.start("delta", &[]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert_eq!(env.state("delta")["reviewer"]["launcher"], "command");
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        format!("rv-delta|delta|{}\n", env.project.display())
    );
    assert!(
        std::fs::read_to_string(format!("{m}.cmd"))
            .unwrap()
            .contains("THREAD=delta")
    );
    env.run(&["next", "--thread", "delta"]);
    env.respond("delta", "accept", "fine");
    assert!(!marker.exists());
}

#[test]
fn headless_runs_codex_exec_in_the_background_and_stops_it_on_close() {
    let env = Env::new();
    let out = env.start("delta", &[]);
    assert_eq!(out.code, 0, "{}", out.text);
    let reviewer = env.state("delta")["reviewer"].clone();
    assert_eq!(reviewer["launcher"], "headless");
    let started = Instant::now();
    while !env
        .calls()
        .iter()
        .any(|c| c.get(1).map(String::as_str) == Some("exec"))
    {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "codex exec never ran"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let exec = env.call(&["codex", "exec"]);
    assert!(
        exec.iter()
            .any(|a| a.starts_with("permissions.reviewright="))
    );
    assert!(env.run(&["reviewer", "delta"]).text.contains("is running"));
    env.run(&["close", "delta", "done", "with", "it"]);
    let pid = reviewer["pid"].as_u64().unwrap().to_string();
    let started = Instant::now();
    loop {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid])
            .status()
            .unwrap()
            .success();
        let zombie = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains('Z'))
            .unwrap_or(false);
        if !alive || zombie {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the headless reviewer survived the close"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn manual_prints_the_command() {
    let env = Env::new();
    let out = env.start("delta", &[("REVIEWRIGHT_LAUNCHER", "manual")]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(
        out.text
            .contains("Start the reviewer in a terminal of its own")
    );
    assert!(out.text.contains("-a never"));
}

#[test]
fn auto_prefers_the_herdr_app_then_herdr_then_tmux() {
    let env = Env::new();
    env.start(
        "one",
        &[("HC_TERMINAL_ID", "t99"), ("HERDR_ENV", "1"), ("TMUX", "x")],
    );
    env.start("two", &[("HERDR_ENV", "1"), ("TMUX", "x")]);
    env.start("three", &[("TMUX", "x")]);
    let picked: Vec<_> = ["one", "two", "three"]
        .iter()
        .map(|id| env.state(id)["reviewer"]["launcher"].clone())
        .collect();
    assert_eq!(
        picked,
        vec![json!("herdr-app"), json!("herdr"), json!("tmux")]
    );
}

// --- Doctor ---

#[test]
fn doctor_passes_on_a_working_setup_and_fails_when_the_project_is_writable() {
    let env = Env::new();
    let out = env.run(&["doctor"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(
        out.text
            .contains("the project is read-only for the reviewer")
    );
    assert!(out.text.contains("launcher: headless"));
    let out = env.run_with(
        &["doctor"],
        None,
        &[("FAKE_SANDBOX_OPEN", "1")],
        &env.project,
    );
    assert_eq!(out.code, 1, "{}", out.text);
    assert!(out.text.contains("the reviewer can write the project"));
}
