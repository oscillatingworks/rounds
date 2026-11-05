//! The review commands.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::args::Args;
use crate::launcher::{agent_name, launch_for, launcher_for, launcher_name};
use crate::store::{Ctx, KIT_NAME, State, flag, round, text};
use crate::util::{clock, die, env_nonempty, git, now_iso};

const OPEN: [&str; 2] = ["requested", "reviewing"];
const DONE: [&str; 2] = ["accepted", "closed"];
const EXIT_OVER: i32 = 30;

pub const USAGE: &str = "\
rounds: review rounds between a writer agent (Claude Code) and a reviewer agent (Codex),
one Codex session per review thread, through files in the project.

  review start <thread> [--title T] [--files a,b] [--stance adversarial]
                                                     < ask   open a thread and start its reviewer
  review request <thread> [--title T] [--files a,b]  < ask   open the next round (restarts a lost reviewer)
  review wait <thread> [--timeout S]                         block until the round has a response (0: no limit)
  review next --thread <thread> [--wait] [--timeout S]       the reviewer takes its thread's open round
  review next [--wait] [--timeout S]                         a shared reviewer takes the oldest shared round
  review respond <thread> --verdict accept|changes|human < body   the reviewer answers the round
  review escalate <thread> \"question\"                        either side stops the loop for a human
  review close <thread> \"reason\"                             the writer closes a thread without a verdict
  review reviewer <thread> [--stop]                          start, restart or stop a thread's reviewer
  review status                                              every thread, one line each
  review show <thread>                                       the whole thread, rounds in order
  review open <thread>                                       the thread's folder and its newest file
  review checks [-- command...]                              the checks (or a command), one run at a time
  review doctor                                              check the setup, the launcher and the sandbox
  review selftest [--timeout S]                              run one real review round end to end

  rounds install [PROJECT] [--skill user|project|none] [--commit] [--no-doctor]
  review uninstall [PROJECT] [--purge]                       remove the kit (the threads stay unless --purge)
  rounds --version

Exit codes of `wait` and `next`: 0 accepted (wait) or a round taken (next), 10 changes requested,
20 a human decision is needed, 3 timed out (run the same command again), 30 (`next --thread`)
the thread is accepted or closed and the reviewer stops.

Settings: .review/config.json (see .review/README.md).";

pub fn is_command(name: &str) -> bool {
    [
        "start", "request", "wait", "next", "respond", "escalate", "close", "reviewer", "status",
        "show", "doctor", "selftest", "open", "checks",
    ]
    .contains(&name)
}

pub fn dispatch(command: &str, words: Vec<String>) {
    let ctx = Ctx::locate();
    match command {
        "start" => open_round(&ctx, words, true),
        "request" => open_round(&ctx, words, false),
        "wait" => wait(&ctx, words),
        "next" => next(&ctx, words),
        "respond" => respond(&ctx, words),
        "escalate" => escalate(&ctx, words),
        "close" => close(&ctx, words),
        "reviewer" => reviewer(&ctx, words),
        "status" => status(&ctx),
        "show" => show(&ctx, words),
        "doctor" => doctor(&ctx),
        "selftest" => selftest(&ctx, words),
        "open" => open(&ctx, words),
        "checks" => checks(&ctx, words),
        _ => unreachable!("dispatch is called for known commands"),
    }
}

fn status_of(state: &State) -> &str {
    text(state, "status")
}

fn set(state: &mut State, key: &str, value: impl Into<Value>) {
    state.insert(key.into(), value.into());
}

fn banner(state: &State) -> String {
    let (id, n) = (text(state, "id"), round(state));
    match status_of(state) {
        "needs-human" => {
            let line = "#".repeat(64);
            format!(
                "\n{line}\n#  HUMAN DECISION NEEDED  thread {id} round {n}\n#  {}\n{line}\n",
                text(state, "question")
            )
        }
        "accepted" => format!("\nACCEPTED  thread {id} after {n} round(s)\n"),
        "responded" => format!("\nCHANGES REQUESTED  thread {id} round {n}\n"),
        _ => String::new(),
    }
}

fn exit_code(state: &State) -> i32 {
    match status_of(state) {
        "accepted" => 0,
        "responded" => 10,
        "needs-human" => 20,
        _ => 3,
    }
}

