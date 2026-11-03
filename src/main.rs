//! rounds: review rounds between Claude Code (the writer) and Codex (the reviewer), one
//! sandboxed Codex session per review thread, through files in the project.
//!
//! One binary: `rounds install` puts the kit into a project, where the same binary runs as
//! `.review/bin/review` for the review commands.

mod args;
mod commands;
mod config;
mod install;
mod launcher;
mod store;
mod util;
mod watch;

fn main() {
    let mut words: Vec<String> = std::env::args().skip(1).collect();
    let command = if words.is_empty() {
        None
    } else {
        Some(words.remove(0))
    };
    match command.as_deref() {
        Some("install") => install::run(words),
        Some("uninstall") => {
            words.push("--uninstall".into());
            install::run(words)
        }
        Some("--version" | "-V" | "version") => {
            println!("rounds {}", env!("CARGO_PKG_VERSION"))
        }
        Some(name) if commands::is_command(name) => commands::dispatch(name, words),
        None | Some("help" | "--help" | "-h") => println!("{}", commands::USAGE),
        Some(_) => {
            println!("{}", commands::USAGE);
            std::process::exit(2);
        }
    }
}
