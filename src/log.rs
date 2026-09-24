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
        eprintln!(
            "{}",
            format_line('I', std::time::SystemTime::now(), target, args)
        );
    }
}

pub fn warn(target: &str, args: std::fmt::Arguments<'_>) {
    eprintln!(
        "{}",
        format_line('W', std::time::SystemTime::now(), target, args)
    );
}

pub fn error(target: &str, args: std::fmt::Arguments<'_>) {
    eprintln!(
        "{}",
        format_line('E', std::time::SystemTime::now(), target, args)
    );
}

pub fn debug(target: &str, args: std::fmt::Arguments<'_>) {
    if enabled(LEVEL_DEBUG) {
        eprintln!(
            "{}",
            format_line('D', std::time::SystemTime::now(), target, args)
        );
    }
}

fn format_line(
    prefix: char,
    time: std::time::SystemTime,
    target: &str,
    message: std::fmt::Arguments<'_>,
) -> String {
    format!("{prefix} {} {target}: {message}", rfc3339_utc(time))
}

fn rfc3339_utc(time: std::time::SystemTime) -> String {
    let seconds = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month + 2) / 5 + 1;
    let month = month + if month < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
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

    #[test]
    fn timestamped_line_has_rfc3339_utc_layout() {
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_772_366_400);
        let line = super::format_line('I', time, "target", format_args!("msg"));
        assert_eq!(line, "I 2026-03-01T12:00:00Z target: msg");
    }
}
