//! Minimal leveled logger (proxalyze style: no framework, stderr only).
//! Level is a process-wide AtomicUsize set once from --verbose.

use std::sync::atomic::{AtomicUsize, Ordering};

pub const LEVEL_INFO: usize = 0;
pub const LEVEL_DEBUG: usize = 1;

static LEVEL: AtomicUsize = AtomicUsize::new(LEVEL_INFO);

pub fn set_level(debug: bool) {
    LEVEL.store(
        if debug { LEVEL_DEBUG } else { LEVEL_INFO },
        Ordering::Relaxed,
    );
}

pub fn info(target: &str, args: std::fmt::Arguments<'_>) {
    eprintln!("I {target}: {args}");
}

pub fn warn(target: &str, args: std::fmt::Arguments<'_>) {
    eprintln!("W {target}: {args}");
}

pub fn error(target: &str, args: std::fmt::Arguments<'_>) {
    eprintln!("E {target}: {args}");
}

pub fn debug(target: &str, args: std::fmt::Arguments<'_>) {
    if LEVEL.load(Ordering::Relaxed) >= LEVEL_DEBUG {
        eprintln!("D {target}: {args}");
    }
}
