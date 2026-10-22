use std::sync::{Once, OnceLock};
use std::time::Instant;

/// Set first thing in `main`; `trace_first_frame` reports the time elapsed since then.
pub static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// `CRUSTDB_TRACE_STARTUP=1`: prints `first_frame_ms=<n>` to stderr once, on the first frame.
pub fn trace_first_frame() {
    static FIRST: Once = Once::new();
    FIRST.call_once(|| {
        if let (Some(start), Some(_)) = (
            PROCESS_START.get(),
            std::env::var_os("CRUSTDB_TRACE_STARTUP"),
        ) {
            eprintln!("first_frame_ms={}", start.elapsed().as_millis());
        }
    });
}

/// Kernel clock ticks per second for /proc stat fields (D31).
const USER_HZ: f64 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcStats {
    pub cpu_percent: f32,
    pub rss_bytes: u64,
}

/// utime + stime (fields 14 and 15), counted after the last `)` because `comm` may hold spaces.
pub fn parse_stat_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut f = rest.split_whitespace().skip(11);
    let utime: u64 = f.next()?.parse().ok()?;
    let stime: u64 = f.next()?.parse().ok()?;
    Some(utime + stime)
}

pub fn parse_vm_rss_kb(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub fn cpu_percent_from_two_samples(t0: u64, t1: u64, elapsed_secs: f64) -> f32 {
    if elapsed_secs <= 0.0 {
        return 0.0;
    }
    (t1.saturating_sub(t0) as f64 / USER_HZ / elapsed_secs * 100.0) as f32
}

#[derive(Default)]
pub struct ProcSampler {
    last: Option<(u64, Instant)>,
}

impl ProcSampler {
    pub fn sample(&mut self) -> Option<ProcStats> {
        let ticks = parse_stat_ticks(&std::fs::read_to_string("/proc/self/stat").ok()?)?;
        let rss_kb = parse_vm_rss_kb(&std::fs::read_to_string("/proc/self/status").ok()?)?;
        let now = Instant::now();
        // The first sample has no previous point: report 0.0 instead of None so the RSS still shows.
        let cpu_percent = match self.last.replace((ticks, now)) {
            Some((t0, at)) => cpu_percent_from_two_samples(t0, ticks, (now - at).as_secs_f64()),
            None => 0.0,
        };
        Some(ProcStats {
            cpu_percent,
            rss_bytes: rss_kb * 1024,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_with_spaces_in_comm() {
        let stat = "123 (my (proc)) S 1 123 123 0 -1 4194304 100 0 0 0 40 2 0 0 20 0 1 0 5 1000 50 18446744073709551615";
        assert_eq!(parse_stat_ticks(stat), Some(42));
        assert_eq!(parse_stat_ticks("garbage"), None);
    }

    #[test]
    fn parses_vm_rss() {
        let status = "Name:\tcrustdb\nVmPeak:\t  9000 kB\nVmRSS:\t   12345 kB\nThreads:\t4\n";
        assert_eq!(parse_vm_rss_kb(status), Some(12345));
        assert_eq!(parse_vm_rss_kb("Name:\tx\n"), None);
    }

    #[test]
    fn cpu_percent_from_two_samples() {
        // 100 ticks = 1 cpu-second over 2 s elapsed -> 50%.
        assert_eq!(super::cpu_percent_from_two_samples(100, 200, 2.0), 50.0);
        assert_eq!(super::cpu_percent_from_two_samples(0, 200, 1.0), 200.0);
        assert_eq!(super::cpu_percent_from_two_samples(5, 5, 1.0), 0.0);
        assert_eq!(super::cpu_percent_from_two_samples(0, 10, 0.0), 0.0);
    }

    #[test]
    fn first_sample_has_zero_cpu_and_real_rss() {
        let mut s = ProcSampler::default();
        let first = s.sample().unwrap();
        assert_eq!(first.cpu_percent, 0.0);
        assert!(first.rss_bytes > 0);
        assert!(s.sample().unwrap().cpu_percent >= 0.0);
    }
}
