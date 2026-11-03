//! The thread store: where the kit is, thread directories, state files, request snapshots.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::config::Config;
use crate::util::{day_prefix, die, env_nonempty, git, now_iso, real};

pub const KIT_NAME: &str = ".review";
const STATE_FILE: &str = "state.json";

pub type State = Map<String, Value>;

pub struct Ctx {
    pub kit: PathBuf,
    pub root: PathBuf,
    pub threads: PathBuf,
    pub config: Config,
}

impl Ctx {
    /// The kit this command belongs to: the one around the running binary when it is a project's
    /// `.review/bin/review` (a worktree's symlink resolves to the main checkout), else the one
    /// of the git project around the working directory.
    pub fn locate() -> Ctx {
        let from_exe = std::env::current_exe()
            .ok()
            .map(|exe| real(&exe))
            .and_then(|exe| exe.parent()?.parent().map(Path::to_path_buf))
            .filter(|kit| kit.file_name().is_some_and(|n| n == KIT_NAME));
        let kit = from_exe.or_else(project_kit).unwrap_or_else(|| {
            die("review: rounds is not installed in this project (run `rounds install`)")
        });
        let root = kit
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| kit.clone());
        let threads = env_nonempty("ROUNDS_THREADS")
            .map(PathBuf::from)
            .unwrap_or_else(|| kit.join("threads"));
        let config = Config::load(&kit);
        Ctx {
            kit,
            root,
            threads,
            config,
        }
    }

    pub fn tool(&self) -> PathBuf {
        self.kit.join("bin").join("review")
    }

    /// The one dated directory whose suffix is the thread name, or today's path when the thread
    /// does not exist yet.
    pub fn thread_dir(&self, id: &str) -> PathBuf {
        let suffix = format!("-{id}");
        let mut found: Vec<String> = std::fs::read_dir(&self.threads)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok()?.file_name().into_string().ok())
                    .filter(|name| {
                        name.len() == 8 + suffix.len()
                            && name.ends_with(&suffix)
                            && name[..8].bytes().all(|b| b.is_ascii_digit())
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        if found.len() > 1 {
            die(format!(
                "thread {id} has {} directories: {}",
                found.len(),
                found.join(", ")
            ));
        }
        self.threads.join(
            found
                .pop()
                .unwrap_or_else(|| format!("{}-{id}", day_prefix())),
        )
    }

    pub fn state_path(&self, id: &str) -> PathBuf {
        self.thread_dir(id).join(STATE_FILE)
    }

    pub fn exists(&self, id: &str) -> bool {
        self.state_path(id).exists()
    }

    pub fn read_state(&self, id: &str) -> State {
        let path = self.state_path(id);
        if !path.exists() {
            die(format!("no thread {id}"));
        }
        read_json(&path)
            .unwrap_or_else(|| die(format!("thread {id}: unreadable {}", path.display())))
    }

    /// Published by rename: a reader sees the old or the new document, never a part of one.
    pub fn write_state(&self, id: &str, state: &mut State) {
        state.insert("updated".into(), Value::String(now_iso()));
        let path = self.state_path(id);
        let tmp = path.with_file_name(format!(".{STATE_FILE}.{}.tmp", std::process::id()));
        let text = serde_json::to_string_pretty(state).expect("a JSON map serializes") + "\n";
        std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .unwrap_or_else(|e| die(format!("review: cannot write {}: {e}", path.display())));
    }

    pub fn round_path(&self, id: &str, round: u64, kind: &str) -> PathBuf {
        self.thread_dir(id).join(format!("{round:02}-{kind}.md"))
    }

    /// Every thread's state, in directory order (oldest day first).
    pub fn threads(&self) -> Vec<State> {
        let mut names: Vec<PathBuf> = std::fs::read_dir(&self.threads)
            .map(|entries| entries.filter_map(|e| Some(e.ok()?.path())).collect())
            .unwrap_or_default();
        names.sort();
        names
            .iter()
            .filter_map(|dir| read_json(&dir.join(STATE_FILE)))
            .collect()
    }

    /// The checkout the writer runs in: the main one or a worktree.
    pub fn checkout_dir(&self) -> PathBuf {
        let cwd = std::env::current_dir().unwrap_or_else(|_| self.root.clone());
        let top = git(&cwd, &["rev-parse", "--show-toplevel"]);
        if top.is_empty() {
            self.root.clone()
        } else {
            real(Path::new(&top))
        }
    }

    /// The revision and the tree of a checkout, as the lines of a request or a response.
    pub fn snapshot(&self, dir: &Path) -> String {
        let dirty = git(dir, &["status", "--short"]);
        let mut lines = Vec::new();
        if dir != self.root {
            lines.push(format!("- checkout: {}", dir.display()));
        }
        lines.push(format!(
            "- base: {} ({})",
            git(dir, &["rev-parse", "--short", "HEAD"]),
            git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        ));
        lines.push(if dirty.is_empty() {
            "- tree: clean".to_string()
        } else {
            format!("- tree: dirty\n\n```\n{dirty}\n```")
        });
        lines.join("\n")
    }

    /// The checkout a thread was asked from (the main one for old threads).
    pub fn dir_of(&self, state: &State) -> PathBuf {
        state
            .get("dir")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone())
    }
}

/// The kit of the git project around the working directory: `.review` beside the main
/// checkout's `.git`.
pub fn project_kit() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let common = git(
        &cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    if common.is_empty() {
        return None;
    }
    let kit = Path::new(&common).parent()?.join(KIT_NAME);
    kit.is_dir().then_some(kit)
}

pub fn read_json(path: &Path) -> Option<State> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

pub fn text<'a>(state: &'a State, key: &str) -> &'a str {
    state.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn round(state: &State) -> u64 {
    state.get("round").and_then(Value::as_u64).unwrap_or(0)
}

pub fn flag(state: &State, key: &str) -> bool {
    state.get(key).and_then(Value::as_bool).unwrap_or(false)
}
