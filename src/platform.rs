//! External reviews: the code platform a review is posted to.
//!
//! A thread started with `--pr <ref>` reviews someone else's change. When the review is done, the
//! writer stores the text for the change's author (`review draft`), the user reads it, and
//! `review post` posts it through the platform's command line, as the user's account, with
//! [`SIGNATURE`] at the bottom. rounds never posts by itself, and the reviewer cannot post (see
//! [`post_gate`]).
//!
//! A platform is a command and a `post` template, as the reviewer agents are. Template elements:
//! `{command}`, `{pr}` (the reference given to `--pr`), `{body_file}` (a file holding the text) and
//! `{body}` (the text itself).

use std::path::Path;

use serde_json::{Value, json};

use crate::store::Ctx;
use crate::util::{command_exists, die, env_nonempty};

pub const SIGNATURE: &str = "reviewed by 🎾rounds";

/// The platforms rounds knows. A plain comment review: rounds never approves or requests changes.
pub fn presets() -> Value {
    json!({
        "github": {
            "command": "gh",
            "post": ["{command}", "pr", "review", "{pr}", "--comment", "--body-file", "{body_file}"]
        },
        "gitlab": {
            "command": "glab",
            "post": ["{command}", "mr", "note", "{pr}", "--message", "{body}"]
        }
    })
}

pub struct Platform {
    pub name: String,
    pub command: String,
    post: Vec<String>,
}

/// The platform for a new external thread: the config's `platform` (or ROUNDS_PLATFORM).
pub fn configured(ctx: &Ctx) -> String {
    env_nonempty("ROUNDS_PLATFORM")
        .or_else(|| ctx.config.string("/platform"))
        .unwrap_or_else(|| "github".to_string())
}

/// The platform by name: the config's `platforms.<name>` over the preset of that name.
/// ROUNDS_PLATFORM_COMMAND overrides the command.
pub fn platform(ctx: &Ctx, name: &str) -> Option<Platform> {
    let preset = presets().get(name).cloned();
    let custom = ctx
        .config
        .value(&format!("/platforms/{name}"))
        .filter(|v| v.is_object())
        .cloned();
    if preset.is_none() && custom.is_none() {
        return None;
    }
    let pick = |key: &str| {
        custom
            .as_ref()
            .and_then(|c| c.get(key))
            .filter(|v| !v.is_null())
            .or_else(|| preset.as_ref().and_then(|p| p.get(key)))
            .cloned()
    };
    let command = env_nonempty("ROUNDS_PLATFORM_COMMAND")
        .or_else(|| pick("command").and_then(|v| v.as_str().map(str::to_string)))
        .unwrap_or_default();
    let post: Vec<String> = pick("post")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    Some(Platform {
        name: name.to_string(),
        command,
        post,
    })
}

impl Platform {
    /// The post command line for a change and a text.
    pub fn post_argv(&self, pr: &str, body: &str, body_file: &Path) -> Vec<String> {
        let file = body_file.to_string_lossy();
        self.post
            .iter()
            .map(|part| {
                part.replace("{command}", &self.command)
                    .replace("{pr}", pr)
                    .replace("{body_file}", &file)
                    .replace("{body}", body)
            })
            .collect()
    }

    pub fn usable(&self) -> Result<(), String> {
        if self.post.is_empty() {
            return Err(format!("platform {}: no `post` template", self.name));
        }
        if self.post.iter().any(|p| p.contains("{command}")) && !command_exists(&self.command) {
            return Err(format!(
                "platform {}: `{}` is not installed",
                self.name, self.command
            ));
        }
        Ok(())
    }
}

/// The text as it will be posted: the writer's text, then the signature.
pub fn signed(text: &str) -> String {
    let text = text.trim_end();
    if text.ends_with(SIGNATURE) {
        return format!("{text}\n");
    }
    format!("{text}\n\n---\n\n{SIGNATURE}\n")
}

/// The log of posts, in the kit's own store, locked for one post at a time. `review post` must
/// open it for writing before it posts. The reviewer's sandbox keeps the project's `.review`
/// read-only (only its own thread folder is writable), so a reviewer that runs `review post`
/// stops here. The path comes from the kit around the running binary, not from ROUNDS_THREADS,
/// and a post refuses another store: a store under /tmp would open the gate.
pub fn post_gate(ctx: &Ctx) -> std::fs::File {
    let store = ctx.kit.join("threads");
    if crate::util::real(&ctx.threads) != crate::util::real(&store) {
        die(format!(
            "review: `review post` uses the kit's own store ({}), not ROUNDS_THREADS",
            store.display()
        ));
    }
    let log = store.join(".posts");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .unwrap_or_else(|e| {
            die(format!(
                "review: cannot write {} ({e}); `review post` runs only outside the reviewer's sandbox",
                log.display()
            ))
        });
    file.lock()
        .unwrap_or_else(|e| die(format!("review: cannot lock {}: {e}", log.display())));
    file
}
