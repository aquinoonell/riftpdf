//! Optional startup phase timing. Enable with `RIFTPDF_TIMING=1`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn init() {
    let on = std::env::var_os("RIFTPDF_TIMING").is_some_and(|v| v != "0");
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub struct PhaseLog {
    start: Instant,
    last: Instant,
}

impl Default for PhaseLog {
    fn default() -> Self {
        Self::new()
    }
}

impl PhaseLog {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last: now,
        }
    }

    pub fn mark(&mut self, label: &str) {
        if !enabled() {
            return;
        }
        let now = Instant::now();
        eprintln!(
            "[riftpdf timing] {label}: +{:.1}ms (total {:.1}ms)",
            now.duration_since(self.last).as_secs_f64() * 1000.0,
            now.duration_since(self.start).as_secs_f64() * 1000.0
        );
        self.last = now;
    }
}
