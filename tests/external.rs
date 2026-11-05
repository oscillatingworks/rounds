//! External reviews: a review of someone else's change, drafted, signed, and posted only by
//! `review post`, as the user, through the platform's command line.

mod common;

use common::Env;

fn start_external(env: &Env, id: &str, extra: &[(&str, &str)]) -> common::Out {
    let mut vars = vec![("ROUNDS_LAUNCHER", "headless")];
    vars.extend_from_slice(extra);
    env.run_with(
        &["start", id, "--title", "PR 42", "--pr", "42"],
        Some("review the change in PR 42"),
        &vars,
        &env.project,
    )
}

fn posted(env: &Env) -> Option<String> {
    std::fs::read_to_string(env.fake_dir.join("posted.md")).ok()
}

fn posts(env: &Env) -> Vec<Vec<String>> {
    env.calls()
        .into_iter()
        .filter(|c| c[0] == "gh" || c[0] == "glab")
        .collect()
}

#[test]
fn an_external_review_is_drafted_signed_and_posted_once_as_a_comment() {
    let env = Env::new();
    let out = start_external(&env, "ext", &[]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert_eq!(env.state("ext")["pr"], "42");
    assert_eq!(env.state("ext")["platform"], "github");
    // The headless reviewer starts in the background: its call reaches the log a moment later.
    let told = (0..50).any(|_| {
        let found = env
            .calls()
            .iter()
            .flatten()
            .any(|a| a.contains("An external review") && a.contains("(42)"));
        if !found {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        found
    });
    assert!(
        told,
        "the reviewer is told it reviews someone else's change"
    );

    let out = env.run(&["post", "ext"]);
    assert_ne!(out.code, 0, "no draft yet: {}", out.text);
    assert!(posts(&env).is_empty());

    let out = env.run_with(
        &["draft", "ext"],
        Some("Thanks! One thing: `parse` drops the last line.\n"),
        &[],
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(out.text.contains("reviewed by 🎾rounds"), "{}", out.text);
    assert!(out.text.contains("only when they say so"), "{}", out.text);
    assert!(posts(&env).is_empty(), "a draft posts nothing");
    let status = env.run(&["status"]).text;
    assert!(status.contains("[pr 42, draft]"), "{status}");

    let out = env.run(&["post", "ext"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(
        out.text.contains("posted ext to 42 (github)"),
        "{}",
        out.text
    );
    let call = env.call(&["gh", "pr", "review", "42"]);
    assert!(call.contains(&"--comment".to_string()), "{call:?}");
    assert!(
        !call
            .iter()
            .any(|a| a == "--approve" || a == "--request-changes"),
        "a comment review only: {call:?}"
    );
    assert_eq!(
        posted(&env).unwrap(),
        "Thanks! One thing: `parse` drops the last line.\n\n---\n\nreviewed by 🎾rounds\n"
    );
    assert!(env.state("ext")["posted"].is_string());
    assert!(env.run(&["status"]).text.contains("[pr 42, posted]"));
    let log = std::fs::read_to_string(env.threads_dir().join(".posts")).unwrap();
    assert!(log.contains(" ext 42 github"), "{log}");

    let out = env.run(&["post", "ext"]);
    assert_ne!(out.code, 0, "posted once: {}", out.text);
    assert!(out.text.contains("--again"), "{}", out.text);
    assert_eq!(posts(&env).len(), 1);
    assert_eq!(env.run(&["post", "ext", "--again"]).code, 0);
    assert_eq!(posts(&env).len(), 2);
}

#[test]
fn a_draft_is_signed_once_and_replaced_by_the_next() {
    let env = Env::new();
    start_external(&env, "ext", &[]);
    env.run_with(&["draft", "ext"], Some("first\n"), &[], &env.project);
    let out = env.run_with(
        &["draft", "ext"],
        Some("second\n\n---\n\nreviewed by 🎾rounds\n"),
        &[],
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    assert_eq!(env.run(&["post", "ext"]).code, 0);
    assert_eq!(
        posted(&env).unwrap(),
        "second\n\n---\n\nreviewed by 🎾rounds\n"
    );
}

#[test]
fn a_personal_thread_has_no_draft_and_no_post() {
    let env = Env::new();
    env.request("mine", "x");
    let out = env.run_with(&["draft", "mine"], Some("text"), &[], &env.project);
    assert_ne!(out.code, 0);
    assert!(out.text.contains("not an external review"), "{}", out.text);
    let out = env.run(&["post", "mine"]);
    assert_ne!(out.code, 0);
    assert!(posts(&env).is_empty());
    assert!(!env.run(&["status"]).text.contains("[pr"));
}

#[test]
fn a_failed_post_records_nothing() {
    let env = Env::new();
    start_external(&env, "ext", &[]);
    env.run_with(&["draft", "ext"], Some("text\n"), &[], &env.project);
    let out = env.run_with(
        &["post", "ext"],
        None,
        &[("FAKE_POST_FAIL", "1")],
        &env.project,
    );
    assert_ne!(out.code, 0, "{}", out.text);
    assert!(out.text.contains("HTTP 422"), "{}", out.text);
    assert!(out.text.contains("nothing was recorded"), "{}", out.text);
    assert!(env.state("ext").get("posted").is_none());
    assert_eq!(env.run(&["post", "ext"]).code, 0, "a later post works");
}

#[test]
fn post_stops_where_the_store_is_read_only_as_in_the_reviewers_sandbox() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    start_external(&env, "ext", &[]);
    env.run_with(&["draft", "ext"], Some("text\n"), &[], &env.project);
    let threads = env.threads_dir();
    let mode = |m| std::fs::set_permissions(&threads, std::fs::Permissions::from_mode(m)).unwrap();
    mode(0o555);
    let out = env.run(&["post", "ext"]);
    mode(0o755);
    assert_ne!(out.code, 0, "{}", out.text);
    assert!(
        out.text.contains("outside the reviewer's sandbox"),
        "{}",
        out.text
    );
    assert!(posts(&env).is_empty(), "nothing reached the platform");
}

#[test]
fn another_platform_comes_from_a_preset_or_the_config() {
    let env = Env::new();
    let glab = env.fakes.join("glab");
    let glab = glab.to_str().unwrap();
    start_external(&env, "lab", &[("ROUNDS_PLATFORM", "gitlab")]);
    assert_eq!(env.state("lab")["platform"], "gitlab");
    env.run_with(&["draft", "lab"], Some("looks good\n"), &[], &env.project);
    let out = env.run_with(
        &["post", "lab"],
        None,
        &[("ROUNDS_PLATFORM_COMMAND", glab)],
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    let call = env.call(&["glab", "mr", "note", "42", "--message"]);
    assert!(
        call.last().unwrap().ends_with("reviewed by 🎾rounds\n"),
        "{call:?}"
    );

    // A platform of the user's own, through the config.
    let config_path = env.project.join(".review/config.json");
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    config["platform"] = "forge".into();
    config["platforms"] = serde_json::json!({
        "forge": { "command": glab, "post": ["{command}", "mr", "note", "{pr}", "--message", "{body}"] }
    });
    std::fs::write(&config_path, config.to_string()).unwrap();
    start_external(&env, "own", &[]);
    assert_eq!(env.state("own")["platform"], "forge");
    env.run_with(&["draft", "own"], Some("ok\n"), &[], &env.project);
    let out = env.run_with(
        &["post", "own"],
        None,
        &[("ROUNDS_PLATFORM_COMMAND", "")],
        &env.project,
    );
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(out.text.contains("(forge)"), "{}", out.text);

    // An unknown platform stops the start.
    let out = start_external(&env, "none", &[("ROUNDS_PLATFORM", "nowhere")]);
    assert_ne!(out.code, 0);
    assert!(out.text.contains("no platform nowhere"), "{}", out.text);
}

#[test]
fn doctor_names_the_platform() {
    let env = Env::new();
    let out = env.run(&["doctor"]);
    assert!(out.text.contains("platform: github"), "{}", out.text);
    let out = env.run_with(
        &["doctor"],
        None,
        &[("ROUNDS_PLATFORM_COMMAND", "no-such-command-xyz")],
        &env.project,
    );
    assert!(out.text.contains("warn  platform github"), "{}", out.text);
}

#[test]
fn post_refuses_a_store_outside_the_kit_even_where_the_kit_is_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    start_external(&env, "ext", &[]);
    env.run_with(&["draft", "ext"], Some("text\n"), &[], &env.project);
    // A reviewer's trick: copy the thread to a writable store and point ROUNDS_THREADS at it.
    let other = env.root.join("tmp-store");
    let thread = env.thread_path("ext");
    let copy = other.join(thread.file_name().unwrap());
    std::fs::create_dir_all(&copy).unwrap();
    for file in ["state.json", "draft.md"] {
        std::fs::copy(thread.join(file), copy.join(file)).unwrap();
    }
    let threads = env.threads_dir();
    let mode = |m| std::fs::set_permissions(&threads, std::fs::Permissions::from_mode(m)).unwrap();
    mode(0o555);
    let out = env.run_with(
        &["post", "ext"],
        None,
        &[("ROUNDS_THREADS", other.to_str().unwrap())],
        &env.project,
    );
    mode(0o755);
    assert_ne!(out.code, 0, "{}", out.text);
    assert!(out.text.contains("not ROUNDS_THREADS"), "{}", out.text);
    assert!(posts(&env).is_empty(), "nothing reached the platform");
}

#[test]
fn two_posts_at_once_reach_the_platform_once() {
    let env = Env::new();
    start_external(&env, "ext", &[]);
    env.run_with(&["draft", "ext"], Some("text\n"), &[], &env.project);
    let slow = [("FAKE_POST_DELAY_MS", "500")];
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| env.run_with(&["post", "ext"], None, &slow, &env.project));
        let b = scope.spawn(|| env.run_with(&["post", "ext"], None, &slow, &env.project));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(posts(&env).len(), 1, "{}\n{}", a.text, b.text);
    let codes = [a.code, b.code];
    assert!(
        codes.contains(&0) && codes.iter().any(|c| *c != 0),
        "{codes:?}"
    );
    let refused = if a.code == 0 { &b.text } else { &a.text };
    assert!(refused.contains("--again"), "{refused}");
}