fn valid_thread(id: &str) {
    let words_ok = !id.is_empty()
        && id.split('-').all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if !words_ok {
        die("thread names are lower-case words and digits joined by dashes");
    }
    if id.len() > 9 && id.as_bytes()[..8].iter().all(u8::is_ascii_digit) && id.as_bytes()[8] == b'-'
    {
        die("thread names do not start with a date; the tool adds the day");
    }
}

fn read_stdin() -> String {
    let mut body = String::new();
    let _ = std::io::stdin().read_to_string(&mut body);
    body.trim().to_string()
}

fn write_file(path: &Path, body: &str) {
    std::fs::write(path, body)
        .unwrap_or_else(|e| die(format!("review: cannot write {}: {e}", path.display())));
}

fn thread_arg<'a>(args: &'a Args, usage: &str) -> &'a str {
    args.first()
        .unwrap_or_else(|| die(format!("usage: {usage}")))
}

// --- The reviewer's lifecycle ---

/// A worktree reaches the one thread store through a symlink; git ignores the name in every
/// worktree (the exclude file is shared), so the link never shows in a diff.
fn link_kit(ctx: &Ctx, dir: &Path) {
    if dir == ctx.root {
        return;
    }
    let link = dir.join(KIT_NAME);
    if link.symlink_metadata().is_ok() {
        if crate::util::real(&link) == crate::util::real(&ctx.kit) {
            return;
        }
        die(format!(
            "review: {} exists and is not the main checkout's {KIT_NAME}; move it away first",
            link.display()
        ));
    }
    std::os::unix::fs::symlink(&ctx.kit, &link)
        .unwrap_or_else(|e| die(format!("review: cannot link {}: {e}", link.display())));
}

fn reviewer_of(state: &State) -> Option<&serde_json::Map<String, Value>> {
    state.get("reviewer").and_then(Value::as_object)
}

fn reviewer_live(ctx: &Ctx, state: &State) -> bool {
    match reviewer_of(state) {
        Some(r) if r.get("closed").is_none() && !text(r, "launcher").is_empty() => {
            launcher_for(text(r, "launcher")).alive(ctx, r)
        }
        _ => false,
    }
}

fn launch_reviewer(ctx: &Ctx, id: &str, state: &mut State) {
    if reviewer_of(state).is_some() {
        stop_reviewer(ctx, id, state, "restarted");
    }
    let dir = ctx.dir_of(state);
    set(state, "dir", dir.to_string_lossy().to_string());
    link_kit(ctx, &dir);
    for path in ctx.config.strings("/sandbox/writable_in_project") {
        let _ = std::fs::create_dir_all(dir.join(path));
    }
    let name = launcher_name(ctx);
    let launch = launch_for(ctx, id, state, &dir);
    let Some(mut info) = launcher_for(&name).start(ctx, &launch) else {
        die(format!(
            "review: the reviewer did not start (launcher {name}); run `review reviewer {id}` to try again"
        ));
    };
    let place = text(&info, "where").to_string();
    info.insert("launcher".into(), name.clone().into());
    info.insert("agent".into(), agent_name(id).into());
    info.insert("thread".into(), id.into());
    info.insert("started".into(), now_iso().into());
    set(state, "reviewer", Value::Object(info));
    ctx.write_state(id, state);
    let stance = crate::launcher::stance_of(ctx, state);
    if stance == "independent" {
        println!("reviewer {} started: {place}", agent_name(id));
    } else {
        println!(
            "reviewer {} started ({stance} review): {place}",
            agent_name(id)
        );
    }
    if name != "headless"
        && crate::agent::reviewer(ctx).sandbox == "codex"
        && !crate::agent::codex_trusts(ctx, &dir)
    {
        println!("note: {}", crate::agent::trust_note(ctx));
    }
}

/// Idempotent: every caller checks the session itself, so a stop that did not happen is done by
/// the next caller (the writer's `wait` among them). A reviewer stopping itself ends its own
/// session; its response is written by then.
fn stop_reviewer(ctx: &Ctx, id: &str, state: &mut State, reason: &str) {
    let (snapshot, fresh) = {
        let Some(reviewer) = state.get_mut("reviewer").and_then(Value::as_object_mut) else {
            return;
        };
        let fresh = reviewer.get("closed").is_none();
        if fresh {
            reviewer.insert("closed".into(), now_iso().into());
            reviewer.insert("closed_reason".into(), reason.into());
        }
        (reviewer.clone(), fresh)
    };
    if fresh {
        ctx.write_state(id, state);
    }
    let _ = std::io::stdout().flush();
    if !text(&snapshot, "launcher").is_empty() {
        launcher_for(text(&snapshot, "launcher")).stop(ctx, &snapshot);
    }
}

