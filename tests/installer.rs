//! The installer: a first install, an upgrade, worktrees, the skill, removal.

mod common;

use common::{Env, git, make_project};
use serde_json::{Value, json};

fn exclude_lines(project: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(project.join(".git/info/exclude"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .collect()
}

#[test]
fn a_first_install_copies_the_kit_ignores_it_and_detects_the_toolchain() {
    let env = Env::bare();
    std::fs::write(env.project.join("Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(env.project.join("AGENTS.md"), "rules\n").unwrap();
    env.install(&env.project, &[]);
    let kit = env.project.join(".reviewright");
    for file in [
        "bin/review",
        "README.md",
        "REVIEWER.md",
        "SHARED.md",
        "config.json",
    ] {
        assert!(kit.join(file).exists(), "{file}");
    }
    let review = std::process::Command::new(kit.join("bin/review"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&review.stdout).starts_with("reviewright "));
    assert!(kit.join("threads").is_dir());
    assert!(exclude_lines(&env.project).contains(&".reviewright".to_string()));
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(kit.join("config.json")).unwrap()).unwrap();
    assert_eq!(config["read_first"], json!(["AGENTS.md"]));
    assert_eq!(config["sandbox"]["writable_in_project"], json!(["target"]));
    assert!(
        config["sandbox"]["writable_outside"]
            .as_array()
            .unwrap()
            .contains(&json!("~/.cargo"))
    );
    assert!(
        env.home
            .join(".claude/skills/reviewright/SKILL.md")
            .exists(),
        "the skill goes to the user by default"
    );
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&env.project)
        .args(["status", "--short"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&status.stdout).contains("reviewright"));
}

#[test]
fn an_upgrade_replaces_the_tool_and_keeps_the_config_and_the_threads() {
    let env = Env::new();
    let kit = env.project.join(".reviewright");
    std::fs::write(kit.join("config.json"), r#"{"checks": ["make test"]}"#).unwrap();
    std::fs::write(kit.join("REVIEWER.md"), "old").unwrap();
    std::fs::create_dir_all(kit.join("threads/20260101-keep")).unwrap();
    let out = env.install(&env.project, &["--skill", "none"]);
    assert!(out.text.contains("upgraded"), "{}", out.text);
    assert_ne!(
        std::fs::read_to_string(kit.join("REVIEWER.md")).unwrap(),
        "old"
    );
    assert_eq!(
        std::fs::read_to_string(kit.join("config.json")).unwrap(),
        r#"{"checks": ["make test"]}"#
    );
    assert!(kit.join("threads/20260101-keep").is_dir());
    assert_eq!(
        exclude_lines(&env.project)
            .iter()
            .filter(|l| *l == ".reviewright")
            .count(),
        1
    );
}

#[test]
fn the_installed_binary_upgrades_its_own_kit() {
    let env = Env::new();
    let out = std::process::Command::new(env.tool())
        .args(["install", "--no-doctor", "--skill", "none"])
        .current_dir(&env.project)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("upgraded"));
}

#[test]
fn installing_from_a_worktree_puts_the_kit_in_the_main_checkout() {
    let env = Env::bare();
    let tree = env.root.join("tree");
    git(
        &env.project,
        &["worktree", "add", "-q", tree.to_str().unwrap()],
    );
    env.install(&tree, &["--skill", "none"]);
    assert!(env.project.join(".reviewright/bin/review").exists());
    assert!(!tree.join(".reviewright").exists());
}

#[test]
fn a_project_skill_is_ignored_too_and_commit_mode_touches_no_exclude() {
    let env = Env::bare();
    env.install(&env.project, &["--skill", "project"]);
    assert!(
        env.project
            .join(".claude/skills/reviewright/SKILL.md")
            .exists()
    );
    assert!(exclude_lines(&env.project).contains(&".claude/skills/reviewright".to_string()));
    let other = make_project(&env.root.join("other"));
    env.install(&other, &["--commit", "--skill", "none"]);
    assert!(!exclude_lines(&other).contains(&".reviewright".to_string()));
}

#[test]
fn uninstall_keeps_the_threads_unless_purged_and_removes_links_and_excludes() {
    let env = Env::bare();
    env.install(&env.project, &["--skill", "project"]);
    let kit = env.project.join(".reviewright");
    let tree = env.root.join("tree");
    git(
        &env.project,
        &["worktree", "add", "-q", tree.to_str().unwrap()],
    );
    std::os::unix::fs::symlink(&kit, tree.join(".reviewright")).unwrap();
    std::fs::create_dir_all(kit.join("threads/20260101-keep")).unwrap();
    env.install(&env.project, &["--uninstall"]);
    let left: Vec<String> = std::fs::read_dir(&kit)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(left, vec!["threads"]);
    assert!(tree.join(".reviewright").symlink_metadata().is_err());
    assert!(!env.project.join(".claude/skills/reviewright").exists());
    assert!(!exclude_lines(&env.project).contains(&".reviewright".to_string()));
    env.install(&env.project, &["--uninstall", "--purge"]);
    assert!(!kit.exists());
}

#[test]
fn a_directory_outside_git_is_refused() {
    let env = Env::bare();
    let plain = env.root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let out = env.install_raw(&["install", plain.to_str().unwrap(), "--no-doctor"]);
    assert_ne!(out.code, 0);
    assert!(
        out.text.contains("not inside a git repository"),
        "{}",
        out.text
    );
}
