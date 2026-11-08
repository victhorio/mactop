use std::collections::VecDeque;

use crate::metrics::{Snapshot, TierKind};

pub const HISTORY_LEN: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Cpu,
    Memory,
}

pub struct History {
    pub cpu: Vec<(TierKind, VecDeque<f64>)>,
    pub memory_used: VecDeque<f64>,
    pub swap_in: VecDeque<f64>,
    pub swap_out: VecDeque<f64>,
}

pub struct App {
    pub snapshot: Snapshot,
    pub history: History,
    pub paused: bool,
    pub expanded: bool,
    pub help: bool,
    pub sort: Sort,
    pub process_offset: usize,
    pub demo_label: &'static str,
}

impl App {
    pub fn new(snapshot: Snapshot) -> Self {
        let mut app = Self {
            snapshot: snapshot.clone(),
            history: History {
                cpu: Vec::new(),
                memory_used: VecDeque::new(),
                swap_in: VecDeque::new(),
                swap_out: VecDeque::new(),
            },
            paused: false,
            expanded: false,
            help: false,
            sort: Sort::Cpu,
            process_offset: 0,
            demo_label: "Everyday",
        };
        app.ingest(snapshot);
        app
    }

    pub fn ingest(&mut self, snapshot: Snapshot) {
        let same_topology = self.history.cpu.len() == snapshot.cpu.len()
            && self
                .history
                .cpu
                .iter()
                .zip(&snapshot.cpu)
                .all(|((kind, _), tier)| *kind == tier.kind);
        if !same_topology {
            self.history.cpu = snapshot
                .cpu
                .iter()
                .map(|tier| (tier.kind, VecDeque::new()))
                .collect();
        }
        for ((_, history), tier) in self.history.cpu.iter_mut().zip(&snapshot.cpu) {
            push(history, tier.active_percent());
        }
        push(
            &mut self.history.memory_used,
            snapshot.memory.used_percent(),
        );
        push(
            &mut self.history.swap_in,
            snapshot.memory.swap_in_bytes_per_sec,
        );
        push(
            &mut self.history.swap_out,
            snapshot.memory.swap_out_bytes_per_sec,
        );
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

fn push<T>(history: &mut VecDeque<T>, value: T) {
    if history.len() == HISTORY_LEN {
        history.pop_front();
    }
    history.push_back(value);
}
