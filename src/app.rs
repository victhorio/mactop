use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use crate::{
    metrics::{Snapshot, TierKind},
    sampling::{DEFAULT_INTERVAL, Update},
};

pub const HISTORY_WINDOW: Duration = Duration::from_secs(60);

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
    fn push(&mut self, snapshot: &Snapshot, at: Instant, continuous: bool) {
        self.samples.push_back(HistorySample {
            at,
            continuous,
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
        while self.samples.len() > 1 && at.duration_since(self.samples[1].at) >= HISTORY_WINDOW {
            self.samples.pop_front();
        }
    }

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
    last_sequence: u64,
    disconnected: bool,
}

impl App {
    pub fn new(snapshot: Snapshot, at: Instant) -> Self {
        let mut app = Self {
            snapshot,
            history: History::default(),
            paused: false,
            expanded: false,
            modal: None,
            update_interval: DEFAULT_INTERVAL,
            sort: Sort::Cpu,
            process_offset: 0,
            demo_label: None,
            sampling_error: None,
            last_sequence: 0,
            disconnected: false,
        };
        app.history.push(&app.snapshot, at, false);
        app
    }

    pub fn accept(&mut self, result: io::Result<Update>) -> bool {
        match result {
            Ok(update) if !self.paused || update.reset => {
                let gap = update.sequence != self.last_sequence + 1 || update.delayed;
                self.last_sequence = update.sequence;
                if update.reset {
                    self.history = History::default();
                    self.process_offset = 0;
                }
                self.ingest(
                    update.snapshot,
                    update.at,
                    !self.disconnected && !gap && !update.reset,
                );
                self.demo_label = update.demo_label;
                self.sampling_error = None;
                self.disconnected = false;
                true
            }
            Err(error) => {
                self.sampling_error = Some(error.to_string());
                self.disconnected = true;
                true
            }
            _ => false,
        }
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            self.disconnected = true;
        }
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
        self.history
            .push(&snapshot, at, continuous && same_topology);
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
