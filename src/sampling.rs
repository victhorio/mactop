use std::{
    io,
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use mactop::{
    app::DEFAULT_INTERVAL,
    metrics::{MetricsSource, MockSource, Snapshot},
};

pub enum Command {
    Scenario,
    Topology,
    Interval(Duration),
    Stop,
}

pub struct Update {
    pub snapshot: Snapshot,
    pub demo_label: Option<&'static str>,
    pub reset: bool,
    pub sequence: u64,
    pub at: Instant,
    pub delayed: bool,
}

type Latest = Arc<Mutex<Option<io::Result<Update>>>>;

pub struct Sampler {
    latest: Latest,
    commands: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

enum Source {
    Demo(MockSource),
    #[cfg(target_os = "macos")]
    Live(Box<mactop::metrics::MacSource>),
}

impl Source {
    fn new(demo: bool) -> io::Result<Self> {
        if demo {
            return Ok(Self::Demo(MockSource::default()));
        }
        #[cfg(target_os = "macos")]
        {
            Ok(Self::Live(Box::new(mactop::metrics::MacSource::new()?)))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(io::Error::other(
                "live metrics require macOS on Apple Silicon; use --demo for fixtures",
            ))
        }
    }

    fn sample(&mut self, reset: bool, sequence: u64) -> io::Result<Update> {
        let (snapshot, demo_label) = match self {
            Self::Demo(source) => (source.sample()?, Some(source.label())),
            #[cfg(target_os = "macos")]
            Self::Live(source) => (source.sample()?, None),
        };
        Ok(Update {
            snapshot,
            demo_label,
            reset,
            sequence,
            at: Instant::now(),
            delayed: false,
        })
    }
}

impl Sampler {
    pub fn start(demo: bool) -> io::Result<(Self, Update)> {
        let latest: Latest = Arc::default();
        let pending = Arc::clone(&latest);
        let (commands, receiver) = mpsc::channel();
        let (ready, first) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("metrics".into())
            .spawn(move || {
                let source = Source::new(demo).and_then(|mut source| {
                    if !demo {
                        // Prime cumulative counters; do not display lifetime averages.
                        source.sample(false, 0)?;
                        thread::sleep(DEFAULT_INTERVAL);
                    }
                    let update = source.sample(true, 0)?;
                    Ok((source, update))
                });
                let (mut source, update) = match source {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                if ready.send(Ok(update)).is_err() {
                    return;
                }
                let mut interval = DEFAULT_INTERVAL;
                let mut next = Instant::now() + interval;
                let mut sequence = 0;
                let mut schedule_gap = false;
                loop {
                    let reset = match receiver
                        .recv_timeout(next.saturating_duration_since(Instant::now()))
                    {
                        Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        Ok(Command::Interval(value)) => {
                            let now = Instant::now();
                            schedule_gap |= now.saturating_duration_since(next) > interval;
                            interval = value;
                            next = now + interval;
                            continue;
                        }
                        Ok(command) => {
                            let Source::Demo(mock) = &mut source else {
                                continue;
                            };
                            match command {
                                Command::Scenario => mock.cycle_scenario(),
                                Command::Topology => mock.cycle_topology(),
                                Command::Interval(_) | Command::Stop => unreachable!(),
                            }
                            true
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => false,
                    };
                    let due = next;
                    next = Instant::now() + interval;
                    sequence += 1;
                    let mut update = source.sample(reset, sequence);
                    if let Ok(update) = &mut update {
                        // Rate commands can intentionally extend the time between samples.
                        update.delayed =
                            schedule_gap || update.at.saturating_duration_since(due) > interval;
                    }
                    schedule_gap = false;
                    // Overwrite an unconsumed result rather than queueing old frames.
                    let mut pending = pending.lock().unwrap();
                    if let (Ok(update), Some(Ok(previous))) = (&mut update, pending.as_ref()) {
                        update.reset |= previous.reset;
                    }
                    *pending = Some(update);
                }
            })?;
        let initial = first
            .recv()
            .map_err(|_| io::Error::other("metrics worker stopped during startup"))?;
        match initial {
            Ok(update) => Ok((
                Self {
                    latest,
                    commands,
                    worker: Some(worker),
                },
                update,
            )),
            Err(error) => {
                let _ = worker.join();
                Err(error)
            }
        }
    }

    pub fn take(&self) -> io::Result<Option<io::Result<Update>>> {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Err(io::Error::other("metrics worker stopped unexpectedly"));
        }
        Ok(self
            .latest
            .lock()
            .map_err(|_| io::Error::other("metrics worker failed"))?
            .take())
    }

    pub fn command(&self, command: Command) -> io::Result<()> {
        self.commands
            .send(command)
            .map_err(|_| io::Error::other("metrics worker stopped"))
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
