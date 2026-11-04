//! Small helpers: errors that end the command, UTC time, shell quoting, external commands.

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Prints the message on stderr and ends the command with exit code 1.
pub fn die(message: impl Display) -> ! {
    eprintln!("{message}");
    std::process::exit(1)
}

/// The UTC calendar fields of now: year, month, day, hour, minute, second.
pub fn utc_now() -> (i64, u32, u32, u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (rest / 3600) as u32,
        (rest % 3600 / 60) as u32,
        (rest % 60) as u32,
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Now as ISO 8601 UTC, to the second: 2026-11-03T14:20:41Z.
pub fn now_iso() -> String {
    let (y, mo, d, h, mi, s) = utc_now();
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Today in UTC as YYYYMMDD, the prefix of a thread directory.
pub fn day_prefix() -> String {
    let (y, mo, d, ..) = utc_now();
    format!("{y:04}{mo:02}{d:02}")
}

/// Now in UTC as HHMMSS.
pub fn clock() -> String {
    let (.., h, mi, s) = utc_now();
    format!("{h:02}{mi:02}{s:02}")
}

/// A duration in a few characters: 45s, 4m, 1h20m, 2d3h.
pub fn duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d{}h", s / 86_400, s % 86_400 / 3600),
    }
}

/// A time as UTC to the minute: 2026-11-04 14:32Z.
pub fn utc_minute(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rest = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60
    )
}

/// FNV-1a: a hash that stays the same across builds (the checks lock's file name).
pub fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Quotes one word for a POSIX shell; safe words stay as they are.
pub fn shell_quote(word: &str) -> String {
    if word.is_empty() {
        return "''".to_string();
    }
    let safe = word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
    if safe {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

pub fn shell_join<S: AsRef<str>>(words: &[S]) -> String {
    words
        .iter()
        .map(|w| shell_quote(w.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A TOML basic string (JSON's string syntax is valid TOML for these values).
pub fn toml_string(value: &str) -> String {
    Value::String(value.to_string()).to_string()
}

/// Runs a program and returns its stdout when it succeeds; stderr is discarded.
pub fn capture(program: &str, args: &[&str], cwd: Option<&Path>) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let output = command.output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

/// Runs a program that answers JSON; None on a failure or an `error` document.
pub fn json_command(program: &str, args: &[&str], cwd: Option<&Path>) -> Option<Value> {
    let out = capture(program, args, cwd)?;
    let doc: Value = serde_json::from_str(out.trim()).ok()?;
    if doc.get("error").is_some() {
        None
    } else {
        Some(doc)
    }
}

/// `git -C dir args...`, trimmed stdout, or "" on any failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let dir = dir.to_string_lossy();
    let mut all = vec!["-C", dir.as_ref()];
    all.extend_from_slice(args);
    capture("git", &all, None)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Whether a program can be run: a path to an executable file, or a name on the PATH.
pub fn command_exists(name: &str) -> bool {
    if name.contains('/') {
        return is_executable(Path::new(name));
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(name))))
        .unwrap_or(false)
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// The value of an environment variable when it is set and not empty.
pub fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The canonical form of a path, or the path itself when it cannot be resolved.
pub fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_match_known_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(20_760), (2026, 11, 3));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn shell_quoting_round_trips_through_shlex() {
        for word in ["plain", "with space", "it's", "", "$HOME", "a\"b"] {
            assert_eq!(
                shlex::split(&shell_quote(word)).unwrap(),
                vec![word.to_string()]
            );
        }
    }
}
