//! `rounds install`: puts the kit into a git project, upgrades it, or removes it.
//!
//! The kit goes into the main checkout's `.review/` (from any checkout or worktree): this
//! binary as `bin/review`, the protocol, the reviewer prompts, and a `config.json` written on the
//! first install with what can be detected. Git ignores the kit through `.git/info/exclude`
//! (unless --commit). The writer's skill goes to the user (`~/.claude/skills/rounds`, which
//! covers every worktree), to the project, or nowhere.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::args::Args;
use crate::store::KIT_NAME;
use crate::util::{die, git};

const README: &str = include_str!("../kit/README.md");
const REVIEWER: &str = include_str!("../kit/REVIEWER.md");
const SHARED: &str = include_str!("../kit/SHARED.md");
const WRITER: &str = include_str!("../kit/WRITER.md");
const SKILL_HEADER: &str = include_str!("../skill/header.md");
const SKILL_PATH: &str = ".claude/skills/rounds";
const USAGE: &str =
    "usage: rounds install [PROJECT] [--skill user|project|none] [--commit] [--no-doctor]";
const UNINSTALL_USAGE: &str = "usage: review uninstall [PROJECT] [--purge]";

fn say(text: impl AsRef<str>) {
    println!("rounds: {}", text.as_ref());
}

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, body)
        .unwrap_or_else(|e| die(format!("rounds: cannot write {}: {e}", path.display())));
}

/// Where the project's pieces are.
struct Project {
    main: PathBuf,
    kit: PathBuf,
    exclude: PathBuf,
}

impl Project {
    fn find(path: &Path) -> Project {
        let top = git(path, &["rev-parse", "--show-toplevel"]);
        if top.is_empty() {
            die(format!(
                "rounds: {} is not inside a git repository",
                path.display()
            ));
        }
        if git(path, &["rev-parse", "--is-bare-repository"]) == "true" {
            die("rounds: bare repositories are not supported; run it in a checkout");
        }
        let common = PathBuf::from(git(
            path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ));
        let main = if common.file_name().is_some_and(|n| n == ".git") {
            common
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(&top))
        } else {
            PathBuf::from(&top)
        };
        let main = crate::util::real(&main);
        Project {
            kit: main.join(KIT_NAME),
            exclude: common.join("info").join("exclude"),
            main,
        }
    }

    fn has(&self, file: &str) -> bool {
        self.main.join(file).exists()
    }
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".codex"))
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| die("rounds: HOME is not set"))
}

/// The first config: the agent instruction files, and the build outputs and caches of the
/// toolchains the project shows.
fn detected_config(project: &Project) -> Value {
    let mut config = crate::config::defaults();
    let read_first: Vec<&str> = ["AGENTS.md", "CLAUDE.md", "CONTRIBUTING.md"]
        .into_iter()
        .filter(|f| project.has(f))
        .collect();
    let (mut inside, mut outside): (Vec<&str>, Vec<&str>) = (Vec::new(), vec!["~/.cache"]);
    if project.has("Cargo.toml") {
        inside.push("target");
        outside.push("~/.cargo");
    }
    if project.has("package.json") {
        inside.push("node_modules/.cache");
        outside.push("~/.npm");
    }
    if project.has("Gemfile") {
        inside.extend(["tmp", "log"]);
        outside.extend(["~/.bundle", "~/.gem"]);
    }
    if project.has("build.gradle") || project.has("build.gradle.kts") {
        inside.push("build");
        outside.push("~/.gradle");
    }
    if project.has("go.mod") {
        outside.push("~/go/pkg");
    }
    if project.has("pom.xml") {
        outside.push("~/.m2");
    }
    if project.has("pyproject.toml") || project.has("setup.py") {
        inside.push(".pytest_cache");
    }
    if project.has(".mise.toml") || project.has(".tool-versions") {
        outside.extend(["~/.local/share/mise", "~/.local/state"]);
    }
    config["read_first"] = read_first.into();
    config["sandbox"]["writable_in_project"] = inside.into();
    config["sandbox"]["writable_outside"] = outside.into();
    config
}

