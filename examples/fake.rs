//! Test double for the programs reviewright drives. The tests link it under the names `hc`,
//! `herdr-app`, `herdr`, `tmux` and `codex`; it acts by the name it runs under. Its state lives in
//! JSON files in $FAKE_DIR, and every call is appended to $FAKE_DIR/calls.jsonl.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

fn load(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

fn save(path: &Path, value: &Value) {
    fs::write(path, value.to_string()).expect("fake state");
}

fn ok(doc: Value) -> ! {
    println!("{doc}");
    std::process::exit(0)
}

fn fail() -> ! {
    println!("{}", json!({ "error": { "code": "not_found" } }));
    std::process::exit(1)
}

fn after<'a>(args: &'a [String], option: &str) -> &'a str {
    args.iter()
        .position(|a| a == option)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .unwrap_or("")
}

fn main() {
    let dir = PathBuf::from(std::env::var("FAKE_DIR").expect("FAKE_DIR"));
    let argv0 = std::env::args().next().unwrap_or_default();
    let tool = Path::new(&argv0)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut call = vec![tool.clone()];
    call.extend(args.clone());
    let log = dir.join("calls.jsonl");
    let mut text = fs::read_to_string(&log).unwrap_or_default();
    text.push_str(&format!("{}\n", json!(call)));
    fs::write(&log, &text).expect("calls");
    let a: Vec<String> = args.iter().filter(|x| *x != "--json").cloned().collect();
    let first = |i: usize| a.get(i).map(String::as_str).unwrap_or("");
    match tool.as_str() {
        "hc" => hc(&dir, &a, &text),
        "herdr-app" => herdr_app(&dir, &a),
        "herdr" => herdr(&dir, &a),
        "tmux" => tmux(&dir, &a),
        "codex" => match first(0) {
            "--version" => println!("codex-cli 0.0.0-fake"),
            "login" => println!("Logged in using ChatGPT"),
            "sandbox" => {
                println!("THREAD-WRITE-OK");
                let open = std::env::var_os("FAKE_SANDBOX_OPEN").is_some();
                println!(
                    "{}",
                    if open {
                        "PROJECT-WRITE-OPEN"
                    } else {
                        "PROJECT-READ-ONLY"
                    }
                );
            }
            "exec" => std::thread::sleep(std::time::Duration::from_secs(60)),
            _ => std::process::exit(1),
        },
        _ => std::process::exit(1),
    }
}

