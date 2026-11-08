#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::{
    hint::black_box,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
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

#[test]
fn native_snapshot_tracks_physical_ram_and_multicore_process_cpu() {
    let mut source = MacSource::new().unwrap();
    thread::sleep(Duration::from_millis(20));
    let before = source.sample().unwrap();
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
    let busy = source.sample().unwrap();
    black_box(&memory);
    drop(load);
    let process = busy
        .processes
        .iter()
        .find(|p| p.pid == current_pid)
        .unwrap();
    assert!(
        (130.0..300.0).contains(&process.cpu_percent),
        "two busy threads, 100% per core: {}",
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
