//! The reviewer agent: which program reviews, with which model, inside which sandbox.
//!
//! An agent is a command and two argument templates (interactive and headless), with a sandbox
//! mode. Presets cover Codex (`codex`, the default) and Claude Code (`claude`); `agents` in
//! config.json overrides a preset or defines another agent. Template elements:
//!
//! - `{command}`: the agent's executable;
//! - `{prompt}`: the reviewer's first message;
//! - `{model}`: the model; without one, the element goes, and so does the option before it
//!   (`"--model", "{model}"` disappears as a pair);
//! - `{args}`: the extra arguments, spliced in;
//! - `{codex_sandbox}`: Codex's permission profile arguments, spliced in (sandbox `codex`);
//! - `{thread}`, `{thread_dir}`, `{cwd}` inside any element.
//!
//! Sandbox modes:
//!
//! - `codex`: Codex's own sandbox, from a permission profile on its command line;
//! - `wrap`: rounds runs the agent inside an operating-system sandbox (bubblewrap on Linux,
//!   sandbox-exec on macOS): the disk read-only, writable only the thread directory, the
//!   project's build outputs, the caches, the temporary directories and the agent's own state;
//! - `none`: no sandbox (the doctor warns).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::store::Ctx;
use crate::util::{die, env_nonempty, toml_string};

/// The name of the Codex permission profile: the one the arguments define is the one they select.
pub const PROFILE: &str = "rounds";

pub fn presets() -> Value {
    json!({
        "codex": {
            "command": "codex",
            "interactive": ["{command}", "-a", "never", "{codex_sandbox}", "--model", "{model}", "{args}", "{prompt}"],
            "headless": ["{command}", "exec", "{codex_sandbox}", "--model", "{model}", "{args}", "{prompt}"],
            "sandbox": "codex",
            "writable": [],
            "herdr_kind": "codex"
        },
        "claude": {
            "command": "claude",
            "interactive": ["{command}", "--allowedTools", "Bash,Read,Grep,Glob,Write,Edit", "--model", "{model}", "{args}", "{prompt}"],
            "headless": ["{command}", "-p", "--allowedTools", "Bash,Read,Grep,Glob,Write,Edit", "--model", "{model}", "{args}", "{prompt}"],
            "sandbox": "wrap",
            "writable": ["~/.claude", "~/.claude.json", "~/.claude.json.backup", "~/.local/share/claude", "~/.local/state"],
            "herdr_kind": "claude"
        }
    })
}

pub struct Agent {
    pub name: String,
    pub command: String,
    interactive: Vec<String>,
    headless: Vec<String>,
    pub sandbox: String,
    writable: Vec<String>,
    pub herdr_kind: Option<String>,
    model: Option<String>,
    args: Vec<String>,
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The agent the config names (`reviewer.agent`, or ROUNDS_REVIEWER), its preset under the
/// config's `agents` entry of the same name.
pub fn reviewer(ctx: &Ctx) -> Agent {
    let name = env_nonempty("ROUNDS_REVIEWER")
        .or_else(|| ctx.config.string("/reviewer/agent"))
        .unwrap_or_else(|| "codex".into());
    let mut def = presets().get(&name).cloned().unwrap_or_else(|| json!({}));
    if let Some(Value::Object(own)) = ctx.config.value(&format!("/agents/{name}")) {
        for (key, value) in own {
            def[key.as_str()] = value.clone();
        }
    }
    if def.get("command").is_none() || def.get("interactive").is_none() {
        die(format!(
            "review: unknown reviewer agent {name}: use codex or claude, or define it under `agents` in .review/config.json"
        ));
    }
    let command = env_nonempty("ROUNDS_REVIEWER_COMMAND")
        .or_else(|| ctx.config.string("/reviewer/command"))
        .unwrap_or_else(|| def["command"].as_str().unwrap_or_default().to_string());
    let mut args = ctx.config.strings("/reviewer/args");
    if let Some(extra) = env_nonempty("ROUNDS_REVIEWER_ARGS") {
        args.extend(shlex::split(&extra).unwrap_or_default());
    }
    let interactive = strings(&def, "interactive");
    let headless = match strings(&def, "headless") {
        h if h.is_empty() => interactive.clone(),
        h => h,
    };
    Agent {
        name,
        command,
        interactive,
        headless,
        sandbox: def["sandbox"].as_str().unwrap_or("wrap").to_string(),
        writable: strings(&def, "writable"),
        herdr_kind: def["herdr_kind"].as_str().map(str::to_string),
        model: env_nonempty("ROUNDS_REVIEWER_MODEL")
            .or_else(|| ctx.config.string("/reviewer/model")),
        args,
    }
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
        toml_string(&format!("rounds: {label}")),
        entries.join(", "),
        ctx.config.bool("/sandbox/network")
    )
}

