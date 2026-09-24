//! Minimal leveled logger (proxalyze style: no framework, stderr only).
//! Level is a process-wide AtomicUsize set once from --quiet/--verbose.

use std::sync::atomic::{AtomicUsize, Ordering};

pub const LEVEL_QUIET: usize = 0;
pub const LEVEL_INFO: usize = 1;
pub const LEVEL_DEBUG: usize = 2;

static LEVEL: AtomicUsize = AtomicUsize::new(LEVEL_INFO);

pub fn set_level(quiet: bool, verbose: bool) {
    let level = match (quiet, verbose) {
        (true, false) => LEVEL_QUIET,
        (_, true) => LEVEL_DEBUG,
        (false, false) => LEVEL_INFO,
    };
    LEVEL.store(level, Ordering::Relaxed);
}

fn enabled(level: usize) -> bool {
    LEVEL.load(Ordering::Relaxed) >= level
}

pub fn info(target: &str, args: std::fmt::Arguments<'_>) {
    if enabled(LEVEL_INFO) {
        eprintln!("I {target}: {args}");
    }
}

pub fn warn(target: &str, args: std::fmt::Arguments<'_>) {
    eprintln!("W {target}: {args}");
}

pub fn error(target: &str, args: std::fmt::Arguments<'_>) {
    eprintln!("E {target}: {args}");
}

pub fn debug(target: &str, args: std::fmt::Arguments<'_>) {
    if enabled(LEVEL_DEBUG) {
        eprintln!("D {target}: {args}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use super::{LEVEL, LEVEL_DEBUG, LEVEL_INFO, LEVEL_QUIET, Ordering, enabled, set_level};

    static LEVEL_TEST_LOCK: Mutex<()> = Mutex::new(());

    struct LevelGuard {
        _lock: MutexGuard<'static, ()>,
        previous: usize,
    }

    impl LevelGuard {
        fn acquire() -> Self {
            let lock = LEVEL_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let previous = LEVEL.load(Ordering::Relaxed);
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for LevelGuard {
        fn drop(&mut self) {
            LEVEL.store(self.previous, Ordering::Relaxed);
        }
    }

    #[test]
    fn levels_gate_info_and_debug_output() {
        let _guard = LevelGuard::acquire();

        set_level(true, false);
        assert!(enabled(LEVEL_QUIET));
        assert!(!enabled(LEVEL_INFO));
        assert!(!enabled(LEVEL_DEBUG));

        set_level(false, false);
        assert!(enabled(LEVEL_INFO));
        assert!(!enabled(LEVEL_DEBUG));

        set_level(false, true);
        assert!(enabled(LEVEL_INFO));
        assert!(enabled(LEVEL_DEBUG));
    }

    #[test]
    fn verbose_takes_precedence_over_quiet() {
        let _guard = LevelGuard::acquire();

        set_level(true, true);

        assert!(enabled(LEVEL_INFO));
        assert!(enabled(LEVEL_DEBUG));
    }
}