fn ensure_reviewer(ctx: &Ctx, id: &str, state: &mut State) {
    if flag(state, "dedicated") && !reviewer_live(ctx, state) {
        launch_reviewer(ctx, id, state);
    }
}

// --- Commands ---

fn open_round(ctx: &Ctx, words: Vec<String>, dedicated: bool) {
    let usage = format!(
        "review {} <thread> [--title T] [--files a,b]{} < ask",
        if dedicated { "start" } else { "request" },
        if dedicated {
            " [--stance independent|adversarial]"
        } else {
            ""
        }
    );
    let options: &[&str] = if dedicated {
        &["title", "files", "stance"]
    } else {
        &["title", "files"]
    };
    let args = Args::parse(words, options, &[], &usage);
    let id = thread_arg(&args, &usage).to_string();
    valid_thread(&id);
    let body = read_stdin();
    if body.is_empty() {
        die("empty request body");
    }
    let exists = ctx.exists(&id);
    if dedicated && exists {
        die(format!(
            "thread {id} exists; continue it with `review request {id}`"
        ));
    }
    let dir = ctx.thread_dir(&id);
    std::fs::create_dir_all(&dir)
        .unwrap_or_else(|e| die(format!("review: cannot create {}: {e}", dir.display())));
    let mut state = if exists {
        ctx.read_state(&id)
    } else {
        json!({ "id": id, "title": args.value("title").unwrap_or(&id), "round": 0, "created": now_iso() })
            .as_object()
            .cloned()
            .expect("an object")
    };
    if OPEN.contains(&status_of(&state)) {
        die(format!(
            "thread {id} is {}; wait for its answer or escalate it first",
            status_of(&state)
        ));
    }
    let n = round(&state) + 1;
    set(&mut state, "round", n);
    set(&mut state, "status", "requested");
    if let Some(title) = args.value("title") {
        set(&mut state, "title", title);
    }
    let checkout = ctx.checkout_dir();
    set(&mut state, "dir", checkout.to_string_lossy().to_string());
    if dedicated {
        set(&mut state, "dedicated", true);
        set(
            &mut state,
            "stance",
            crate::launcher::new_stance(ctx, args.value("stance")),
        );
    }
    if let Some(terminal) = env_nonempty("HC_TERMINAL_ID") {
        set(&mut state, "writer_terminal", terminal);
    }
    state.remove("question");
    let files = args
        .value("files")
        .map_or("- files: see the tree below".to_string(), |f| {
            format!("- files: {f}")
        });
    let request = format!(
        "# Review request: {title} (round {n})\n\n- thread: {id}\n- round: {n}\n- created: {now}\n{files}\n{snapshot}\n\n\
         ## Ask\n\n{body}\n\n## Answer with\n\n    {KIT_NAME}/bin/review respond {id} --verdict accept|changes|human < response.md\n",
        title = text(&state, "title"),
        now = now_iso(),
        snapshot = ctx.snapshot(&checkout),
    );
    let path = ctx.round_path(&id, n, "request");
    write_file(&path, &request);
    ctx.write_state(&id, &mut state);
    println!("requested {id} round {n}: {}", path.display());
    if n > ctx.config.max_rounds() {
        eprintln!(
            "review: round {n} on {id}; after {} rounds, escalate to the human",
            ctx.config.max_rounds()
        );
    }
    ensure_reviewer(ctx, &id, &mut state);
}

fn reviewer(ctx: &Ctx, words: Vec<String>) {
    let usage = "review reviewer <thread> [--stop]";
    let args = Args::parse(words, &[], &["stop"], usage);
    let id = thread_arg(&args, usage).to_string();
    let mut state = ctx.read_state(&id);
    if args.flag("stop") {
        stop_reviewer(ctx, &id, &mut state, "stopped");
        println!("reviewer of {id} stopped");
    } else if reviewer_live(ctx, &state) {
        let r = reviewer_of(&state).expect("a live reviewer");
        println!(
            "reviewer {} is running: {}",
            text(r, "agent"),
            text(r, "where")
        );
    } else {
        if DONE.contains(&status_of(&state)) {
            die(format!(
                "thread {id} is {}; nothing to review",
                status_of(&state)
            ));
        }
        set(&mut state, "dedicated", true);
        launch_reviewer(ctx, &id, &mut state);
    }
}