fn exclude_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|t| t.lines().map(|l| l.trim().to_string()).collect())
        .unwrap_or_default()
}

fn add_excludes(path: &Path, wanted: &[&str]) {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    let lines = exclude_lines(path);
    let missing: Vec<&&str> = wanted
        .iter()
        .filter(|w| !lines.iter().any(|l| l == **w))
        .collect();
    if missing.is_empty() {
        return;
    }
    let mut text = current.clone();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# rounds\n");
    for line in missing {
        text.push_str(line);
        text.push('\n');
    }
    write(path, &text);
}

/// The project's name before rounds: its user skill would stay beside the new one.
const LEGACY_SKILL_PATH: &str = ".claude/skills/reviewright";

/// Removes the skill the earlier name installed, only when it is that skill.
fn remove_legacy_skill(dir: &Path) {
    let ours = std::fs::read_to_string(dir.join("SKILL.md"))
        .is_ok_and(|text| text.starts_with("---\nname: reviewright\n"));
    if ours && std::fs::remove_dir_all(dir).is_ok() {
        say(format!(
            "removed the skill of the earlier name: {}",
            dir.display()
        ));
    }
}

fn remove_excludes(path: &Path) {
    let Ok(current) = std::fs::read_to_string(path) else {
        return;
    };
    let kept: Vec<&str> = current
        .lines()
        .filter(|l| {
            ![
                KIT_NAME,
                SKILL_PATH,
                "# rounds",
                LEGACY_SKILL_PATH,
                "# reviewright",
            ]
            .contains(&l.trim())
        })
        .collect();
    let mut text = kept.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    write(path, &text);
}

/// Copies this binary to the kit by a rename, so an install run by the kit's own binary (an
/// upgrade through `.review/bin/review install`) never writes over a running file.
fn install_binary(kit: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let target = kit.join("bin").join("review");
    let me = std::env::current_exe()
        .unwrap_or_else(|e| die(format!("rounds: cannot find this binary: {e}")));
    if crate::util::real(&me) == crate::util::real(&target) {
        return;
    }
    let tmp = kit
        .join("bin")
        .join(format!(".review.{}.tmp", std::process::id()));
    std::fs::create_dir_all(kit.join("bin"))
        .and_then(|()| std::fs::copy(&me, &tmp).map(|_| ()))
        .and_then(|()| std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)))
        .and_then(|()| std::fs::rename(&tmp, &target))
        .unwrap_or_else(|e| die(format!("rounds: cannot install {}: {e}", target.display())));
}

/// `review uninstall [PROJECT] [--purge]`: removes the kit (the threads stay unless --purge), the
/// worktree links, the project skill and the exclude lines.
pub fn uninstall_command(words: Vec<String>) {
    let args = Args::parse(words, &[], &["purge"], UNINSTALL_USAGE);
    let path = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let project = Project::find(&path);
    uninstall(
        &project,
        &home().join(SKILL_PATH),
        &project.main.join(SKILL_PATH),
        args.flag("purge"),
    );
}

