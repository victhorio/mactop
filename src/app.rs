use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use crate::metrics::{Snapshot, TierKind};

pub const HISTORY_WINDOW: Duration = Duration::from_secs(60);
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Cpu,
    Memory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modal {
    Help,
    Frequency,
}

#[derive(Default)]
pub struct History {
    pub samples: VecDeque<HistorySample>,
}

pub struct HistorySample {
    pub at: Instant,
    pub continuous: bool,
    pub cpu: Vec<(TierKind, f64)>,
    pub memory_used: f64,
    pub swap_in: f64,
    pub swap_out: f64,
}

impl History {
    pub fn points(&self) -> impl Iterator<Item = (f64, &HistorySample)> {
        let latest = self.samples.back().map(|sample| sample.at);
        self.samples.iter().map(move |sample| {
            let x = HISTORY_WINDOW.as_secs_f64()
                - latest.unwrap().duration_since(sample.at).as_secs_f64();
            (x, sample)
        })
    }
}

pub struct App {
    pub snapshot: Snapshot,
    pub history: History,
    pub paused: bool,
    pub expanded: bool,
    pub modal: Option<Modal>,
    pub update_interval: Duration,
    pub sort: Sort,
    pub process_offset: usize,
    pub demo_label: Option<&'static str>,
    pub sampling_error: Option<String>,
}

impl App {
    pub fn new(snapshot: Snapshot, at: Instant) -> Self {
        let mut app = Self {
            snapshot: snapshot.clone(),
            history: History::default(),
            paused: false,
            expanded: false,
            modal: None,
            update_interval: DEFAULT_INTERVAL,
            sort: Sort::Cpu,
            process_offset: 0,
            demo_label: None,
            sampling_error: None,
        };
        app.ingest(snapshot, at, false);
        app
    }

    pub fn change_frequency(&mut self, faster: bool) -> Option<Duration> {
        let intervals = [250, 500, 1000, 2000, 5000].map(Duration::from_millis);
        let next = if faster {
            intervals
                .into_iter()
                .rev()
                .find(|v| *v < self.update_interval)
        } else {
            intervals.into_iter().find(|v| *v > self.update_interval)
        }?;
        self.update_interval = next;
        Some(next)
    }

    pub fn ingest(&mut self, snapshot: Snapshot, at: Instant, continuous: bool) {
        let same_topology = self.snapshot.cpu.len() == snapshot.cpu.len()
            && self
                .snapshot
                .cpu
                .iter()
                .zip(&snapshot.cpu)
                .all(|(a, b)| a.kind == b.kind && a.cores.len() == b.cores.len());
        if !same_topology {
            self.history = History::default();
        }
        self.history.samples.push_back(HistorySample {
            at,
            continuous: continuous && same_topology,
            cpu: snapshot
                .cpu
                .iter()
                .map(|tier| (tier.kind, tier.active_percent()))
                .collect(),
            memory_used: snapshot.memory.used_percent(),
            swap_in: snapshot.memory.swap_in_bytes_per_sec,
            swap_out: snapshot.memory.swap_out_bytes_per_sec,
        });
        // Keep one predecessor so intervals crossing the left edge can be clipped.
        while self.history.samples.len() > 1
            && at.duration_since(self.history.samples[1].at) >= HISTORY_WINDOW
        {
            self.history.samples.pop_front();
        }
        self.snapshot = snapshot;
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.sort = sort;
        self.process_offset = 0;
    }

    pub fn scroll(&mut self, down: bool, visible_rows: usize) {
        let last = self.snapshot.processes.len().saturating_sub(visible_rows);
        let offset = self.process_offset.min(last);
        self.process_offset = if down {
            (offset + 1).min(last)
        } else {
            offset.saturating_sub(1)
        };
    }
}
