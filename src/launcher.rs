//! Where the reviewer's session runs, and the Codex command line it runs with.
//!
//! A launcher starts the reviewer in the writer's checkout, says whether it still runs, and stops
//! it. What `start` returns is kept in the thread's state as the reviewer.

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::store::{Ctx, State, text};
use crate::util::{
    capture, command_exists, die, env_nonempty, json_command, shell_join, shell_quote, toml_string,
};

pub type Reviewer = Map<String, Value>;

pub fn agent_name(id: &str) -> String {
    let name: String = format!("rv-{id}").chars().take(32).collect();
    name.trim_end_matches('-').to_string()
}

pub fn tab_label(id: &str) -> String {
    format!("review {id}")
}

fn exe(name: &str, default: &str) -> String {
    env_nonempty(&format!("REVIEWRIGHT_{name}")).unwrap_or_else(|| default.to_string())
}

pub fn codex_exe(ctx: &Ctx) -> String {
    env_nonempty("REVIEWRIGHT_CODEX")
        .or_else(|| ctx.config.string("/reviewer/command"))
        .unwrap_or_else(|| "codex".into())
}

/// Codex's permission profile, passed on its command line (nothing changes in
/// ~/.codex/config.toml): the disk readable, the project read-only, writable only the thread
/// directory, the build outputs and caches the config lists, and the temporary directories.
pub fn permission_profile(ctx: &Ctx, label: &str, thread_dir: &Path) -> String {
    let mut entries = vec![
        r#"":root" = "read""#.to_string(),
        r#"":tmpdir" = "write""#.to_string(),
        r#"":slash_tmp" = "write""#.to_string(),
    ];
    let outside = std::iter::once(thread_dir.to_string_lossy().to_string())
        .chain(ctx.config.strings("/sandbox/writable_outside"));
    entries.extend(outside.map(|path| format!(r#"{} = "write""#, toml_string(&path))));
    let mut inside = vec![r#""." = "read""#.to_string()];
    inside.extend(
        ctx.config
            .strings("/sandbox/writable_in_project")
            .iter()
            .map(|path| format!(r#"{} = "write""#, toml_string(path))),
    );
    entries.push(format!(
        r#"":workspace_roots" = {{ {} }}"#,
        inside.join(", ")
    ));
    format!(
        r#"{{ description = {}, filesystem = {{ {} }}, network = {{ enabled = {} }} }}"#,
        toml_string(&format!("reviewright: {label}")),
        entries.join(", "),
        ctx.config.bool("/sandbox/network")
    )
}

/// The name of the Codex permission profile: the one the arguments define is the one they select.
pub const PROFILE: &str = "reviewright";

pub fn profile_args(profile: &str) -> Vec<String> {
    vec![
        "-c".into(),
        format!("default_permissions={}", toml_string(PROFILE)),
        "-c".into(),
        format!("permissions.{PROFILE}={profile}"),
    ]
}

fn codex_config_args(ctx: &Ctx, id: &str, dir: &Path) -> Vec<String> {
    let mut args = profile_args(&permission_profile(ctx, id, &ctx.thread_dir(id)));
    args.push("-c".into());
    args.push(format!(
        r#"projects.{}.trust_level="trusted""#,
        toml_string(&dir.to_string_lossy())
    ));
    args
}

fn extra_codex_args(ctx: &Ctx) -> Vec<String> {
    let mut args = ctx.config.strings("/reviewer/args");
    if let Some(extra) = env_nonempty("REVIEWRIGHT_CODEX_ARGS") {
        args.extend(shlex::split(&extra).unwrap_or_default());
    }
    args
}

/// The interactive reviewer: approvals off, so a refused command returns to Codex as an error.
pub fn codex_argv(ctx: &Ctx, id: &str, dir: &Path) -> Vec<String> {
    let mut argv = vec![codex_exe(ctx), "-a".into(), "never".into()];
    argv.extend(codex_config_args(ctx, id, dir));
    argv.extend(extra_codex_args(ctx));
    argv
}

/// The headless reviewer: `codex exec` runs the same loop until the thread is over.
fn codex_exec_argv(ctx: &Ctx, id: &str, dir: &Path) -> Vec<String> {
    let mut argv = vec![codex_exe(ctx), "exec".into()];
    argv.extend(codex_config_args(ctx, id, dir));
    argv.extend(extra_codex_args(ctx));
    argv
}

pub fn reviewer_prompt(ctx: &Ctx, id: &str) -> String {
    let mut prompt = format!(
        "You are the dedicated reviewer of the review thread {id}. Read {} and follow it with \
         THREAD={id} and THREAD_DIR={}.",
        ctx.kit.join("REVIEWER.md").display(),
        ctx.thread_dir(id).display()
    );
    let reads = ctx.config.strings("/read_first");
    if !reads.is_empty() {
        prompt.push_str(&format!(" Read first: {}.", reads.join(", ")));
    }
    let checks = ctx.config.strings("/checks");
    if !checks.is_empty() {
        let quoted: Vec<String> = checks.iter().map(|c| format!("`{c}`")).collect();
        prompt.push_str(&format!(" The project's checks: {}.", quoted.join(", ")));
    }
    prompt
}

/// What a launcher needs to start a reviewer.
pub struct Launch<'a> {
    pub id: &'a str,
    pub dir: &'a Path,
    pub argv: Vec<String>,
    pub prompt: String,
    pub writer_terminal: Option<String>,
}

pub trait Launcher {
    fn start(&self, ctx: &Ctx, launch: &Launch) -> Option<Reviewer>;
    fn alive(&self, ctx: &Ctx, reviewer: &Reviewer) -> bool;
    fn stop(&self, ctx: &Ctx, reviewer: &Reviewer);
}

pub const NAMES: [&str; 6] = [
    "herdr-app",
    "herdr",
    "tmux",
    "command",
    "headless",
    "manual",
];

fn available(ctx: &Ctx, name: &str) -> bool {
    match name {
        "herdr-app" => env_nonempty("HC_TERMINAL_ID").is_some() && command_exists(&exe("HC", "hc")),
        "herdr" => {
            std::env::var("HERDR_ENV").as_deref() == Ok("1")
                && command_exists(&exe("HERDR", "herdr"))
        }
        "tmux" => env_nonempty("TMUX").is_some() && command_exists(&exe("TMUX", "tmux")),
        "command" => ctx.config.string("/command_launcher/start").is_some(),
        _ => true,
    }
}

/// The launcher the config (or REVIEWRIGHT_LAUNCHER) names; `auto` takes the first that fits.
pub fn launcher_name(ctx: &Ctx) -> String {
    let wanted = env_nonempty("REVIEWRIGHT_LAUNCHER")
        .or_else(|| ctx.config.string("/launcher"))
        .unwrap_or_else(|| "auto".into());
    if wanted != "auto" {
        return wanted;
    }
    ["herdr-app", "herdr", "tmux", "command", "headless"]
        .into_iter()
        .find(|name| available(ctx, name))
        .unwrap_or("headless")
        .to_string()
}

pub fn launcher_for(name: &str) -> Box<dyn Launcher> {
    match name {
        "herdr-app" => Box::new(HerdrApp),
        "herdr" => Box::new(Herdr),
        "tmux" => Box::new(Tmux),
        "command" => Box::new(CommandLauncher),
        "headless" => Box::new(Headless),
        "manual" => Box::new(Manual),
        other => die(format!(
            "review: unknown launcher {other} (one of {}, or auto)",
            NAMES.join(", ")
        )),
    }
}

fn s(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn field(reviewer: &Reviewer, key: &str) -> String {
    text(reviewer, key).to_string()
}

// --- The Herdr app: herdr-core (`hc`) runs the terminal, the app shows it as a tab in a space ---

struct HerdrApp;

impl HerdrApp {
    fn hc(&self, command: &str, args: &[&str]) -> Option<Value> {
        let mut all = vec![command, "--json"];
        all.extend_from_slice(args);
        json_command(&exe("HC", "hc"), &all, None)
    }

    fn spaces(&self) -> Vec<Value> {
        json_command(&exe("HERDR_APP", "herdr-app"), &["ls"], None)
            .and_then(|doc| doc.get("spaces").and_then(Value::as_array).cloned())
            .unwrap_or_else(|| {
                die("review: the Herdr app does not answer (herdr-app ls); open it first")
            })
    }

    fn holds(space: &Value, terminal: &str) -> bool {
        space["tabs"].as_array().into_iter().flatten().any(|tab| {
            tab["terminals"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|t| t["terminal"] == terminal)
        })
    }

    /// REVIEWRIGHT_SPACE, else the space of the writer's terminal (or of the terminal running this
    /// command), else the space whose folder is the checkout.
    fn space_for(&self, launch: &Launch) -> String {
        if let Some(space) = env_nonempty("REVIEWRIGHT_SPACE") {
            return space;
        }
        let spaces = self.spaces();
        let terminals = [
            launch.writer_terminal.clone(),
            env_nonempty("HC_TERMINAL_ID"),
        ];
        let found = terminals
            .iter()
            .flatten()
            .find_map(|t| spaces.iter().find(|space| Self::holds(space, t)))
            .or_else(|| {
                spaces
                    .iter()
                    .find(|space| space["folder"].as_str().map(Path::new) == Some(launch.dir))
            });
        found.and_then(|space| s(space, "/id")).unwrap_or_else(|| {
            die("review: no Herdr space for this thread; set REVIEWRIGHT_SPACE (herdr-app ls)")
        })
    }

    fn tab_of(&self, terminal: &str) -> Option<String> {
        self.spaces().iter().find_map(|space| {
            space["tabs"].as_array()?.iter().find_map(|tab| {
                tab["terminals"]
                    .as_array()?
                    .iter()
                    .any(|t| t["terminal"] == terminal)
                    .then(|| s(tab, "/id"))
                    .flatten()
            })
        })
    }

    fn terminal(&self, reviewer: &Reviewer) -> Option<Value> {
        let address = field(reviewer, "terminal");
        if address.is_empty() {
            return None;
        }
        self.hc("get", &[&address])
            .filter(|t| t["name"].as_str() == Some(text(reviewer, "agent")))
    }
}

impl Launcher for HerdrApp {
    fn start(&self, _ctx: &Ctx, launch: &Launch) -> Option<Reviewer> {
        let space = self.space_for(launch);
        let (name, dir) = (
            agent_name(launch.id),
            launch.dir.to_string_lossy().to_string(),
        );
        let (space_label, thread_label) = (
            format!("herdr.space={space}"),
            format!("reviewright.thread={}", launch.id),
        );
        let mut args: Vec<&str> = vec![
            "--bg",
            "--name",
            &name,
            "--cwd",
            &dir,
            "--no-parent",
            "--no-notify",
            "--rm",
            "--label",
            &space_label,
            "--label",
            &thread_label,
        ];
        args.extend(launch.argv.iter().map(String::as_str));
        args.push(&launch.prompt);
        let started = self.hc("run", &args)?;
        let address = s(&started, "/address")?;
        let mut tab = None;
        for _ in 0..20 {
            tab = self.tab_of(&address);
            if tab.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if let Some(tab) = &tab {
            let label = tab_label(launch.id);
            json_command(
                &exe("HERDR_APP", "herdr-app"),
                &["do", "tab.rename", "--id", tab, "--name", &label],
                None,
            );
        }
        let json = json!({
            "terminal": address, "space": space, "tab": tab,
            "where": format!("Herdr app: tab \"{}\" (terminal {address}); hc read {name}", tab_label(launch.id))
        });
        json.as_object().cloned()
    }

    fn alive(&self, _ctx: &Ctx, reviewer: &Reviewer) -> bool {
        self.terminal(reviewer)
            .is_some_and(|t| t["state"].as_str() == Some("live"))
    }

    /// Started with --rm: the terminal goes once it ends; a finished one left behind is removed.
    fn stop(&self, _ctx: &Ctx, reviewer: &Reviewer) {
        if let Some(terminal) = self.terminal(reviewer) {
            let address = field(reviewer, "terminal");
            let command = if terminal["state"].as_str() == Some("live") {
                "kill"
            } else {
                "rm"
            };
            self.hc(command, &[&address]);
        }
    }
}

// --- Herdr, the terminal multiplexer: a tab in the checkout's workspace, Codex as a named agent ---

struct Herdr;

impl Herdr {
    fn herdr(&self, args: &[&str], cwd: Option<&Path>) -> Option<Value> {
        json_command(&exe("HERDR", "herdr"), args, cwd).and_then(|doc| doc.get("result").cloned())
    }

    fn workspace_for(&self, dir: &Path) -> String {
        if let Some(workspace) = env_nonempty("HERDR_WORKSPACE_ID") {
            let here = std::env::current_dir()
                .map(|d| crate::util::real(&d))
                .unwrap_or_default();
            if here.starts_with(dir) {
                return workspace;
            }
        }
        let list = self
            .herdr(&["worktree", "list"], Some(dir))
            .unwrap_or_else(|| die("review: herdr worktree list failed"));
        let entry = list["worktrees"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|w| {
                w["path"].as_str().map(|p| crate::util::real(Path::new(p)))
                    == Some(dir.to_path_buf())
            });
        if let Some(workspace) = entry.and_then(|e| s(e, "/open_workspace_id")) {
            return workspace;
        }
        let source = s(&list, "/source/source_workspace_id")
            .unwrap_or_else(|| die("review: open the repository in Herdr first"));
        let dir_text = dir.to_string_lossy();
        self.herdr(
            &[
                "worktree",
                "open",
                "--workspace",
                &source,
                "--path",
                &dir_text,
                "--no-focus",
            ],
            Some(dir),
        )
        .and_then(|opened| s(&opened, "/workspace/workspace_id"))
        .unwrap_or_else(|| die("review: herdr worktree open failed"))
    }
}

impl Launcher for Herdr {
    fn start(&self, _ctx: &Ctx, launch: &Launch) -> Option<Reviewer> {
        let dir = launch.dir;
        let dir_text = dir.to_string_lossy();
        let workspace = self.workspace_for(dir);
        let label = tab_label(launch.id);
        let tab = self.herdr(
            &[
                "tab",
                "create",
                "--workspace",
                &workspace,
                "--cwd",
                &dir_text,
                "--label",
                &label,
                "--no-focus",
            ],
            Some(dir),
        )?;
        let tab_id = s(&tab, "/tab/tab_id")?;
        let pane = s(&tab, "/root_pane/pane_id")?;
        let name = agent_name(launch.id);
        let mut args: Vec<&str> = vec![
            "agent",
            "start",
            &name,
            "--kind",
            "codex",
            "--pane",
            &pane,
            "--timeout",
            "90000",
            "--",
        ];
        args.extend(launch.argv.iter().skip(1).map(String::as_str));
        if self.herdr(&args, Some(dir)).is_none() {
            self.herdr(&["tab", "close", &tab_id], None);
            return None;
        }
        self.herdr(&["agent", "prompt", &name, &launch.prompt], Some(dir));
        json!({
            "workspace": workspace, "tab": tab_id, "pane": pane,
            "where": format!("Herdr: tab \"{label}\" ({tab_id}), agent {name}")
        })
        .as_object()
        .cloned()
    }

    fn alive(&self, _ctx: &Ctx, reviewer: &Reviewer) -> bool {
        self.herdr(&["agent", "get", text(reviewer, "agent")], None)
            .is_some_and(|agent| {
                agent.pointer("/agent/pane_id").and_then(Value::as_str)
                    == Some(text(reviewer, "pane"))
            })
    }

    /// Only the tab the tool opened, under the label it gave.
    fn stop(&self, _ctx: &Ctx, reviewer: &Reviewer) {
        let tab = field(reviewer, "tab");
        let label = self
            .herdr(&["tab", "get", &tab], None)
            .and_then(|t| s(&t, "/tab/label"));
        if label.as_deref() == Some(tab_label(text(reviewer, "thread")).as_str()) {
            self.herdr(&["tab", "close", &tab], None);
        }
    }
}

// --- tmux: a window named after the thread, in the writer's tmux server ---

struct Tmux;

impl Tmux {
    fn tmux(&self, args: &[&str]) -> Option<String> {
        capture(&exe("TMUX", "tmux"), args, None).map(|s| s.trim().to_string())
    }

    fn window_name(id: &str) -> String {
        tab_label(id).replace(' ', "-")
    }
}

impl Launcher for Tmux {
    fn start(&self, _ctx: &Ctx, launch: &Launch) -> Option<Reviewer> {
        let mut words = launch.argv.clone();
        words.push(launch.prompt.clone());
        let command = shell_join(&words);
        let name = Self::window_name(launch.id);
        let dir = launch.dir.to_string_lossy();
        let window = self.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-n",
            &name,
            "-c",
            &dir,
            &command,
        ])?;
        json!({ "window": window, "where": format!("tmux: window {name} ({window})") })
            .as_object()
            .cloned()
    }

    fn alive(&self, _ctx: &Ctx, reviewer: &Reviewer) -> bool {
        let window = text(reviewer, "window");
        self.tmux(&["list-windows", "-a", "-F", "#{window_id} #{pane_dead}"])
            .is_some_and(|list| list.lines().any(|l| l.split_whitespace().eq([window, "0"])))
    }

    fn stop(&self, _ctx: &Ctx, reviewer: &Reviewer) {
        let window = text(reviewer, "window");
        let listed = self
            .tmux(&["list-windows", "-a", "-F", "#{window_id}"])
            .is_some_and(|list| list.split_whitespace().any(|w| w == window));
        if listed {
            self.tmux(&["kill-window", "-t", window]);
        }
    }
}

// --- Any other manager: shell templates from config.json ---

struct CommandLauncher;

impl CommandLauncher {
    /// Fills {name}, {thread}, {cwd}, {label} and {command} (all shell-quoted).
    fn fill(template: &str, id: &str, cwd: &Path, command: &str) -> String {
        let values = [
            ("{name}", shell_quote(&agent_name(id))),
            ("{thread}", shell_quote(id)),
            ("{cwd}", shell_quote(&cwd.to_string_lossy())),
            ("{label}", shell_quote(&tab_label(id))),
            ("{command}", shell_quote(command)),
        ];
        let mut out = String::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            rest = &rest[start..];
            match values.iter().find(|(key, _)| rest.starts_with(key)) {
                Some((key, value)) => {
                    out.push_str(value);
                    rest = &rest[key.len()..];
                }
                None => {
                    out.push('{');
                    rest = &rest[1..];
                }
            }
        }
        out.push_str(rest);
        out
    }

    fn sh(script: &str, cwd: &Path, quiet: bool) -> bool {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(script)
            .current_dir(cwd)
            .stdin(Stdio::null());
        if quiet {
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }
        command.status().is_ok_and(|s| s.success())
    }
}

impl Launcher for CommandLauncher {
    fn start(&self, ctx: &Ctx, launch: &Launch) -> Option<Reviewer> {
        let template = ctx.config.string("/command_launcher/start")?;
        let mut words = launch.argv.clone();
        words.push(launch.prompt.clone());
        let script = Self::fill(&template, launch.id, launch.dir, &shell_join(&words));
        Self::sh(&script, launch.dir, false).then(|| {
            let mut reviewer = Map::new();
            reviewer.insert(
                "where".into(),
                format!("your agent manager: {}", agent_name(launch.id)).into(),
            );
            reviewer
        })
    }

    fn alive(&self, ctx: &Ctx, reviewer: &Reviewer) -> bool {
        match ctx.config.string("/command_launcher/alive") {
            Some(template) => Self::sh(
                &Self::fill(&template, text(reviewer, "thread"), &ctx.root, ""),
                &ctx.root,
                true,
            ),
            None => true,
        }
    }

    fn stop(&self, ctx: &Ctx, reviewer: &Reviewer) {
        if let Some(template) = ctx.config.string("/command_launcher/stop") {
            Self::sh(
                &Self::fill(&template, text(reviewer, "thread"), &ctx.root, ""),
                &ctx.root,
                true,
            );
        }
    }
}

// --- No manager: `codex exec` in the background, its output in the thread directory ---

struct Headless;

impl Headless {
    fn pid(reviewer: &Reviewer) -> Option<u64> {
        reviewer.get("pid").and_then(Value::as_u64)
    }
}

impl Launcher for Headless {
    fn start(&self, ctx: &Ctx, launch: &Launch) -> Option<Reviewer> {
        let log_path = ctx.thread_dir(launch.id).join("reviewer.log");
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok()?;
        let argv = codex_exec_argv(ctx, launch.id, launch.dir);
        let child = Command::new(&argv[0])
            .args(&argv[1..])
            .arg(&launch.prompt)
            .current_dir(launch.dir)
            .stdin(Stdio::null())
            .stdout(log.try_clone().ok()?)
            .stderr(log)
            .process_group(0)
            .spawn()
            .ok()?;
        let pid = child.id();
        json!({ "pid": pid, "log": log_path, "where": format!("headless: codex exec, pid {pid}, log {}", log_path.display()) })
            .as_object()
            .cloned()
    }

    fn alive(&self, _ctx: &Ctx, reviewer: &Reviewer) -> bool {
        let Some(pid) = Self::pid(reviewer) else {
            return false;
        };
        let pid = pid.to_string();
        capture("ps", &["-o", "command=", "-p", &pid], None)
            .is_some_and(|command| command.contains("exec"))
    }

    fn stop(&self, ctx: &Ctx, reviewer: &Reviewer) {
        if self.alive(ctx, reviewer) {
            if let Some(pid) = Self::pid(reviewer) {
                capture("kill", &["-TERM", "--", &format!("-{pid}")], None);
            }
        }
    }
}

// --- The person starts it: the tool prints the command ---

struct Manual;

impl Launcher for Manual {
    fn start(&self, _ctx: &Ctx, launch: &Launch) -> Option<Reviewer> {
        let mut words = launch.argv.clone();
        words.push(launch.prompt.clone());
        println!(
            "review: no agent manager found. Start the reviewer in a terminal of its own:\n\n  cd {} && {}\n",
            shell_quote(&launch.dir.to_string_lossy()),
            shell_join(&words)
        );
        let mut reviewer = Map::new();
        reviewer.insert(
            "where".into(),
            "a terminal you start (the command above)".into(),
        );
        Some(reviewer)
    }

    fn alive(&self, _ctx: &Ctx, _reviewer: &Reviewer) -> bool {
        true
    }

    fn stop(&self, _ctx: &Ctx, _reviewer: &Reviewer) {}
}

/// A launch for a thread's state.
pub fn launch_for<'a>(ctx: &Ctx, id: &'a str, state: &State, dir: &'a Path) -> Launch<'a> {
    Launch {
        id,
        dir,
        argv: codex_argv(ctx, id, dir),
        prompt: reviewer_prompt(ctx, id),
        writer_terminal: state
            .get("writer_terminal")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selected_profile_is_the_defined_profile() {
        let args = profile_args("{ }");
        let selected = args[1].strip_prefix("default_permissions=").unwrap();
        let selected: String = serde_json::from_str(selected).unwrap();
        let defined = args[3]
            .strip_prefix("permissions.")
            .unwrap()
            .split('=')
            .next()
            .unwrap();
        assert_eq!(selected, defined);
        assert_eq!(defined, PROFILE);
    }

    #[test]
    fn templates_fill_known_keys_and_keep_other_braces() {
        let filled = CommandLauncher::fill(
            "run {name} in {cwd}: {command} {other}",
            "fix-x",
            Path::new("/a b"),
            "codex 'x'",
        );
        assert_eq!(
            filled,
            r#"run rv-fix-x in '/a b': 'codex '\''x'\''' {other}"#
        );
    }

    #[test]
    fn agent_names_fit_32_characters_without_a_trailing_dash() {
        assert_eq!(agent_name("short"), "rv-short");
        let long = agent_name("a-very-long-thread-name-that-goes-on-and-on");
        assert!(long.len() <= 32 && !long.ends_with('-'));
    }
}
