//! `SHAREZ_TIMING=1` prints how long each step of a capture takes, from the moment it was asked
//! for, to stderr. For finding out where the wait before the region overlay goes.

use std::{cell::Cell, sync::LazyLock, time::Instant};

static ENABLED: LazyLock<bool> =
    LazyLock::new(|| std::env::var_os("SHAREZ_TIMING").is_some_and(|v| v != "0"));

thread_local! {
    static START: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Starts timing a capture.
pub fn start(what: &str) {
    if *ENABLED {
        START.set(Some(Instant::now()));
        eprintln!("[timing] {what}: start");
    }
}

/// Notes that `step` finished, with the time since [`start`].
pub fn mark(step: &str) {
    if *ENABLED && let Some(start) = START.get() {
        eprintln!(
            "[timing] {step}: {:.0} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}

/// Prints `what` when timing is on, for following steps that aren't part of a capture.
pub fn note(what: &str) {
    if *ENABLED {
        eprintln!("[timing] {what}");
    }
}