fn wait(ctx: &Ctx, words: Vec<String>) {
    let usage = "review wait <thread> [--timeout S] (0: until the verdict)";
    let args = Args::parse(words, &["timeout"], &[], usage);
    let id = thread_arg(&args, usage).to_string();
    let timeout = args.seconds("timeout", 590);
    ctx.read_state(&id);
    let started = Instant::now();
    let deadline = (timeout > 0).then(|| started + Duration::from_secs(timeout));
    let mut next_note = Duration::from_secs(600);
    let mut watcher = crate::watch::Watcher::new(&ctx.threads, suspend_records(ctx, Some(&id)));
    loop {
        let mut state = ctx.read_state(&id);
        if !OPEN.contains(&status_of(&state)) {
            let response = ctx.round_path(&id, round(&state), "response");
            if let Ok(text) = std::fs::read_to_string(&response) {
                print!("{text}");
            }
            println!("{}", banner(&state));
            if status_of(&state) == "accepted" {
                stop_reviewer(ctx, &id, &mut state, "accepted");
            }
            std::process::exit(exit_code(&state));
        }
        let now = Instant::now();
        if deadline.is_some_and(|d| now >= d) {
            println!(
                "still {} after {timeout}s: run `review wait {id}` again",
                status_of(&state)
            );
            std::process::exit(3);
        }
        if now - started >= next_note {
            println!(
                "still {} on round {} after {}",
                status_of(&state),
                round(&state),
                crate::util::duration(now - started)
            );
            let _ = std::io::stdout().flush();
            next_note += Duration::from_secs(600);
        }
        watcher.wait(deadline.map_or(Duration::from_secs(60), |d| d - now));
    }
}

fn take(ctx: &Ctx, mut state: State) -> ! {
    set(&mut state, "status", "reviewing");
    let id = text(&state, "id").to_string();
    ctx.write_state(&id, &mut state);
    let path = ctx.round_path(&id, round(&state), "request");
    println!(
        "reviewing {id} round {}: {}\n",
        round(&state),
        path.display()
    );
    print!("{}", std::fs::read_to_string(&path).unwrap_or_default());
    std::process::exit(0)
}

fn next(ctx: &Ctx, words: Vec<String>) {
    let usage = "review next [--thread T] [--wait] [--timeout S]";
    let args = Args::parse(words, &["timeout", "thread"], &["wait"], usage);
    let (wait, timeout) = (args.flag("wait"), args.seconds("timeout", 590));
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut watcher = wait.then(|| {
        crate::watch::Watcher::new(&ctx.threads, suspend_records(ctx, args.value("thread")))
    });
    loop {
        let message = match args.value("thread") {
            // The dedicated reviewer: its own thread only. A pending human decision or a changes
            // verdict keeps it waiting; a round it already took is handed back; an accepted or
            // closed thread ends it.
            Some(id) => {
                let state = ctx.read_state(id);
                if OPEN.contains(&status_of(&state)) {
                    take(ctx, state);
                }
                if DONE.contains(&status_of(&state)) {
                    println!("thread {id} is {}: the review is over", status_of(&state));
                    std::process::exit(EXIT_OVER);
                }
                (
                    format!("no open round on {id} ({})", status_of(&state)),
                    format!(
                        "no round on {id} after {timeout}s: run `review next --thread {id} --wait` again"
                    ),
                )
            }
            // The shared reviewer never sees a dedicated thread.
            None => {
                let shared: Vec<State> = ctx
                    .threads()
                    .into_iter()
                    .filter(|t| !flag(t, "dedicated"))
                    .collect();
                if let Some(human) = shared.iter().find(|t| status_of(t) == "needs-human") {
                    println!("{}", banner(human));
                    std::process::exit(20);
                }
                let oldest = shared
                    .into_iter()
                    .filter(|t| status_of(t) == "requested")
                    .min_by(|a, b| text(a, "updated").cmp(text(b, "updated")));
                if let Some(state) = oldest {
                    take(ctx, state);
                }
                (
                    "no open request".to_string(),
                    format!("no request after {timeout}s: run `review next --wait` again"),
                )
            }
        };
        let Some(watcher) = watcher.as_mut() else {
            println!("{}", message.0);
            std::process::exit(3);
        };
        let now = Instant::now();
        if now >= deadline {
            println!("{}", message.1);
            std::process::exit(3);
        }
        watcher.wait(deadline - now);
    }
}

