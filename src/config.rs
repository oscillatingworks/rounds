//! `.reviewright/config.json`, merged over the defaults.

use std::path::Path;

use serde_json::{Value, json};

use crate::util::die;

pub struct Config(Value);

pub fn defaults() -> Value {
    json!({
        "launcher": "auto",
        "reviewer": { "command": "codex", "args": [] },
        "read_first": [],
        "checks": [],
        "sandbox": {
            "network": true,
            "writable_in_project": [],
            "writable_outside": ["~/.cache"]
        },
        "command_launcher": { "start": null, "stop": null, "alive": null },
        "max_rounds": 6
    })
}

fn merge(base: &mut Value, extra: Value) {
    match (base, extra) {
        (Value::Object(base), Value::Object(extra)) => {
            for (key, value) in extra {
                match base.get_mut(&key) {
                    Some(slot) if slot.is_object() && value.is_object() => merge(slot, value),
                    _ => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (slot, value) => *slot = value,
    }
}

impl Config {
    pub fn load(kit: &Path) -> Config {
        let mut config = defaults();
        let path = kit.join("config.json");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let extra: Value = serde_json::from_str(&text).unwrap_or_else(|e| {
                die(format!("review: {} is not valid JSON: {e}", path.display()))
            });
            merge(&mut config, extra);
        }
        Config(config)
    }

    fn get(&self, pointer: &str) -> Option<&Value> {
        self.0.pointer(pointer).filter(|v| !v.is_null())
    }

    pub fn string(&self, pointer: &str) -> Option<String> {
        self.get(pointer)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    pub fn strings(&self, pointer: &str) -> Vec<String> {
        self.get(pointer)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn bool(&self, pointer: &str) -> bool {
        self.get(pointer).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn max_rounds(&self) -> u64 {
        self.get("/max_rounds").and_then(Value::as_u64).unwrap_or(6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_config_keeps_the_other_defaults() {
        let mut config = defaults();
        merge(
            &mut config,
            json!({ "sandbox": { "network": false }, "checks": ["make test"] }),
        );
        let config = Config(config);
        assert!(!config.bool("/sandbox/network"));
        assert_eq!(
            config.strings("/sandbox/writable_outside"),
            vec!["~/.cache".to_string()]
        );
        assert_eq!(config.strings("/checks"), vec!["make test".to_string()]);
        assert_eq!(config.string("/reviewer/command").as_deref(), Some("codex"));
    }
}