/// `rounds install`. It still takes `--uninstall [--purge]`, the form before `review uninstall`.
pub fn run(words: Vec<String>) {
    let args = Args::parse(
        words,
        &["skill"],
        &["commit", "no-doctor", "uninstall", "purge"],
        USAGE,
    );
    let path = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let project = Project::find(&path);
    let skill = args.value("skill").unwrap_or("user");
    if !["user", "project", "none"].contains(&skill) {
        die(USAGE);
    }
    let home_skill = home().join(SKILL_PATH);
    let project_skill = project.main.join(SKILL_PATH);

    if args.flag("uninstall") {
        uninstall(&project, &home_skill, &project_skill, args.flag("purge"));
        return;
    }

    let first = !project.kit.join("config.json").exists();
    // A `.review` that another tool made (no config.json of ours) is never written into.
    if first && project.kit.exists() {
        die(format!(
            "rounds: {} exists and was not made by rounds; move it away first",
            project.kit.display()
        ));
    }
    install_binary(&project.kit);
    let _ = std::fs::create_dir_all(project.kit.join("threads"));
    write(&project.kit.join("README.md"), README);
    write(&project.kit.join("REVIEWER.md"), REVIEWER);
    write(&project.kit.join("SHARED.md"), SHARED);
    write(&project.kit.join("WRITER.md"), WRITER);
    if first {
        let config = serde_json::to_string_pretty(&detected_config(&project)).expect("JSON") + "\n";
        write(&project.kit.join("config.json"), &config);
    }
    if !args.flag("commit") {
        let mut wanted = vec![KIT_NAME];
        if skill == "project" {
            wanted.push(SKILL_PATH);
        }
        add_excludes(&project.exclude, &wanted);
    }
    // The writer's skill: WRITER.md under a skill header. For the user, it goes where Claude Code
    // reads skills, and where Codex does when Codex is there (either can be the writer).
    let mut skill_files: Vec<PathBuf> = match skill {
        "user" => vec![home_skill.join("SKILL.md")],
        "project" => vec![project_skill.join("SKILL.md")],
        _ => Vec::new(),
    };
    if skill == "user" && codex_home().is_dir() {
        skill_files.push(codex_home().join("skills").join("rounds").join("SKILL.md"));
    }
    let skill_text = format!("{SKILL_HEADER}{WRITER}");
    for file in &skill_files {
        write(file, &skill_text);
    }
    remove_legacy_skill(&home().join(LEGACY_SKILL_PATH));

    say(format!(
        "{} in {}{}",
        if first { "installed" } else { "upgraded" },
        project.kit.display(),
        if first {
            ""
        } else {
            " (config.json and threads kept)"
        }
    ));
    say(if skill_files.is_empty() {
        "skill: not installed".to_string()
    } else {
        let files: Vec<String> = skill_files
            .iter()
            .map(|f| f.display().to_string())
            .collect();
        format!("skill: {}", files.join(", "))
    });
    say(if args.flag("commit") {
        format!("not ignored by git: commit {KIT_NAME} yourself")
    } else {
        format!("ignored by git through {}", project.exclude.display())
    });
    say(format!(
        "next: set `checks` in {}, then run {KIT_NAME}/bin/review selftest",
        project.kit.join("config.json").display()
    ));
    if !args.flag("no-doctor") {
        println!();
        let code = Command::new(project.kit.join("bin").join("review"))
            .arg("doctor")
            .current_dir(&project.main)
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(1);
        std::process::exit(code);
    }
}

fn uninstall(project: &Project, home_skill: &Path, project_skill: &Path, purge: bool) {
    if purge {
        let _ = std::fs::remove_dir_all(&project.kit);
    } else if let Ok(entries) = std::fs::read_dir(&project.kit) {
        for entry in entries.flatten() {
            if entry.file_name() != "threads" {
                let path = entry.path();
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
            }
        }
        let _ = std::fs::remove_dir(&project.kit); // only when no threads are left
    }
    let trees = git(&project.main, &["worktree", "list", "--porcelain"]);
    for tree in trees.lines().filter_map(|l| l.strip_prefix("worktree ")) {
        let link = Path::new(tree).join(KIT_NAME);
        if link
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            let _ = std::fs::remove_file(&link);
        }
    }
    let _ = std::fs::remove_dir_all(project_skill);
    if home_skill.join("SKILL.md").exists() {
        say(format!(
            "the user-level skill {} serves every project; remove it by hand when no project uses rounds",
            home_skill.display()
        ));
    }
    remove_excludes(&project.exclude);
    if project.kit.exists() {
        say(format!(
            "removed from {}; the threads stay in {} (--purge removes them)",
            project.main.display(),
            project.kit.join("threads").display()
        ));
    } else {
        say(format!("removed from {}", project.main.display()));
    }
}
