//! Device-capacity probe: turn THIS machine's limits into fetch width.
//!
//! Mirrors the Python capacity component and proxalyze's 80%-of-parallelism
//! governor: available_parallelism honours cgroups/affinity; the soft fd
//! limit is RAISED toward the hard one (never lowered); free memory is
//! budgeted ~64 KB per concurrent fetch. An explicit --concurrency overrides.

use std::fs;

use rlimit::{Resource, getrlimit, setrlimit};

const FETCH_KB_PER_CONN: u64 = 64;
const FD_RESERVE: u64 = 512;
const MEM_RESERVE_KB: u64 = 256_000; // ~250 MB headroom for interpreter + bodies
const MAX_FD_RAISE: u64 = 65_536;

pub struct Capabilities {
    pub cpus: usize,
    pub fd_limit: u64,
    pub fetch_concurrency: usize,
}

fn mem_available_kb() -> u64 {
    if let Ok(text) = fs::read_to_string("/proc/meminfo") {
        let mut total_kb = None;
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(key), Some(val)) = (it.next(), it.next().and_then(|v| v.parse().ok())) else {
                continue;
            };
            match key {
                "MemAvailable:" => return val,
                "MemTotal:" => total_kb = Some(val),
                _ => {}
            }
        }
        if let Some(t) = total_kb {
            return t * 4 / 5;
        }
    }
    4_000_000 // conservative 4 GB assumption
}

/// Raises the process soft RLIMIT_NOFILE; returns the achieved value.
fn raise_fd_limit() -> u64 {
    let (soft, hard) = getrlimit(Resource::NOFILE).unwrap_or((1024, 1024));
    let want = soft.max(hard.min(MAX_FD_RAISE));
    if want > soft {
        // raising soft toward hard is permitted without privileges on Linux
        let _ = setrlimit(Resource::NOFILE, want.min(hard), hard);
    }
    getrlimit(Resource::NOFILE).map(|(s, _)| s).unwrap_or(soft)
}

fn size_fetch(cpus: usize, mem_kb: u64, fd_limit: u64) -> usize {
    let usable_mem_kb = (FD_RESERVE * 4).max(mem_kb.saturating_sub(MEM_RESERVE_KB));
    let memory_fetch = (usable_mem_kb / FETCH_KB_PER_CONN) as usize;
    let cpu_fetch = (cpus * 32).min(256).min(memory_fetch);
    let fd_fetch = fd_limit.saturating_sub(FD_RESERVE) as usize;

    // Keep useful forward progress even on severely constrained hosts.
    cpu_fetch.min(fd_fetch).max(16)
}

pub fn probe() -> Capabilities {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let fd_limit = raise_fd_limit();
    let mem_kb = mem_available_kb();
    let capabilities = Capabilities {
        cpus,
        fd_limit,
        fetch_concurrency: size_fetch(cpus, mem_kb, fd_limit),
    };

    crate::log::info(
        "capacity",
        format_args!(
            "cpus={} fd={} mem={} MB → fetch concurrency={}",
            capabilities.cpus,
            capabilities.fd_limit,
            mem_kb / 1024,
            capabilities.fetch_concurrency
        ),
    );
    capabilities
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fd_limit_below_reserve_uses_floor() {
        assert_eq!(size_fetch(8, 16_000_000, FD_RESERVE - 1), 16);
    }

    #[test]
    fn fd_limit_above_ceiling_preserves_cpu_limit() {
        assert_eq!(size_fetch(16, 16_000_000, 100_000), 256);
    }

    #[test]
    fn absurdly_low_memory_uses_conservative_reserve() {
        assert_eq!(size_fetch(8, 1, 1024), 32);
    }

    #[test]
    fn normal_machine_uses_existing_ceiling() {
        assert_eq!(size_fetch(8, 16_000_000, 1024), 256);
    }
}
