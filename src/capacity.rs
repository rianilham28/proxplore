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

#[allow(dead_code)] // cpus/fd_limit are diagnostic context the probe logs
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

pub fn probe() -> Capabilities {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let fd_limit = raise_fd_limit();
    let mem_kb = mem_available_kb();
    let usable_mem_kb = (FD_RESERVE * 4).max(mem_kb.saturating_sub(MEM_RESERVE_KB));

    let mut fetch = (16usize).max(
        (cpus * 32)
            .min(256)
            .min((usable_mem_kb / FETCH_KB_PER_CONN) as usize),
    );
    fetch = fetch
        .max(16)
        .min(fetch.min((fd_limit - FD_RESERVE).max(16) as usize));

    crate::log::info(
        "capacity",
        format_args!(
            "cpus={cpus} fd={fd_limit} mem={} MB → fetch concurrency={fetch}",
            mem_kb / 1024
        ),
    );
    Capabilities {
        cpus,
        fd_limit,
        fetch_concurrency: fetch,
    }
}