pub fn profile_args(profile: &str) -> Vec<String> {
    vec![
        "-c".into(),
        format!("default_permissions={}", toml_string(PROFILE)),
        "-c".into(),
        format!("permissions.{PROFILE}={profile}"),
    ]
}

fn codex_sandbox_args(ctx: &Ctx, id: &str, dir: &Path) -> Vec<String> {
    let mut args = profile_args(&permission_profile(ctx, id, &ctx.thread_dir(id)));
    args.push("-c".into());
    args.push(format!(
        r#"projects.{}.trust_level="trusted""#,
        toml_string(&dir.to_string_lossy())
    ));
    args
}

pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(rest),
        None => PathBuf::from(path),
    }
}

/// The reviewer's command line, ready for a launcher.
pub struct ReviewerCommand {
    /// The whole command line, the prompt included.
    pub argv: Vec<String>,
    /// For a manager that starts agents of a known kind (Herdr): the kind, and the arguments
    /// after the executable without the prompt. None when the agent runs wrapped.
    pub kind: Option<(String, Vec<String>)>,
}

impl Agent {
    fn expand(
        &self,
        template: &[String],
        ctx: &Ctx,
        id: &str,
        dir: &Path,
        prompt: &str,
    ) -> Vec<String> {
        let thread_dir = ctx.thread_dir(id).to_string_lossy().to_string();
        let cwd = dir.to_string_lossy().to_string();
        let mut out: Vec<String> = Vec::new();
        for element in template {
            match element.as_str() {
                "{command}" => out.push(self.command.clone()),
                "{prompt}" => out.push(prompt.to_string()),
                "{args}" => out.extend(self.args.iter().cloned()),
                "{codex_sandbox}" => out.extend(codex_sandbox_args(ctx, id, dir)),
                "{model}" => match &self.model {
                    Some(model) => out.push(model.clone()),
                    None => {
                        if out.last().is_some_and(|o| o.starts_with('-')) {
                            out.pop();
                        }
                    }
                },
                other => out.push(
                    other
                        .replace("{thread_dir}", &thread_dir)
                        .replace("{thread}", id)
                        .replace("{cwd}", &cwd),
                ),
            }
        }
        out
    }

    pub fn command(
        &self,
        ctx: &Ctx,
        id: &str,
        dir: &Path,
        prompt: &str,
        headless: bool,
    ) -> ReviewerCommand {
        let template = if headless {
            &self.headless
        } else {
            &self.interactive
        };
        let mut inner = self.expand(template, ctx, id, dir, prompt);
        if self.sandbox == "wrap" {
            if let Some(first) = inner.first_mut() {
                *first = resolve_binary(first);
            }
            return ReviewerCommand {
                argv: self.wrap(ctx, dir, &ctx.thread_dir(id), inner),
                kind: None,
            };
        }
        let kind = self.herdr_kind.clone().map(|kind| {
            let args = inner
                .iter()
                .skip(1)
                .filter(|a| a.as_str() != prompt)
                .cloned()
                .collect();
            (kind, args)
        });
        ReviewerCommand { argv: inner, kind }
    }

    /// The paths the wrapped agent may write: the thread directory, the project's build outputs,
    /// the caches, the agent's own state, the temporary directories.
    fn writable(&self, ctx: &Ctx, dir: &Path, thread_dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut inside: Vec<PathBuf> = vec![thread_dir.to_path_buf()];
        inside.extend(
            ctx.config
                .strings("/sandbox/writable_in_project")
                .iter()
                .map(|p| dir.join(p)),
        );
        let mut outside: Vec<PathBuf> = ctx
            .config
            .strings("/sandbox/writable_outside")
            .iter()
            .chain(self.writable.iter())
            .map(|p| expand_home(p))
            .collect();
        outside.push(PathBuf::from("/tmp"));
        if let Some(tmp) = env_nonempty("TMPDIR") {
            outside.push(PathBuf::from(tmp));
        }
        (inside, outside)
    }

