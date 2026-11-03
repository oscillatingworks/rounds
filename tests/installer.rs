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
    let kit = env.project.join(".review");
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
    assert!(String::from_utf8_lossy(&review.stdout).starts_with("rounds "));
    assert!(kit.join("threads").is_dir());
    assert!(exclude_lines(&env.project).contains(&".review".to_string()));
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
        env.home.join(".claude/skills/rounds/SKILL.md").exists(),
        "the skill goes to the user by default"
    );
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(&env.project)
        .args(["status", "--short"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&status.stdout).contains("rounds"));
}

#[test]
fn an_upgrade_replaces_the_tool_and_keeps_the_config_and_the_threads() {
    let env = Env::new();
    let kit = env.project.join(".review");
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
            .filter(|l| *l == ".review")
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
    assert!(env.project.join(".review/bin/review").exists());
    assert!(!tree.join(".review").exists());
}

#[test]
fn a_project_skill_is_ignored_too_and_commit_mode_touches_no_exclude() {
    let env = Env::bare();
    env.install(&env.project, &["--skill", "project"]);
    assert!(env.project.join(".claude/skills/rounds/SKILL.md").exists());
    assert!(exclude_lines(&env.project).contains(&".claude/skills/rounds".to_string()));
    let other = make_project(&env.root.join("other"));
    env.install(&other, &["--commit", "--skill", "none"]);
    assert!(!exclude_lines(&other).contains(&".review".to_string()));
}

#[test]
fn uninstall_keeps_the_threads_unless_purged_and_removes_links_and_excludes() {
    let env = Env::bare();
    env.install(&env.project, &["--skill", "project"]);
    let kit = env.project.join(".review");
    let tree = env.root.join("tree");
    git(
        &env.project,
        &["worktree", "add", "-q", tree.to_str().unwrap()],
    );
    std::os::unix::fs::symlink(&kit, tree.join(".review")).unwrap();
    std::fs::create_dir_all(kit.join("threads/20260101-keep")).unwrap();
    let out = env.run(&["uninstall"]);
    assert_eq!(out.code, 0, "{}", out.text);
    let left: Vec<String> = std::fs::read_dir(&kit)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(left, vec!["threads"]);
    assert!(tree.join(".review").symlink_metadata().is_err());
    assert!(!env.project.join(".claude/skills/rounds").exists());
    assert!(!exclude_lines(&env.project).contains(&".review".to_string()));
    let project = env.project.to_str().unwrap();
    let out = env.install_raw(&["uninstall", project, "--purge"]);
    assert_eq!(out.code, 0, "{}", out.text);
    assert!(!kit.exists());
}

#[test]
fn the_skill_of_the_earlier_name_is_replaced_and_another_skill_is_kept() {
    let env = Env::bare();
    let legacy = env.home.join(".claude/skills/reviewright");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(
        legacy.join("SKILL.md"),
        "---\nname: reviewright\n---\nold\n",
    )
    .unwrap();
    env.install(&env.project, &[]);
    assert!(!legacy.exists());
    assert!(env.home.join(".claude/skills/rounds/SKILL.md").exists());

    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("SKILL.md"), "---\nname: something-else\n---\n").unwrap();
    env.install(&env.project, &[]);
    assert!(legacy.exists(), "a skill that is not ours stays");
}

#[test]
fn a_review_directory_of_another_tool_is_left_alone() {
    let env = Env::bare();
    let other = env.project.join(".review");
    std::fs::create_dir_all(other.join("threads")).unwrap();
    std::fs::write(other.join("notes.md"), "someone else's\n").unwrap();
    let project = env.project.to_str().unwrap();
    let out = env.install_raw(&["install", project, "--no-doctor", "--skill", "none"]);
    assert_ne!(out.code, 0);
    assert!(out.text.contains("was not made by rounds"), "{}", out.text);
    assert!(!other.join("bin").exists());
    assert_eq!(
        std::fs::read_to_string(other.join("notes.md")).unwrap(),
        "someone else's\n"
    );
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

#[test]
fn the_writer_instructions_go_to_the_kit_and_the_skill_to_claude_and_codex() {
    let env = Env::bare();
    std::fs::create_dir_all(env.home.join(".codex")).unwrap();
    env.install(&env.project, &[]);
    let writer = std::fs::read_to_string(env.project.join(".review/WRITER.md")).unwrap();
    for skill in [
        ".claude/skills/rounds/SKILL.md",
        ".codex/skills/rounds/SKILL.md",
    ] {
        let text =
            std::fs::read_to_string(env.home.join(skill)).unwrap_or_else(|_| panic!("{skill}"));
        assert!(text.starts_with("---\nname: rounds\n"), "{skill}");
        assert!(text.ends_with(&writer), "{skill} carries WRITER.md");
    }
    let other = Env::bare();
    other.install(&other.project, &[]);
    assert!(
        !other.home.join(".codex").exists(),
        "no Codex folder is created"
    );
}

#[test]
fn install_still_takes_the_old_uninstall_flag() {
    let env = Env::new();
    env.install(&env.project, &["--uninstall", "--purge"]);
    assert!(!env.project.join(".review").exists());
}