fn respond(ctx: &Ctx, words: Vec<String>) {
    let usage = "review respond <thread> --verdict accept|changes|human < body";
    let args = Args::parse(words, &["verdict"], &[], usage);
    let id = thread_arg(&args, usage).to_string();
    let verdict = args.value("verdict").unwrap_or("");
    let status = match verdict {
        "accept" => "accepted",
        "changes" => "responded",
        "human" => "needs-human",
        _ => die("--verdict is one of accept, changes, human"),
    };
    let mut state = ctx.read_state(&id);
    if !OPEN.contains(&status_of(&state)) {
        die(format!(
            "thread {id} has no open round (status {})",
            status_of(&state)
        ));
    }
    let body = read_stdin();
    if body.is_empty() {
        die("empty response body");
    }
    let n = round(&state);
    let response = format!(
        "# Review response: {title} (round {n})\n\n- thread: {id}\n- round: {n}\n- verdict: {verdict}\n- created: {now}\n{snapshot}\n\n{body}\n",
        title = text(&state, "title"),
        now = now_iso(),
        snapshot = ctx.snapshot(&ctx.dir_of(&state)),
    );
    write_file(&ctx.round_path(&id, n, "response"), &response);
    set(&mut state, "status", status);
    if verdict == "human" {
        set(
            &mut state,
            "question",
            body.lines().next().unwrap_or("").trim(),
        );
    }
    ctx.write_state(&id, &mut state);
    println!("responded {id} round {n}: {verdict}");
    if verdict == "accept" {
        stop_reviewer(ctx, &id, &mut state, "accepted");
    }
}

fn escalate(ctx: &Ctx, words: Vec<String>) {
    let usage = "review escalate <thread> \"question\"";
    let args = Args::parse(words, &[], &[], usage);
    let id = thread_arg(&args, usage).to_string();
    let question = args.rest().join(" ").trim().to_string();
    if question.is_empty() {
        die("give the question for the human");
    }
    let mut state = ctx.read_state(&id);
    set(&mut state, "status", "needs-human");
    set(&mut state, "question", question.clone());
    write_file(
        &ctx.round_path(&id, round(&state), "stop"),
        &format!(
            "# Human decision needed\n\n{question}\n\n- created: {}\n",
            now_iso()
        ),
    );
    ctx.write_state(&id, &mut state);
    println!("{}", banner(&state));
}

fn close(ctx: &Ctx, words: Vec<String>) {
    let usage = "review close <thread> \"reason\"";
    let args = Args::parse(words, &[], &[], usage);
    let id = thread_arg(&args, usage).to_string();
    let reason = args.rest().join(" ").trim().to_string();
    if reason.is_empty() {
        die("give the reason");
    }
    let mut state = ctx.read_state(&id);
    if status_of(&state) == "accepted" {
        die(format!("thread {id} is accepted; nothing to close"));
    }
    set(&mut state, "status", "closed");
    set(&mut state, "question", reason.clone());
    write_file(
        &ctx.round_path(&id, round(&state), "closed"),
        &format!("# Closed\n\n{reason}\n\n- created: {}\n", now_iso()),
    );
    ctx.write_state(&id, &mut state);
    println!("closed {id} round {}: {reason}", round(&state));
    stop_reviewer(ctx, &id, &mut state, "closed");
}

