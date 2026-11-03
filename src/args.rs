//! A small option parser: `--name value`, `--name=value` and `--flag`, anywhere among the words.

use std::collections::{HashMap, HashSet};

use crate::util::die;

pub struct Args {
    positional: Vec<String>,
    values: HashMap<String, String>,
    flags: HashSet<String>,
}

impl Args {
    /// `with_value` names the options that take a value, `flags` the ones that do not.
    pub fn parse(words: Vec<String>, with_value: &[&str], flags: &[&str], usage: &str) -> Args {
        let mut args = Args {
            positional: Vec::new(),
            values: HashMap::new(),
            flags: HashSet::new(),
        };
        let mut words = words.into_iter();
        while let Some(word) = words.next() {
            let Some(option) = word.strip_prefix("--") else {
                args.positional.push(word);
                continue;
            };
            let (name, inline) = match option.split_once('=') {
                Some((n, v)) => (n.to_string(), Some(v.to_string())),
                None => (option.to_string(), None),
            };
            if with_value.contains(&name.as_str()) {
                let value = inline
                    .or_else(|| words.next())
                    .unwrap_or_else(|| die(format!("--{name} needs a value\n{usage}")));
                args.values.insert(name, value);
            } else if flags.contains(&name.as_str()) && inline.is_none() {
                args.flags.insert(name);
            } else {
                die(format!("unknown option --{name}\n{usage}"));
            }
        }
        args
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    pub fn first(&self) -> Option<&str> {
        self.positional.first().map(String::as_str)
    }

    pub fn rest(&self) -> &[String] {
        self.positional.get(1..).unwrap_or(&[])
    }

    pub fn seconds(&self, name: &str, default: u64) -> u64 {
        self.value(name).map_or(default, |v| {
            v.parse()
                .unwrap_or_else(|_| die(format!("--{name} takes a number of seconds")))
        })
    }
}
