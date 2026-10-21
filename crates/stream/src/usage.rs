//! CPU and memory used by this app, shown next to the stream statistics.

use std::collections::HashSet;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Usage {
    /// Share of the whole machine (all cores), 0–100.
    pub cpu_percent: f32,
    /// Resident memory.
    pub memory_bytes: u64,
}

/// Samples this process plus its child processes (the web view runs in children on Windows
/// and Linux; on macOS WebKit's processes belong to launchd and are not counted).
pub struct UsageSampler {
    system: System,
    pid: Option<Pid>,
    cores: f32,
    /// This process and its descendants, found by the last full scan.
    ours: Vec<Pid>,
    /// Samples until the next full scan of the machine's processes (to find new children);
    /// in between only ours are refreshed.
    until_scan: u32,
}

/// Samples between two full scans (one every ~30 s at the usual 2 s pace).
const SCAN_EVERY: u32 = 15;

impl Default for UsageSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageSampler {
    pub fn new() -> Self {
        let mut sampler = Self {
            system: System::new(),
            pid: sysinfo::get_current_pid().ok(),
            cores: std::thread::available_parallelism().map_or(1, |n| n.get()) as f32,
            ours: Vec::new(),
            until_scan: 0,
        };
        // CPU usage is a difference between two refreshes: prime the first one.
        sampler.refresh();
        sampler
    }

    fn refresh(&mut self) {
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
        if self.until_scan == 0 || self.ours.is_empty() {
            self.until_scan = SCAN_EVERY;
            self.system
                .refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
            self.ours = self.descendants();
        } else {
            self.until_scan -= 1;
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&self.ours),
                true,
                kind,
            );
        }
    }

    /// This process and its descendants, in the processes last scanned.
    fn descendants(&self) -> Vec<Pid> {
        let Some(root) = self.pid else {
            return Vec::new();
        };
        let processes = self.system.processes();
        let mut ours = HashSet::from([root]);
        // Children may be listed before their parents: repeat until no new descendant.
        loop {
            let before = ours.len();
            for (pid, process) in processes {
                if process.parent().is_some_and(|p| ours.contains(&p)) {
                    ours.insert(*pid);
                }
            }
            if ours.len() == before {
                break;
            }
        }
        ours.into_iter().collect()
    }

    /// Usage since the previous call (call every second or two; blocking, a few ms).
    pub fn sample(&mut self) -> Option<Usage> {
        let root = self.pid?;
        self.refresh();
        let processes = self.system.processes();
        let (cpu, memory) = self
            .ours
            .iter()
            .filter_map(|pid| processes.get(pid))
            .fold((0.0, 0), |(cpu, mem), p| {
                (cpu + p.cpu_usage(), mem + p.memory())
            });
        processes.get(&root)?;
        Some(Usage {
            cpu_percent: (cpu / self.cores).clamp(0.0, 100.0),
            memory_bytes: memory,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burn(duration: std::time::Duration) {
        let start = std::time::Instant::now();
        let mut x = 0u64;
        while start.elapsed() < duration {
            x = x.wrapping_mul(31).wrapping_add(7);
        }
        std::hint::black_box(x);
    }

    #[test]
    fn measures_this_process() {
        // Linux: sysinfo skips a process whose CPU time was still zero at the first sample
        // (only a freshly started process like this test; the app has run for a while).
        burn(std::time::Duration::from_millis(50));
        let mut sampler = UsageSampler::new();
        // Burn some CPU so the second sample has something to show.
        burn(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL * 2);
        let usage = sampler.sample().expect("own process visible");
        assert!(usage.memory_bytes > 1_000_000, "{usage:?}");
        assert!(
            usage.cpu_percent > 0.0 && usage.cpu_percent <= 100.0,
            "{usage:?}"
        );
    }
}
