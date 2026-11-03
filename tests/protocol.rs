//! The thread protocol: rounds, verdicts, waits, the shared reviewer. Threads opened with
//! `request` go to a shared reviewer, so no reviewer is started here.

mod common;

use std::time::{Duration, Instant};

use common::{Env, today};

#[test]
fn the_first_request_creates_todays_dated_thread_directory() {
    let env = Env::new();
    let out = env.request("alpha", "please review");
    assert_eq!(out.code, 0, "{}", out.text);
    let names: Vec<String> = std::fs::read_dir(env.threads_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec![format!("{}-alpha", today())]);
    let request = std::fs::read_to_string(env.thread_path("alpha").join("01-request.md")).unwrap();
    assert!(request.contains("- tree: clean"), "{request}");
    assert!(request.contains("## Ask\n\nplease review"));
}

#[test]
fn a_thread_keeps_its_first_date_and_a_second_directory_is_refused() {
    let env = Env::new();
    env.request("beta", "x");
    std::fs::rename(
        env.thread_path("beta"),
        env.threads_dir().join("20200101-beta"),
    )
    .unwrap();
    env.respond("beta", "changes", "fix");
    assert_eq!(env.request("beta", "fixed").code, 0);
    assert!(
        env.threads_dir()
            .join("20200101-beta/02-request.md")
            .exists()
    );
    std::fs::create_dir_all(env.threads_dir().join("20200102-beta")).unwrap();
    let out = env.run(&["status"]);
    assert_eq!(out.code, 1);
    assert!(out.text.contains("has 2 directories"), "{}", out.text);
}

#[test]
fn rounds_are_request_response_and_stop_in_order() {
    let env = Env::new();
    env.request("gamma", "x");
    env.respond("gamma", "changes", "fix line 3");
    env.request("gamma", "fixed line 3");
    env.run(&["escalate", "gamma", "Which", "contract?"]);
    let out = env.run(&["show", "gamma"]).text;
    let positions: Vec<usize> = [
        "01-request.md",
        "01-response.md",
        "02-request.md",
        "02-stop.md",
    ]
    .iter()
    .map(|f| out.find(f).unwrap_or_else(|| panic!("{f} missing")))
    .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]));
    let state = env.state("gamma");
    assert_eq!(state["status"], "needs-human");
    assert_eq!(state["question"], "Which contract?");
}

#[test]
fn an_open_round_refuses_a_new_request() {
    let env = Env::new();
    env.request("delta", "x");
    let out = env.request("delta", "again");
    assert_eq!(out.code, 1);
    assert!(out.text.contains("wait for its answer"));
}

#[test]
fn a_closed_thread_is_never_picked() {
    let env = Env::new();
    env.request("epsilon", "x");
    assert_eq!(
        env.run(&["close", "epsilon", "superseded", "by", "zeta"])
            .code,
        0
    );
    assert_eq!(env.run(&["next"]).code, 3);
    assert_eq!(env.state("epsilon")["status"], "closed");
}

#[test]
fn bad_thread_names_are_refused() {
    let env = Env::new();
    for name in ["20260101-x", "Upper", "a/b", "-x", "a--b"] {
        assert_eq!(env.request(name, "x").code, 1, "{name}");
    }
}

#[test]
fn state_is_published_by_rename_and_leaves_no_temporary_file() {
    let env = Env::new();
    env.request("eta", "x");
    env.respond("eta", "accept", "fine");
    let leftovers = std::fs::read_dir(env.thread_path("eta"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        })
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn a_verdict_drives_the_wait_exit_code() {
    let env = Env::new();
    for (verdict, code) in [("accept", 0), ("changes", 10), ("human", 20)] {
        let id = format!("w-{verdict}");
        env.request(&id, "x");
        env.respond(&id, verdict, "Question first?\nmore");
        let out = env.run(&["wait", &id, "--timeout", "5"]);
        assert_eq!(out.code, code, "{}", out.text);
    }
    assert_eq!(env.state("w-human")["question"], "Question first?");
}

#[test]
fn a_wait_times_out_with_exit_3() {
    let env = Env::new();
    env.request("theta", "x");
    let out = env.run_with(
        &["wait", "theta", "--timeout", "1"],
        None,
        &[("ROUNDS_POLL", "1")],
        &env.project,
    );
    assert_eq!(out.code, 3, "{}", out.text);
}

#[test]
fn a_wait_wakes_when_the_response_lands() {
    let env = Env::new();
    env.request("iota", "x");
    std::thread::scope(|scope| {
        let waiting = scope.spawn(|| env.run(&["wait", "iota", "--timeout", "30"]));
        std::thread::sleep(Duration::from_millis(1500));
        let started = Instant::now();
        env.respond("iota", "accept", "fine");
        let out = waiting.join().unwrap();
        assert_eq!(out.code, 0, "{}", out.text);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "woke after {:?}",
            started.elapsed()
        );
    });
}

#[test]
fn a_polling_wait_still_sees_the_response() {
    let env = Env::new();
    env.request("kappa", "x");
    std::thread::scope(|scope| {
        let waiting = scope.spawn(|| {
            env.run_with(
                &["wait", "kappa", "--timeout", "30"],
                None,
                &[("ROUNDS_POLL", "1")],
                &env.project,
            )
        });
        std::thread::sleep(Duration::from_millis(500));
        env.respond("kappa", "changes", "fix");
        let out = waiting.join().unwrap();
        assert_eq!(out.code, 10, "{}", out.text);
    });
}

#[test]
fn the_shared_reviewer_takes_the_oldest_request_and_stops_on_a_human_question() {
    let env = Env::new();
    env.request("lambda", "x");
    std::thread::sleep(Duration::from_millis(1100));
    env.request("mu", "x");
    let out = env.run(&["next"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(
        out.text.contains("reviewing lambda round 1"),
        "{}",
        out.text
    );
    env.respond("lambda", "human", "Ship it?");
    let out = env.run(&["next"]);
    assert_eq!(out.code, 20, "{}", out.text);
    assert!(out.text.contains("HUMAN DECISION NEEDED"));
}

#[test]
fn next_with_wait_wakes_on_a_new_request() {
    let env = Env::new();
    std::thread::scope(|scope| {
        let waiting = scope.spawn(|| env.run(&["next", "--wait", "--timeout", "30"]));
        std::thread::sleep(Duration::from_millis(1500));
        env.request("nu", "x");
        let out = waiting.join().unwrap();
        assert_eq!(out.code, 0, "{}", out.text);
        assert!(out.text.contains("reviewing nu round 1"));
    });
}

#[test]
fn help_lists_the_commands() {
    let env = Env::new();
    let out = env.run(&["help"]);
    assert_eq!(out.code, 0);
    for command in [
        "start", "request", "wait", "next", "respond", "escalate", "close", "reviewer", "status",
        "show", "doctor", "selftest",
    ] {
        assert!(out.text.contains(&format!("review {command}")), "{command}");
    }
}