fn hc(dir: &Path, a: &[String], calls: &str) {
    let path = dir.join("terminals.json");
    let mut terms = load(&path);
    let map = terms.as_object_mut().unwrap();
    let target = a.get(1).cloned().unwrap_or_default();
    match a.first().map(String::as_str) {
        Some("run") => {
            let n = calls
                .lines()
                .filter(|l| l.starts_with(r#"["hc","run""#))
                .count();
            let address = format!("t{n}");
            map.insert(
                address.clone(),
                json!({ "name": after(a, "--name"), "state": "live", "tab": format!("tb{n}"), "rm": a.iter().any(|x| x == "--rm") }),
            );
            save(&path, &terms);
            ok(json!({ "address": address, "name": after(a, "--name"), "state": "live" }))
        }
        Some("get") => {
            let t = map
                .get(&target)
                .cloned()
                .unwrap_or_else(|| std::process::exit(1));
            ok(json!({ "address": target, "name": t["name"], "state": t["state"] }))
        }
        Some("kill") => {
            let rm = map.get(&target).unwrap_or_else(|| std::process::exit(1))["rm"] == true;
            if rm {
                map.remove(&target);
            } else {
                map[&target]["state"] = json!("finished");
            }
            save(&path, &terms);
            ok(json!({ "address": target }))
        }
        Some("rm") => {
            map.remove(&target).unwrap_or_else(|| std::process::exit(1));
            save(&path, &terms);
            ok(json!({ "removed": target }))
        }
        _ => std::process::exit(1),
    }
}

fn herdr_app(dir: &Path, a: &[String]) {
    let path = dir.join("terminals.json");
    let mut terms = load(&path);
    match a.first().map(String::as_str) {
        Some("ls") => {
            let mut tabs =
                vec![json!({ "id": "tbW", "name": "1", "terminals": [{ "terminal": "t99" }] })];
            for (address, t) in terms.as_object().unwrap() {
                tabs.push(json!({ "id": t["tab"], "name": t.get("tab_name").cloned().unwrap_or(json!("2")), "terminals": [{ "terminal": address }] }));
            }
            ok(json!({ "spaces": [{ "id": "spW", "folder": "/elsewhere", "tabs": tabs }] }))
        }
        Some("do") => {
            let (tab, name) = (after(a, "--id"), after(a, "--name"));
            for t in terms.as_object_mut().unwrap().values_mut() {
                if t["tab"] == tab {
                    t["tab_name"] = json!(name);
                }
            }
            save(&path, &terms);
            ok(json!({ "ok": true }))
        }
        _ => std::process::exit(1),
    }
}

fn herdr(dir: &Path, a: &[String]) {
    let path = dir.join("herdr.json");
    let mut st = load(&path);
    for key in ["tabs", "agents"] {
        if st.get(key).is_none() {
            st[key] = json!({});
        }
    }
    let result = |r: Value| ok(json!({ "result": r }));
    let words: Vec<&str> = a.iter().take(2).map(String::as_str).collect();
    match words.as_slice() {
        ["worktree", "list"] => {
            let cwd = std::env::current_dir().unwrap();
            result(
                json!({ "source": { "source_workspace_id": "w1" }, "worktrees": [{ "path": cwd, "open_workspace_id": "w9" }] }),
            )
        }
        ["tab", "create"] => {
            let n = st["n"].as_u64().unwrap_or(0) + 1;
            st["n"] = json!(n);
            let id = format!("w9:t{n}");
            st["tabs"][&id] = json!({ "label": after(a, "--label"), "pane": format!("w9:p{n}") });
            save(&path, &st);
            result(
                json!({ "tab": { "tab_id": id }, "root_pane": { "pane_id": format!("w9:p{n}") } }),
            )
        }
        ["agent", "start"] => {
            st["agents"][&a[2]] = json!(after(a, "--pane"));
            save(&path, &st);
            result(json!({ "type": "agent_started" }))
        }
        ["agent", "prompt"] => result(json!({ "type": "ok" })),
        ["agent", "get"] => match st["agents"].get(&a[2]) {
            Some(pane) => result(json!({ "agent": { "name": a[2], "pane_id": pane } })),
            None => fail(),
        },
        ["tab", "get"] => match st["tabs"].get(&a[2]) {
            Some(tab) => result(json!({ "tab": { "tab_id": a[2], "label": tab["label"] } })),
            None => fail(),
        },
        ["tab", "close"] => {
            let Some(tab) = st["tabs"].as_object_mut().unwrap().remove(&a[2]) else {
                fail()
            };
            st["agents"]
                .as_object_mut()
                .unwrap()
                .retain(|_, pane| *pane != tab["pane"]);
            save(&path, &st);
            result(json!({ "type": "ok" }))
        }
        _ => fail(),
    }
}

fn tmux(dir: &Path, a: &[String]) {
    let path = dir.join("tmux.json");
    let mut windows: Map<String, Value> = load(&path).as_object().cloned().unwrap_or_default();
    match a.first().map(String::as_str) {
        Some("new-window") => {
            let id = format!("@{}", windows.len() + 1);
            windows.insert(id.clone(), json!("0"));
            save(&path, &Value::Object(windows));
            println!("{id}");
        }
        Some("list-windows") => {
            let with_dead = a.last().is_some_and(|f| f.contains("pane_dead"));
            for (id, dead) in &windows {
                if with_dead {
                    println!("{id} {}", dead.as_str().unwrap_or("0"));
                } else {
                    println!("{id}");
                }
            }
        }
        Some("kill-window") => {
            windows
                .remove(after(a, "-t"))
                .unwrap_or_else(|| std::process::exit(1));
            save(&path, &Value::Object(windows));
        }
        _ => std::process::exit(1),
    }
}
