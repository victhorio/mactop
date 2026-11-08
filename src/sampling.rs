use std::{
    io,
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use mactop::{
    app::{DEFAULT_INTERVAL, Update},
    metrics::{MetricsSource, MockSource},
};

pub enum Command {
    Scenario,
    Topology,
    Interval(Duration),
    Stop,
}

type Latest = Arc<Mutex<Option<io::Result<Update>>>>;

pub struct Sampler {
    latest: Latest,
    commands: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
}

struct Schedule {
    interval: Duration,
    next: Instant,
    gap: bool,
}

impl Schedule {
    fn new(now: Instant) -> Self {
        Self {
            interval: DEFAULT_INTERVAL,
            next: now + DEFAULT_INTERVAL,
            gap: false,
        }
    }

    fn reschedule(&mut self, interval: Duration, now: Instant) {
        self.gap |= now.saturating_duration_since(self.next) > self.interval;
        self.interval = interval;
        self.next = now + interval;
    }

    fn collect(
        &mut self,
        now: Instant,
        sample: impl FnOnce() -> io::Result<Update>,
    ) -> io::Result<Update> {
        let due = self.next;
        self.next = now + self.interval;
        let mut update = sample();
        if let Ok(update) = &mut update {
            // Rate commands can intentionally extend the time between samples.
            update.delayed = self.gap || update.at.saturating_duration_since(due) > self.interval;
        }
        self.gap = false;
        update
    }
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
                let mut schedule = Schedule::new(Instant::now());
                let mut sequence = 0;
                loop {
                    let reset = match receiver
                        .recv_timeout(schedule.next.saturating_duration_since(Instant::now()))
                    {
                        Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        Ok(Command::Interval(value)) => {
                            schedule.reschedule(value, Instant::now());
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
                    sequence += 1;
                    let mut update =
                        schedule.collect(Instant::now(), || source.sample(reset, sequence));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn update(at: Instant) -> io::Result<Update> {
        Ok(Update {
            snapshot: MockSource::default().sample()?,
            demo_label: Some("Everyday"),
            reset: false,
            sequence: 1,
            at,
            delayed: false,
        })
    }

    #[test]
    fn rate_changes_do_not_create_gaps_or_hide_existing_stalls() {
        let start = Instant::now();
        let at = |millis| start + Duration::from_millis(millis);
        let mut schedule = Schedule::new(start);
        schedule.reschedule(Duration::from_millis(250), at(0));
        schedule.reschedule(Duration::from_secs(5), at(10));
        schedule.reschedule(Duration::from_millis(250), at(4_000));
        let sample = schedule.collect(at(4_250), || update(at(4_260))).unwrap();
        assert!(!sample.delayed, "a timely slow→fast change stays connected");

        let mut schedule = Schedule::new(start);
        schedule.reschedule(Duration::from_millis(250), at(10_000));
        schedule.reschedule(Duration::from_millis(500), at(10_050));
        let sample = schedule.collect(at(10_550), || update(at(10_560))).unwrap();
        assert!(
            sample.delayed,
            "rescheduling must preserve an already overdue deadline"
        );
        let recovered = schedule.collect(at(11_050), || update(at(11_060))).unwrap();
        assert!(
            !recovered.delayed,
            "only the interrupted interval is disconnected"
        );
    }

    #[test]
    fn collection_lateness_uses_the_deadline_and_recovers_after_errors() {
        let start = Instant::now();
        for (millis, delayed) in [(2_000, false), (2_001, true)] {
            let mut schedule = Schedule::new(start);
            let sample = schedule
                .collect(start + Duration::from_secs(1), || {
                    update(start + Duration::from_millis(millis))
                })
                .unwrap();
            assert_eq!(sample.delayed, delayed, "lateness at {millis}ms");
        }

        let mut schedule = Schedule::new(start);
        let resumed = start + Duration::from_secs(10);
        schedule.reschedule(Duration::from_millis(250), resumed);
        let failed = schedule.collect(resumed, || Err(io::Error::other("collection failed")));
        assert_eq!(failed.err().unwrap().to_string(), "collection failed");
        let next = resumed + Duration::from_millis(250);
        assert!(!schedule.collect(next, || update(next)).unwrap().delayed);
    }

    #[test]
    fn latest_delivery_retains_an_overwritten_demo_reset_until_consumed() {
        let (sampler, initial) = Sampler::start(true).unwrap();
        assert_eq!(initial.sequence, 0);
        sampler
            .command(Command::Interval(Duration::from_millis(250)))
            .unwrap();
        sampler.command(Command::Topology).unwrap();
        let take_after = |sequence| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let ready = sampler
                    .latest
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|result| {
                        result
                            .as_ref()
                            .is_ok_and(|update| update.sequence >= sequence)
                    });
                if ready {
                    return sampler.take().unwrap().unwrap().unwrap();
                }
                assert!(
                    Instant::now() < deadline,
                    "demo worker did not deliver a sample"
                );
                thread::sleep(Duration::from_millis(5));
            }
        };
        let overwritten = take_after(3);
        assert!(
            overwritten.reset,
            "ordinary samples cannot erase an unconsumed reset"
        );
        assert_eq!(overwritten.snapshot.cpu.len(), 2);
        let next = take_after(overwritten.sequence + 1);
        assert!(!next.reset, "a consumed reset must not be replayed");
        assert_eq!(next.snapshot.cpu.len(), 2);
    }
}