/// Where a wait notes a suspend: the store, or the thread's own directory when the sandbox keeps
/// the store read-only.
fn suspend_records(ctx: &Ctx, id: Option<&str>) -> Vec<PathBuf> {
    let file = crate::watch::SUSPENDS_FILE;
    let mut records = vec![ctx.threads.join(file)];
    if let Some(id) = id {
        records.push(ctx.thread_dir(id).join(file));
    }
    records
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The time the reviewer spent on a thread: each round from its request to its response (an
/// open round counts until now), from the files' times, so older threads have it too. The
/// suspends the waits noted do not count.
fn review_time(
    ctx: &Ctx,
    id: &str,
    state: &State,
    suspends: &[crate::watch::Suspend],
) -> (Duration, bool) {
    let now = std::time::SystemTime::now();
    let (mut total, mut open) = (Duration::ZERO, false);
    for n in 1..=round(state) {
        let Some(asked) = modified(&ctx.round_path(id, n, "request")) else {
            continue;
        };
        let answered = modified(&ctx.round_path(id, n, "response"))
            .or_else(|| modified(&ctx.round_path(id, n, "stop")))
            .or_else(|| modified(&ctx.round_path(id, n, "closed")));
        if answered.is_none() && n == round(state) && OPEN.contains(&status_of(state)) {
            open = true;
        }
        let answered = answered.unwrap_or(now);
        total += answered
            .duration_since(asked)
            .unwrap_or_default()
            .saturating_sub(crate::watch::suspended(suspends, asked, answered));
    }
    (total, open)
}

fn status(ctx: &Ctx) {
    let list = ctx.threads();
    if list.is_empty() {
        println!("no threads");
        return;
    }
    let now = std::time::SystemTime::now();
    let mut rows: Vec<(std::time::SystemTime, String)> = Vec::new();
    let width = list.iter().map(|t| text(t, "id").len()).max().unwrap_or(0);
    let suspends = crate::watch::suspends(&ctx.threads);
    for t in &list {
        let id = text(t, "id");
        let last = modified(&ctx.state_path(id)).unwrap_or(now);
        let ago = crate::util::duration(now.duration_since(last).unwrap_or_default());
        let (spent, open) = review_time(ctx, id, t, &suspends);
        let reviewer = match reviewer_of(t) {
            Some(r) if r.get("closed").is_none() => {
                format!("  [{}: {}]", text(r, "launcher"), text(r, "agent"))
            }
            _ => String::new(),
        };
        rows.push((
            last,
            format!(
                "{id:<width$}  round {:<2}  {:<12}  {} ({ago} ago)  review {}{}  {}{reviewer}",
                round(t),
                status_of(t),
                crate::util::utc_minute(last),
                crate::util::duration(spent),
                if open { ", open" } else { "" },
                text(t, "title")
            ),
        ));
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    for (_, row) in rows {
        println!("{row}");
    }
    for t in list.iter().filter(|t| status_of(t) == "needs-human") {
        println!("{}", banner(t));
    }
}

/// `review open <thread>`: where a thread's files are (git ignores `.review`, so editors that hide
/// ignored files do not show them), and its newest file.
fn open(ctx: &Ctx, words: Vec<String>) {
    let usage = "review open <thread>";
    let args = Args::parse(words, &[], &[], usage);
    let id = thread_arg(&args, usage).to_string();
    ctx.read_state(&id);
    let dir = ctx.thread_dir(&id);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| entries.filter_map(|e| Some(e.ok()?.path())).collect())
        .unwrap_or_default();
    files.retain(|f| {
        f.file_name()
            .is_some_and(|n| !n.to_string_lossy().starts_with('.'))
            && f.file_name().is_some_and(|n| n != "state.json")
    });
    files.sort();
    println!("{}", dir.display());
    if let Some(latest) = files.last() {
        println!("{}", latest.display());
    }
}

/// `review checks [-- command...]`: the configured checks (or the given command) under one lock
/// for the whole kit, so reviewers and writers do not run heavy checks at the same time. The lock
/// is a file in /tmp, which both sandboxes can write.
fn checks(ctx: &Ctx, words: Vec<String>) {
    let split = words.iter().position(|w| w == "--");
    let commands: Vec<String> = match split {
        Some(i) => vec![crate::util::shell_join(&words[i + 1..])],
        None => ctx.config.strings("/checks"),
    };
    if commands.is_empty() || commands[0].is_empty() {
        die("review: no checks in .review/config.json; give a command: review checks -- <command>");
    }
    let lock_path = PathBuf::from("/tmp").join(format!(
        "rounds-checks-{:016x}.lock",
        crate::util::fnv1a(&ctx.kit.to_string_lossy())
    ));
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .unwrap_or_else(|e| die(format!("review: cannot open {}: {e}", lock_path.display())));
    let asked = Instant::now();
    lock.lock()
        .unwrap_or_else(|e| die(format!("review: cannot lock {}: {e}", lock_path.display())));
    println!(
        "checks lock: waited {}",
        crate::util::duration(asked.elapsed())
    );
    let mut first_failure = 0;
    for command in &commands {
        println!("$ {command}");
        let _ = std::io::stdout().flush();
        let started = Instant::now();
        let code = Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(1);
        println!(
            "exit {code} after {}",
            crate::util::duration(started.elapsed())
        );
        if code != 0 && first_failure == 0 {
            first_failure = code;
        }
    }
    drop(lock);
    std::process::exit(first_failure);
}

fn show(ctx: &Ctx, words: Vec<String>) {
    let usage = "review show <thread>";
    let args = Args::parse(words, &[], &[], usage);
    let id = thread_arg(&args, usage).to_string();
    ctx.read_state(&id);
    let dir = ctx.thread_dir(&id);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| entries.filter_map(|e| Some(e.ok()?.path())).collect())
        .unwrap_or_default();
    files.retain(|f| f.extension().is_some_and(|e| e == "md"));
    files.sort();
    let rule = "=".repeat(72);
    for file in files {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        println!(
            "{rule}\n{name}\n{rule}\n{}\n",
            std::fs::read_to_string(&file).unwrap_or_default()
        );
    }
}

/// Checks what a review needs and says what is missing. Exit 0 when the setup works.
fn doctor(ctx: &Ctx) {
    let mut problems = 0;
    let mut check = |ok: bool, good: String, bad: String| {
        println!(
            "{}  {}",
            if ok { "ok  " } else { "FAIL" },
            if ok { good } else { bad }
        );
        if !ok {
            problems += 1;
        }
    };
    check(
        true,
        format!("rounds {}", env!("CARGO_PKG_VERSION")),
        String::new(),
    );
    check(
        git(&ctx.root, &["rev-parse", "--is-inside-work-tree"]) == "true",
        format!("git repository at {}", ctx.root.display()),
        format!("{} is not a git repository", ctx.root.display()),
    );
    let ignored = Command::new("git")
        .args([
            "-C",
            &ctx.root.to_string_lossy(),
            "check-ignore",
            "-q",
            KIT_NAME,
        ])
        .status()
        .is_ok_and(|s| s.success());
    check(
        ignored,
        format!("{KIT_NAME} is ignored by git"),
        format!("{KIT_NAME} is not ignored: add it to .git/info/exclude (or commit it on purpose)"),
    );
    let agent = crate::agent::reviewer(ctx);
    let found = crate::util::command_exists(&agent.command);
    check(
        found,
        format!(
            "reviewer: {} ({}), sandbox {}",
            agent.name, agent.command, agent.sandbox
        ),
        format!(
            "reviewer: {} is not installed (the command {})",
            agent.name, agent.command
        ),
    );
    if found && agent.name == "codex" {
        let login = Command::new(&agent.command)
            .args(["login", "status"])
            .stdin(Stdio::null())
            .output();
        let (logged, line) = match login {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).to_string()
                    + &String::from_utf8_lossy(&out.stderr);
                (
                    out.status.success(),
                    text.lines().next().unwrap_or("").trim().to_string(),
                )
            }
            Err(_) => (false, String::new()),
        };
        check(
            logged,
            format!("codex login: {line}"),
            "codex is not logged in: run `codex login`".into(),
        );
    }
    if found && agent.sandbox == "codex" && !crate::agent::codex_trusts(ctx, &ctx.root) {
        println!(
            "warn  {} (an interactive reviewer: not the headless launcher)",
            crate::agent::trust_note(ctx)
        );
    }
    if agent.sandbox == "none" {
        println!(
            "warn  sandbox: none; the reviewer can write the project (sandbox \"none\" in its agent)"
        );
    } else if found {
        // The same sandbox the reviewer gets, around a probe instead of the agent.
        let probe = ctx.threads.join(format!(".doctor-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&probe);
        let marker = ctx
            .root
            .join(format!(".review-doctor-{}", std::process::id()));
        let script = format!(
            "touch {} && echo THREAD-WRITE-OK; touch {} 2>/dev/null && echo PROJECT-WRITE-OPEN || echo PROJECT-READ-ONLY",
            crate::util::shell_quote(&probe.join("ok").to_string_lossy()),
            crate::util::shell_quote(&marker.to_string_lossy())
        );
        let probe_command = vec!["sh".to_string(), "-c".to_string(), script];
        let argv = if agent.sandbox == "codex" {
            let mut argv = vec![agent.command.clone(), "sandbox".to_string()];
            argv.extend(crate::agent::profile_args(
                &crate::agent::permission_profile(ctx, "doctor", &probe),
            ));
            argv.extend(["-P", crate::agent::PROFILE, "--"].map(String::from));
            argv.extend(probe_command);
            argv
        } else {
            agent.wrap(ctx, &ctx.root, &probe, probe_command)
        };
        let out = Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(&ctx.root)
            .stdin(Stdio::null())
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr)
            })
            .unwrap_or_else(|e| format!("{}: {e}", argv[0]));
        let _ = std::fs::remove_file(&marker);
        let _ = std::fs::remove_dir_all(&probe);
        check(
            out.contains("THREAD-WRITE-OK"),
            format!(
                "sandbox ({}): the reviewer can write its thread directory",
                agent.sandbox
            ),
            format!(
                "sandbox ({}): the thread directory is not writable ({})",
                agent.sandbox,
                out.trim().lines().last().unwrap_or("")
            ),
        );
        check(
            out.contains("PROJECT-READ-ONLY"),
            format!(
                "sandbox ({}): the project is read-only for the reviewer",
                agent.sandbox
            ),
            format!(
                "sandbox ({}): the reviewer can write the project",
                agent.sandbox
            ),
        );
    }
    check(
        true,
        format!(
            "stance: {} (\"stance\": \"adversarial\" in config.json, or `review start --stance adversarial` for one thread)",
            crate::launcher::new_stance(ctx, None)
        ),
        String::new(),
    );
    check(
        true,
        format!("launcher: {}", launcher_name(ctx)),
        String::new(),
    );
    println!(
        "      (auto picks: herdr-app inside the Herdr app, herdr inside Herdr, tmux inside tmux, the command launcher when configured, else headless)"
    );
    std::process::exit(if problems == 0 { 0 } else { 1 });
}

