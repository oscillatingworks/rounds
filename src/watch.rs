//! Waking on a write to the thread store instead of polling.
//!
//! The watch starts before the first state check, so a write that lands between the check and the
//! wait is an event already queued. It uses the platform's file events (inotify on Linux, FSEvents
//! on macOS); without them, or with ROUNDS_POLL set, the waits poll every five seconds. Every
//! wait re-checks the state at least once a minute anyway.
//!
//! A wait also notes the machine's suspends, so `review status` can leave them out of the review
//! time. The monotonic clock stops while the machine sleeps and the wall clock does not; when the
//! wall clock gets ahead by more than [`SUSPEND`] between two wakes, the wait appends
//! `<from> <to> <gap>` (UNIX seconds) to the first `.suspends` file it can write: the store's, or
//! the thread's own (a sandboxed reviewer can write only there). The record says that the window
//! from the previous wake to this one held `gap` seconds of suspend, and the rest of it, up to a
//! minute, awake. Several waits can see the same suspend, or one wait two suspends as one gap, so
//! [`suspended`] only adds up records whose windows do not overlap.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

const POLL: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_secs(60);
const SUSPEND: Duration = Duration::from_secs(30);
pub const SUSPENDS_FILE: &str = ".suspends";

pub struct Watcher {
    events: Option<(RecommendedWatcher, Receiver<notify::Result<notify::Event>>)>,
    records: Vec<PathBuf>,
    last: (SystemTime, Instant),
}

impl Watcher {
    /// Watches `dir`; a suspend goes to the first of `records` that takes it.
    pub fn new(dir: &Path, records: Vec<PathBuf>) -> Watcher {
        let _ = std::fs::create_dir_all(dir);
        let last = (SystemTime::now(), Instant::now());
        if std::env::var_os("ROUNDS_POLL").is_some() {
            return Watcher {
                events: None,
                records,
                last,
            };
        }
        let (tx, rx) = channel();
        let events = notify::recommended_watcher(tx)
            .ok()
            .and_then(|mut watcher| {
                watcher
                    .watch(dir, RecursiveMode::Recursive)
                    .ok()
                    .map(|()| (watcher, rx))
            });
        Watcher {
            events,
            records,
            last,
        }
    }

    /// Blocks until an event lands or the time runs out (at most a minute; five seconds when
    /// polling).
    pub fn wait(&mut self, remaining: Duration) {
        self.block(remaining.max(Duration::from_millis(100)));
        self.note_suspend();
    }

    fn block(&mut self, remaining: Duration) {
        let Some((_, rx)) = &self.events else {
            std::thread::sleep(remaining.min(POLL));
            return;
        };
        match rx.recv_timeout(remaining.min(QUIET)) {
            Ok(_) => while rx.try_recv().is_ok() {},
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                self.events = None;
                std::thread::sleep(remaining.min(POLL));
            }
        }
    }

    fn note_suspend(&mut self) {
        let (wall, mono) = (SystemTime::now(), Instant::now());
        let last = self.last.0;
        let gap = suspend_gap(
            wall.duration_since(last).unwrap_or_default(),
            mono - self.last.1,
        );
        self.last = (wall, mono);
        let (Some(gap), Ok(from), Ok(to)) = (
            gap,
            last.duration_since(UNIX_EPOCH),
            wall.duration_since(UNIX_EPOCH),
        ) else {
            return;
        };
        let line = record(from, to, gap);
        for path in &self.records {
            let written = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| file.write_all(line.as_bytes()));
            if written.is_ok() {
                return;
            }
        }
    }
}

/// A `.suspends` line. Whole seconds, both window ends down: the windows of one wait touch and
/// do not overlap.
fn record(from: Duration, to: Duration, gap: Duration) -> String {
    format!("{} {} {}\n", from.as_secs(), to.as_secs(), gap.as_secs())
}

/// The time the machine slept between two wakes: how far the wall clock got ahead of the
/// monotonic one, when it is more than a clock adjustment.
fn suspend_gap(wall: Duration, mono: Duration) -> Option<Duration> {
    wall.checked_sub(mono).filter(|gap| *gap > SUSPEND)
}

/// One record: the window from `from` to `to` (UNIX seconds) held `gap` seconds of suspend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Suspend {
    pub from: u64,
    pub to: u64,
    pub gap: u64,
}

/// Every record the waits noted in a store: its own file and each thread's.
pub fn suspends(threads: &Path) -> Vec<Suspend> {
    let mut files = vec![threads.join(SUSPENDS_FILE)];
    if let Ok(entries) = std::fs::read_dir(threads) {
        files.extend(entries.filter_map(|e| Some(e.ok()?.path().join(SUSPENDS_FILE))));
    }
    let mut records: Vec<Suspend> = files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .flat_map(|text| text.lines().filter_map(parse).collect::<Vec<Suspend>>())
        .collect();
    records.sort_unstable_by_key(|r| (r.from, r.to, r.gap));
    records
}

