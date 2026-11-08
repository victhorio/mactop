use std::io;

use compact_str::CompactString;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacSource;

pub const GIB: u64 = 1024 * 1024 * 1024;
pub const MIB: u64 = 1024 * 1024;

/// A provider owns its sampling clock and any OS handles or previous counters.
/// Rates are deltas per second, not cumulative counters. CPU is active residency,
/// not frequency-scaled utilization. Process CPU uses 100% per logical core.
pub trait MetricsSource {
    fn sample(&mut self) -> io::Result<Snapshot>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TierKind {
    Efficiency,
    Performance,
    Super,
}

impl TierKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Efficiency => "E-CPU",
            Self::Performance => "P-CPU",
            Self::Super => "S-CPU",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Efficiency => "Efficiency",
            Self::Performance => "Performance",
            Self::Super => "Super",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CpuTier {
    pub kind: TierKind,
    pub cores: Vec<f64>,
    /// Residency-weighted clock while active; None if idle or unavailable.
    pub frequency_ghz: Option<f64>,
    pub max_frequency_ghz: Option<f64>,
}

impl CpuTier {
    pub fn active_percent(&self) -> f64 {
        self.cores.iter().sum::<f64>() / self.cores.len() as f64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressure {
    Normal,
    Warning,
    Critical,
}

impl Pressure {
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Warning => "Elevated",
            Self::Critical => "Critical",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Memory {
    pub app: u64,
    pub wired: u64,
    /// Physical bytes occupied by compressed contents, not their original size.
    pub compressed: u64,
    /// Reclaimable file cache, disjoint from the other RAM categories.
    pub cached: u64,
    pub free: u64,
    pub pressure: Pressure,
    pub swap_used: u64,
    pub swap_allocated: u64,
    pub swap_in_bytes_per_sec: f64,
    pub swap_out_bytes_per_sec: f64,
}

impl Memory {
    pub fn used(&self) -> u64 {
        self.app + self.wired + self.compressed
    }

    pub fn occupied(&self) -> u64 {
        self.used() + self.cached
    }

    pub fn available(&self) -> u64 {
        self.cached + self.free
    }

    pub fn total(&self) -> u64 {
        self.occupied() + self.free
    }

    pub fn used_percent(&self) -> f64 {
        self.used() as f64 / self.total() as f64 * 100.0
    }
}

#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    pub name: CompactString,
    pub cpu_percent: f64,
    pub resident_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub machine: CompactString,
    /// Only physically available tiers appear here. Never synthesize missing ones.
    pub cpu: Vec<CpuTier>,
    pub memory: Memory,
    pub processes: Vec<Process>,
}

impl Snapshot {
    pub fn core_count(&self) -> usize {
        self.cpu.iter().map(|tier| tier.cores.len()).sum()
    }

    pub fn cpu_percent(&self) -> f64 {
        self.cpu.iter().flat_map(|tier| &tier.cores).sum::<f64>() / self.core_count() as f64
    }
}

/// Deterministic fixtures exercise the real provider contract, without OS access.
#[derive(Default)]
pub struct MockSource {
    tick: u64,
    scenario: usize,
    topology: usize,
}

impl MockSource {
    pub fn label(&self) -> &'static str {
        ["Everyday", "CPU load", "Memory pressure"][self.scenario]
    }

    pub fn cycle_scenario(&mut self) {
        self.scenario = (self.scenario + 1) % 3;
        self.tick = 0;
    }

    pub fn cycle_topology(&mut self) {
        self.topology = (self.topology + 1) % 3;
        self.tick = 0;
    }

    fn wave(&self, phase: f64) -> f64 {
        let t = self.tick as f64;
        (t * 0.19 + phase).sin() * 0.65 + (t * 0.53 + phase * 1.7).cos() * 0.35
    }
}

impl MetricsSource for MockSource {
    fn sample(&mut self) -> io::Result<Snapshot> {
        let topology: &[(TierKind, usize)] = match self.topology {
            0 => &[
                (TierKind::Efficiency, 4),
                (TierKind::Performance, 6),
                (TierKind::Super, 6),
            ],
            1 => &[(TierKind::Efficiency, 4), (TierKind::Performance, 8)],
            _ => &[(TierKind::Performance, 10), (TierKind::Super, 6)],
        };
        let cpu = topology
            .iter()
            .map(|&(kind, count)| {
                let (base, max) = match kind {
                    TierKind::Efficiency => (28.0, 2.8),
                    TierKind::Performance => (47.0, 4.1),
                    TierKind::Super => (72.0, 4.6),
                };
                let base = if self.scenario == 1 { 88.0 } else { base };
                let cores = (0..count)
                    .map(|i| {
                        (base
                            + self.wave(i as f64 * 0.73 + max) * 17.0
                            + (i as f64 - count as f64 / 2.0) * 2.0)
                            .clamp(0.0, 100.0)
                    })
                    .collect();
                CpuTier {
                    kind,
                    cores,
                    frequency_ghz: Some(max * (0.67 + self.wave(max) * 0.12)),
                    max_frequency_ghz: Some(max),
                }
            })
            .collect();

        let pressure = self.scenario == 2;
        let gib = |value: f64| (value * GIB as f64) as u64;
        let app = gib(if pressure { 18.4 } else { 14.5 } + self.wave(0.0) * 0.35);
        let wired = gib(3.8);
        let compressed = gib(if pressure { 6.2 } else { 2.7 });
        let cached = gib(if pressure { 2.3 } else { 7.2 });
        let memory = Memory {
            app,
            wired,
            compressed,
            cached,
            free: 32 * GIB - app - wired - compressed - cached,
            pressure: if pressure {
                if self.tick % 60 < 16 {
                    Pressure::Critical
                } else {
                    Pressure::Warning
                }
            } else {
                Pressure::Normal
            },
            swap_used: gib(if pressure { 6.4 } else { 1.4 }),
            swap_allocated: if pressure { 8 * GIB } else { 3 * GIB },
            swap_in_bytes_per_sec: if pressure {
                (1.8 + self.wave(2.0)) * MIB as f64
            } else {
                0.0
            },
            swap_out_bytes_per_sec: if pressure {
                (8.2 + self.wave(3.0) * 4.0) * MIB as f64
            } else {
                0.0
            },
        };
        let processes = [
            (8241, "OrbStack", 324.5, 6.8),
            (402, "WindowServer", 24.2, 0.72),
            (19384, "cargo", 182.4, 1.2),
            (628, "Google Chrome", 16.8, 3.4),
            (9012, "rust-analyzer", 48.6, 1.8),
            (741, "Ghostty", 4.2, 0.24),
            (375, "kernel_task", 8.7, 0.38),
            (1854, "Code Helper", 12.4, 1.1),
            (912, "Finder", 0.8, 0.12),
            (318, "coreaudiod", 1.2, 0.04),
            (842, "Slack", 2.1, 0.62),
            (597, "mds_stores", 3.8, 0.18),
            (1205, "Safari", 1.4, 0.88),
            (292, "launchd", 0.1, 0.02),
            (116, "logd", 0.7, 0.05),
            (206, "opendirectoryd", 0.3, 0.04),
            (344, "launchservicesd", 0.4, 0.03),
            (388, "notifyd", 0.1, 0.01),
            (514, "distnoted", 0.2, 0.02),
            (537, "cfprefsd", 0.2, 0.03),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (pid, name, cpu, ram))| Process {
            pid,
            name: name.into(),
            cpu_percent: (cpu
                * (1.0 + self.wave(i as f64) * 0.13)
                * if self.scenario == 1 { 1.35 } else { 1.0 })
            .min(topology.iter().map(|(_, n)| *n).sum::<usize>() as f64 * 100.0),
            resident_bytes: gib(ram * if pressure && i == 0 { 1.5 } else { 1.0 }),
        })
        .collect();

        self.tick += 1;
        Ok(Snapshot {
            machine: "Apple Silicon".into(),
            cpu,
            memory,
            processes,
        })
    }
}
