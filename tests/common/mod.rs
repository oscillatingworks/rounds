//! Shared setup: every test gets a fresh git project with reviewright installed by the real
//! installer, a private HOME, and the fake (examples/fake.rs) linked under the names of the
//! programs reviewright drives.

#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

pub const FAKES: [&str; 5] = ["hc", "herdr-app", "herdr", "tmux", "codex"];
const CLEARED: [&str; 12] = [
    "REVIEWRIGHT_LAUNCHER",
    "REVIEWRIGHT_SPACE",
    "REVIEWRIGHT_THREADS",
    "REVIEWRIGHT_POLL",
    "REVIEWRIGHT_CODEX_ARGS",
    "HC_TERMINAL_ID",
    "HERDR_ENV",
    "HERDR_WORKSPACE_ID",
    "TMUX",
    "FAKE_SANDBOX_OPEN",
    "REVIEWRIGHT_HERDR",
    "REVIEWRIGHT_TMUX",
];

pub struct Env {
    pub tmp: TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub fake_dir: PathBuf,
    pub project: PathBuf,
    pub fakes: PathBuf,
}

pub fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_reviewright"))
}

/// The fake, built once per test run: `cargo test --test <one>` does not build examples, and a
/// stale fake would test the tool against old behavior.
fn fake_binary() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let status = Command::new(cargo)
                .args(["build", "--quiet", "--example", "fake"])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .status()
                .unwrap();
            assert!(status.success(), "cargo build --example fake failed");
            let fake = binary().parent().unwrap().join("examples").join("fake");
            assert!(fake.exists(), "{} is missing", fake.display());
            fake
        })
        .clone()
}

pub fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

pub fn make_project(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q"]);
    std::fs::write(dir.join("README.md"), "a project\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", "first"]);
    dir.canonicalize().unwrap()
}

pub struct Out {
    pub text: String,
    pub code: i32,
}

impl Env {
    /// A project with reviewright installed (no skill, no doctor).
    pub fn new() -> Env {
        let env = Env::bare();
        env.install(&env.project, &["--skill", "none"]);
        env
    }

    /// A project without reviewright.
    pub fn bare() -> Env {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (home, fake_dir, fakes) = (root.join("home"), root.join("fake"), root.join("fakes"));
        for dir in [&home, &fake_dir, &fakes] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let fake = fake_binary();
        for name in FAKES {
            std::os::unix::fs::symlink(&fake, fakes.join(name)).unwrap();
        }
        let project = make_project(&root.join("project"));
        Env {
            tmp,
            root,
            home,
            fake_dir,
            project,
            fakes,
        }
    }

    fn command(&self, program: &Path, args: &[&str], env: &[(&str, &str)], cwd: &Path) -> Command {
        let mut command = Command::new(program);
        command.args(args).current_dir(cwd);
        for name in CLEARED {
            command.env_remove(name);
        }
        command
            .env("HOME", &self.home)
            .env("FAKE_DIR", &self.fake_dir)
            .env("REVIEWRIGHT_CODEX", self.fakes.join("codex"))
            .env("REVIEWRIGHT_HC", self.fakes.join("hc"))
            .env("REVIEWRIGHT_HERDR_APP", self.fakes.join("herdr-app"))
            .env("REVIEWRIGHT_HERDR", self.fakes.join("herdr"))
            .env("REVIEWRIGHT_TMUX", self.fakes.join("tmux"));
        for (key, value) in env {
            command.env(key, value);
        }
        command
    }

    fn output(mut command: Command, stdin: Option<&str>) -> Out {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        if let Some(text) = stdin {
            let _ = child.stdin.take().unwrap().write_all(text.as_bytes()); // the tool may exit before it reads
        }
        drop(child.stdin.take());
        let out = child.wait_with_output().unwrap();
        Out {
            text: String::from_utf8_lossy(&out.stdout).to_string()
                + &String::from_utf8_lossy(&out.stderr),
            code: out.status.code().unwrap_or(-1),
        }
    }

