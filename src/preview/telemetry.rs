//! Browsing latency, as the preview saw it: when each `Show` arrived, when
//! its diff first reached the screen, and when it was fully colored; plus
//! how long each background prefetch took. Bounded, newest last. The
//! browsing-latency scenario writes it out as its artifact.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Records kept per kind, oldest dropped first.
const KEPT: usize = 256;

/// How long one `Show` took to reach the screen.
#[derive(Debug, Clone)]
pub struct ShowTiming {
    pub file: PathBuf,
    requested: Instant,
    /// Until the first diff for this Show was on screen; `None` =
    /// superseded before it landed.
    pub first_paint: Option<Duration>,
    /// Until every on-screen line of it carried syntax colors.
    pub colored: Option<Duration>,
}

#[derive(Debug, Default)]
pub struct Telemetry {
    shows: VecDeque<ShowTiming>,
    prefetches: VecDeque<(PathBuf, Duration)>,
}

impl Telemetry {
    pub fn shows(&self) -> &VecDeque<ShowTiming> {
        &self.shows
    }

    /// Background builds and how long each took.
    pub fn prefetches(&self) -> &VecDeque<(PathBuf, Duration)> {
        &self.prefetches
    }

    /// A Show for `file` arrived.
    pub fn requested(&mut self, file: &Path) {
        push(
            &mut self.shows,
            ShowTiming {
                file: file.to_path_buf(),
                requested: Instant::now(),
                first_paint: None,
                colored: None,
            },
        );
    }

    /// The newest Show's diff (for `file`) changed on screen; `colored`
    /// says whether it is now fully colored.
    pub fn painted(&mut self, file: &Path, colored: bool) {
        if let Some(t) = self.shows.back_mut()
            && t.file == file
        {
            let took = t.requested.elapsed();
            t.first_paint.get_or_insert(took);
            if colored && t.colored.is_none() {
                t.colored = Some(took);
            }
        }
    }

    pub fn prefetched(&mut self, file: PathBuf, took: Duration) {
        push(&mut self.prefetches, (file, took));
    }
}

fn push<T>(records: &mut VecDeque<T>, record: T) {
    if records.len() >= KEPT {
        records.pop_front();
    }
    records.push_back(record);
}