    /// The agent's command inside the operating system's sandbox. On Linux, bubblewrap: the disk
    /// read-only, the writable paths outside the project, the project read-only again (it may sit
    /// under one of them, /tmp for one), then the thread directory and the build outputs.
    pub fn wrap(
        &self,
        ctx: &Ctx,
        dir: &Path,
        thread_dir: &Path,
        inner: Vec<String>,
    ) -> Vec<String> {
        let (inside, outside) = self.writable(ctx, dir, thread_dir);
        let s = |p: &Path| p.to_string_lossy().to_string();
        if cfg!(target_os = "macos") {
            let mut profile =
                String::from("(version 1)(allow default)(deny file-write*)(allow file-write*");
            profile.push_str(r#" (literal "/dev/null") (regex #"^/dev/tty") (subpath "/dev/fd") (subpath "/private/tmp") (subpath "/private/var/folders")"#);
            for path in outside.iter().chain(inside.iter()) {
                profile.push_str(&format!(" (subpath {})", Value::String(s(path))));
            }
            profile.push(')');
            let mut argv = vec![exe_sandbox("sandbox-exec"), "-p".into(), profile];
            argv.extend(inner);
            return argv;
        }
        let mut argv = vec![
            exe_sandbox("bwrap"),
            "--ro-bind".into(),
            "/".into(),
            "/".into(),
            "--dev-bind".into(),
            "/dev".into(),
            "/dev".into(),
        ];
        for path in &outside {
            argv.extend(["--bind-try".into(), s(path), s(path)]);
        }
        argv.extend(["--ro-bind".into(), s(dir), s(dir)]);
        if ctx.root != dir {
            argv.extend(["--ro-bind".into(), s(&ctx.root), s(&ctx.root)]);
        }
        for path in &inside {
            argv.extend(["--bind-try".into(), s(path), s(path)]);
        }
        argv.extend(["--chdir".into(), s(dir), "--".into()]);
        argv.extend(inner);
        argv
    }
}

/// The real binary behind a command name, for the sandbox: a version manager's shim (mise, asdf)
/// writes its own state when it runs, which the read-only disk refuses, so the agent's own binary
/// runs instead. A name that is not on the PATH stays as it is.
pub fn resolve_binary(command: &str) -> String {
    let path = if command.contains('/') {
        PathBuf::from(command)
    } else {
        match std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|d| d.join(command))
                .find(|p| p.is_file())
        }) {
            Some(found) => found,
            None => return command.to_string(),
        }
    };
    let is_shim = path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|d| d == "shims");
    if is_shim {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        for manager in ["mise", "asdf"] {
            if let Some(real) = crate::util::capture(manager, &["which", &name], None) {
                let real = real.trim();
                if !real.is_empty() && Path::new(real).is_file() {
                    return real.to_string();
                }
            }
        }
    }
    path.to_string_lossy().to_string()
}

/// The sandbox program: ROUNDS_SANDBOX names another one (the tests use a fake).
fn exe_sandbox(default: &str) -> String {
    env_nonempty("ROUNDS_SANDBOX").unwrap_or_else(|| default.to_string())
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
    fn a_command_resolves_to_its_path_and_an_unknown_one_stays() {
        assert_eq!(
            resolve_binary("no-such-command-anywhere"),
            "no-such-command-anywhere"
        );
        let sh = resolve_binary("sh");
        assert!(sh.starts_with('/') && sh.ends_with("/sh"), "{sh}");
    }

    #[test]
    fn every_preset_has_a_command_templates_and_a_known_sandbox() {
        for (name, preset) in presets().as_object().unwrap() {
            assert!(preset["command"].is_string(), "{name}");
            for key in ["interactive", "headless"] {
                let template = strings(preset, key);
                assert_eq!(
                    template.first().map(String::as_str),
                    Some("{command}"),
                    "{name} {key}"
                );
                assert!(template.contains(&"{prompt}".to_string()), "{name} {key}");
            }
            assert!(
                ["codex", "wrap", "none"].contains(&preset["sandbox"].as_str().unwrap()),
                "{name}"
            );
        }
    }
}
