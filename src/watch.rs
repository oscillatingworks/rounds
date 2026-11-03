//! Waking on a write to the thread store instead of polling.
//!
//! The watch starts before the first state check, so a write that lands between the check and the
//! wait is an event already queued. It uses the platform's file events (inotify on Linux, FSEvents
//! on macOS); without them, or with REVIEWRIGHT_POLL set, the waits poll every five seconds. Every
//! wait re-checks the state at least once a minute anyway.

use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

const POLL: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_secs(60);

pub struct Watcher {
    events: Option<(RecommendedWatcher, Receiver<notify::Result<notify::Event>>)>,
}

impl Watcher {
    pub fn new(dir: &Path) -> Watcher {
        let _ = std::fs::create_dir_all(dir);
        if std::env::var_os("REVIEWRIGHT_POLL").is_some() {
            return Watcher { events: None };
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
        Watcher { events }
    }

    /// Blocks until an event lands or the time runs out (at most a minute; five seconds when
    /// polling).
    pub fn wait(&mut self, remaining: Duration) {
        let remaining = remaining.max(Duration::from_millis(100));
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
}
