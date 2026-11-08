#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::{
    hint::black_box,
    mem::MaybeUninit,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use mactop::metrics::{MacSource, MetricsSource};

struct Load {
    running: Arc<AtomicBool>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl Drop for Load {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            thread.join().unwrap();
        }
    }
}

fn consumed_cpu_time() -> Duration {
    let mut usage = MaybeUninit::<libc::rusage>::uninit();
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0,
        "getrusage(RUSAGE_SELF): {}",
        std::io::Error::last_os_error()
    );
    let usage = unsafe { usage.assume_init() };
    Duration::from_secs((usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as u64)
        + Duration::from_micros((usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as u64)
}

#[test]
fn native_snapshot_tracks_physical_ram_and_multicore_process_cpu() {
    let mut source = MacSource::new().unwrap();
    thread::sleep(Duration::from_millis(20));
    let before_started = Instant::now();
    let before_cpu_start = consumed_cpu_time();
    let before = source.sample().unwrap();
    let before_cpu_end = consumed_cpu_time();
    let before_finished = Instant::now();
    let physical = Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .unwrap();
    assert!(physical.status.success());
    let physical: u64 = String::from_utf8(physical.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(before.memory.total(), physical);
    assert_eq!(before.memory.used() + before.memory.available(), physical);
    assert_eq!(before.memory.swap_in_bytes_per_sec, 0.0);
    assert_eq!(before.memory.swap_out_bytes_per_sec, 0.0);
    assert!(before.cpu.iter().all(|tier| !tier.cores.is_empty()));
    let current_pid = std::process::id();
    let initial = before
        .processes
        .iter()
        .find(|p| p.pid == current_pid)
        .unwrap();
    assert_eq!(
        initial.cpu_percent, 0.0,
        "no lifetime CPU average on first sighting"
    );

    let running = Arc::new(AtomicBool::new(true));
    let load = Load {
        running: Arc::clone(&running),
        threads: (0..2)
            .map(|_| {
                let running = Arc::clone(&running);
                thread::spawn(move || {
                    while running.load(Ordering::Relaxed) {
                        black_box(1u64.wrapping_add(black_box(1)));
                    }
                })
            })
            .collect(),
    };
    let memory = vec![0xABu8; 32 * 1024 * 1024];
    thread::sleep(Duration::from_millis(600));
    let busy_started = Instant::now();
    let busy_cpu_start = consumed_cpu_time();
    let busy = source.sample().unwrap();
    let busy_cpu_end = consumed_cpu_time();
    let busy_finished = Instant::now();
    black_box(&memory);
    drop(load);
    let process = busy
        .processes
        .iter()
        .find(|p| p.pid == current_pid)
        .unwrap();

    // Each internal process-counter read and timestamp lies within its sample's
    // brackets. Bound both deltas instead of assuming collection is instantaneous,
    // or that two runnable threads receive two cores' worth of CPU time.
    // getrusage reports user and system counters at microsecond precision.
    let rounding = Duration::from_micros(4);
    let min_cpu = busy_cpu_start.saturating_sub(before_cpu_end + rounding);
    let max_cpu = busy_cpu_end - before_cpu_start + rounding;
    let min_elapsed = busy_started.duration_since(before_finished).as_secs_f64();
    let max_elapsed = busy_finished.duration_since(before_started).as_secs_f64();
    let min_percent = 100.0 * min_cpu.as_secs_f64() / max_elapsed;
    let max_percent = 100.0 * max_cpu.as_secs_f64() / min_elapsed;
    assert!(
        (min_percent..=max_percent).contains(&process.cpu_percent),
        "getrusage CPU envelope {min_percent}..={max_percent}%, observed {}%",
        process.cpu_percent
    );
    assert!(process.resident_bytes > initial.resident_bytes + 16 * 1024 * 1024);
    assert_eq!(busy.memory.total(), physical);
    assert_eq!(busy.core_count(), before.core_count());
    assert!(
        busy.cpu
            .iter()
            .flat_map(|tier| &tier.cores)
            .all(|p| (0.0..=100.0).contains(p))
    );
    for tier in &busy.cpu {
        if let (Some(current), Some(max)) = (tier.frequency_ghz, tier.max_frequency_ghz) {
            assert!(current > 0.0 && current <= max);
        }
    }
}