/// One real round end to end: a thread, its reviewer, a write it must not manage, a note it must
/// write, an accept. The thread is deleted afterwards, whatever the outcome.
fn selftest(ctx: &Ctx, words: Vec<String>) {
    let args = Args::parse(words, &["timeout"], &[], "review selftest [--timeout S]");
    let timeout = args.seconds("timeout", 900);
    let id = format!("rounds-selftest-{}", clock());
    let forbidden = ctx.checkout_dir().join(".review-selftest");
    let ask = format!(
        "This is the rounds self-test, not a code review. Do exactly this:\n\
         1. Run `touch {}` once. It must fail (the project is read-only for you). Do not try another way.\n\
         2. Write the file `01-notes-selftest.md` in THREAD_DIR with one line: the result of step 1.\n\
         3. Respond with --verdict accept and one line: the result of step 1.\n",
        forbidden.display()
    );
    let tool = std::env::current_exe().unwrap_or_else(|_| ctx.tool());
    let mut child = Command::new(&tool)
        .args(["start", &id, "--title", "rounds self-test"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| die(format!("selftest: {e}")));
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(ask.as_bytes());
    }
    if !child.wait().is_ok_and(|s| s.success()) {
        die("selftest: the thread did not start");
    }
    let code = Command::new(&tool)
        .args(["wait", &id, "--timeout", &timeout.to_string()])
        .status()
        .ok()
        .and_then(|s| s.code())
        .unwrap_or(1);
    let notes = ctx.thread_dir(&id).join("01-notes-selftest.md").exists();
    let wrote = forbidden.exists();
    let mut state = ctx.read_state(&id);
    stop_reviewer(ctx, &id, &mut state, "selftest");
    println!();
    println!(
        "verdict:            {}",
        if code == 0 {
            "accept".to_string()
        } else {
            format!("exit {code}")
        }
    );
    println!(
        "note in the thread: {}",
        if notes { "written" } else { "MISSING" }
    );
    println!(
        "write to project:   {}",
        if wrote { "NOT BLOCKED" } else { "blocked" }
    );
    let _ = std::fs::remove_file(&forbidden);
    let _ = std::fs::remove_dir_all(ctx.thread_dir(&id));
    let ok = code == 0 && notes && !wrote;
    println!(
        "{}",
        if ok {
            "\nselftest passed"
        } else {
            "\nselftest FAILED"
        }
    );
    std::process::exit(if ok { 0 } else { 1 });
}