    pub fn install(&self, project: &Path, args: &[&str]) -> Out {
        let mut all = vec!["install", project.to_str().unwrap(), "--no-doctor"];
        all.extend_from_slice(args);
        let out = Self::output(self.command(&binary(), &all, &[], &self.root), None);
        assert_eq!(out.code, 0, "{}", out.text);
        out
    }

    pub fn install_raw(&self, args: &[&str]) -> Out {
        Self::output(self.command(&binary(), args, &[], &self.root), None)
    }

    pub fn tool(&self) -> PathBuf {
        self.project.join(".review").join("bin").join("review")
    }

    pub fn run(&self, args: &[&str]) -> Out {
        self.run_with(args, None, &[], &self.project)
    }

    pub fn run_with(
        &self,
        args: &[&str],
        stdin: Option<&str>,
        env: &[(&str, &str)],
        cwd: &Path,
    ) -> Out {
        Self::output(self.command(&self.tool(), args, env, cwd), stdin)
    }

    pub fn request(&self, id: &str, body: &str) -> Out {
        self.run_with(
            &["request", id, "--title", &format!("T {id}")],
            Some(body),
            &[],
            &self.project,
        )
    }

    pub fn start(&self, id: &str, env: &[(&str, &str)]) -> Out {
        self.run_with(
            &["start", id, "--title", &format!("T {id}")],
            Some("please review"),
            env,
            &self.project,
        )
    }

    pub fn respond(&self, id: &str, verdict: &str, body: &str) -> Out {
        self.run_with(
            &["respond", id, "--verdict", verdict],
            Some(body),
            &[],
            &self.project,
        )
    }

    pub fn threads_dir(&self) -> PathBuf {
        self.project.join(".review").join("threads")
    }

    pub fn thread_path(&self, id: &str) -> PathBuf {
        std::fs::read_dir(self.threads_dir())
            .unwrap()
            .filter_map(|e| Some(e.ok()?.path()))
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(&format!("-{id}"))
            })
            .unwrap_or_else(|| panic!("no thread {id}"))
    }

    pub fn state(&self, id: &str) -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(self.thread_path(id).join("state.json")).unwrap(),
        )
        .unwrap()
    }

    pub fn calls(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.fake_dir.join("calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    pub fn call(&self, prefix: &[&str]) -> Vec<String> {
        self.calls()
            .into_iter()
            .find(|c| c.len() >= prefix.len() && c.iter().zip(prefix).all(|(a, b)| a == b))
            .unwrap_or_else(|| panic!("no call {prefix:?}"))
    }

    pub fn fake_state(&self, name: &str) -> Value {
        std::fs::read_to_string(self.fake_dir.join(name))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!({}))
    }

    pub fn write_fake_state(&self, name: &str, value: &Value) {
        std::fs::write(self.fake_dir.join(name), value.to_string()).unwrap();
    }

    pub fn config(&self, edit: impl FnOnce(&mut Value)) {
        let path = self.project.join(".review").join("config.json");
        let mut config: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        edit(&mut config);
        std::fs::write(path, config.to_string()).unwrap();
    }
}

impl Drop for Env {
    /// Ends any headless reviewer a test left running.
    fn drop(&mut self) {
        let Ok(entries) = std::fs::read_dir(self.project.join(".review").join("threads")) else {
            return;
        };
        for entry in entries.flatten() {
            let state =
                std::fs::read_to_string(entry.path().join("state.json")).unwrap_or_default();
            if let Some(pid) = serde_json::from_str::<Value>(&state)
                .ok()
                .and_then(|s| s.pointer("/reviewer/pid")?.as_u64())
            {
                let _ = Command::new("kill")
                    .args(["-TERM", "--", &format!("-{pid}")])
                    .status();
            }
        }
    }
}

pub fn today() -> String {
    let out = Command::new("date")
        .args(["-u", "+%Y%m%d"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