fn parse(line: &str) -> Option<Suspend> {
    let mut fields = line.split_whitespace().map(|f| f.parse::<u64>().ok());
    let (from, to, gap) = (fields.next()??, fields.next()??, fields.next()??);
    (from < to && gap > 0 && fields.next().is_none()).then(|| Suspend {
        from,
        to,
        gap: gap.min(to - from),
    })
}

/// The suspended time certain to lie between `from` and `to`, never more than the real one.
///
/// A record proves, inside the span, the part of its window in the span less the window's awake
/// time. Records whose windows do not overlap saw different suspends, so their proofs add up;
/// records whose windows overlap may have seen the same one. The result is the largest sum over
/// records with disjoint windows (weighted interval scheduling).
pub fn suspended(suspends: &[Suspend], from: SystemTime, to: SystemTime) -> Duration {
    let secs = |t: SystemTime| t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (from, to) = (secs(from), secs(to));
    let mut proofs: Vec<(u64, u64, u64)> = suspends
        .iter()
        .filter_map(|s| {
            let inside = s.to.min(to).saturating_sub(s.from.max(from));
            let proof = inside.saturating_sub(s.to - s.from - s.gap);
            (proof > 0).then_some((s.from, s.to, proof))
        })
        .collect();
    proofs.sort_unstable_by_key(|p| p.1);
    // best[i]: the largest sum over the first i windows (by end).
    let mut best = vec![0u64; proofs.len() + 1];
    for (i, &(start, _, proof)) in proofs.iter().enumerate() {
        let before = proofs.partition_point(|p| p.1 <= start);
        best[i + 1] = best[i].max(best[before] + proof);
    }
    Duration::from_secs(best[proofs.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_counts_only_beyond_a_clock_adjustment() {
        let s = Duration::from_secs;
        assert_eq!(suspend_gap(s(60), s(60)), None);
        assert_eq!(suspend_gap(s(80), s(60)), None);
        assert_eq!(suspend_gap(s(60), s(80)), None);
        assert_eq!(suspend_gap(s(3660), s(60)), Some(s(3600)));
    }

    fn store(files: &[(&str, &str)]) -> Vec<Suspend> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rounds-suspends-{}-{n}", std::process::id()));
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let found = suspends(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        found
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn deducted(files: &[(&str, &str)], from: u64, to: u64) -> u64 {
        suspended(&store(files), at(from), at(to)).as_secs()
    }

    #[test]
    fn waits_that_wake_at_different_times_count_one_suspend_once() {
        // One 40-second suspend from 100 to 140. One wait last woke at 95 and wakes 1 second after
        // the resume; another last woke at 99 and wakes 59 seconds after it.
        let files = [
            (SUSPENDS_FILE, "95 141 40\n"),
            ("20260101-a/.suspends", "99 199 40\n"),
        ];
        assert_eq!(store(&files).len(), 2);
        assert_eq!(deducted(&files, 0, 1000), 40);
        // A round that starts after the resume loses no awake time.
        assert_eq!(deducted(&files, 150, 1000), 0);
        // A round that starts during the suspend: only the part certain to be inside it.
        assert_eq!(deducted(&files, 120, 1000), 19);
    }

    #[test]
    fn one_wait_that_sees_two_suspends_in_a_row_counts_both() {
        // Wakes at 0.2, 3660.2 and 7320.2 s, each after 3600 s asleep and 60 s awake: the lines
        // the wait writes have windows that touch.
        let ms = Duration::from_millis;
        let lines = record(ms(200), ms(3_660_200), ms(3_600_000))
            + &record(ms(3_660_200), ms(7_320_200), ms(3_600_000));
        assert_eq!(lines, "0 3660 3600\n3660 7320 3600\n");
        assert_eq!(deducted(&[(SUSPENDS_FILE, &lines)], 0, 8000), 7200);
    }

    #[test]
    fn a_wait_that_sees_two_suspends_as_one_gap_does_not_double_them() {
        // Asleep [59, 3659], awake 2 s, asleep [3661, 7261]. One wait saw them apart, another
        // as one gap; and a third wait saw only the first.
        let files = [
            (SUSPENDS_FILE, "0 3660 3600\n3660 7320 3600\n"),
            ("20260101-a/.suspends", "58 7318 7200\n"),
            ("20260101-b/.suspends", "30 3700 3600\n"),
        ];
        assert_eq!(deducted(&files, 0, 8000), 7200);
        assert_eq!(deducted(&files[1..], 0, 8000), 7200);
        assert_eq!(deducted(&files[1..2], 0, 8000), 7200);
    }

    #[test]
    fn separate_suspends_add_up_and_junk_is_skipped() {
        let files = [(
            SUSPENDS_FILE,
            "100 200 90\n500 600 60\nnoise\n1 2\n9 8 1\n10 20 0\n10 20 30 40\n",
        )];
        assert_eq!(store(&files).len(), 2);
        assert_eq!(deducted(&files, 0, 1000), 150);
        assert_eq!(deducted(&files, 300, 400), 0);
        assert_eq!(deducted(&[], 0, 1000), 0);
    }
}
