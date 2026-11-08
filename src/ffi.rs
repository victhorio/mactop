use std::{
    ffi::c_char,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    time::{Duration, Instant},
};

use crate::{
    app::Update,
    metrics::{Pressure, TierKind},
    sampling::{Command, Sampler},
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MactopString {
    pub data: *const u8,
    pub len: usize,
}

impl From<&str> for MactopString {
    fn from(value: &str) -> Self {
        Self {
            data: value.as_ptr(),
            len: value.len(),
        }
    }
}

#[repr(C)]
pub struct MactopCpuTier {
    pub kind: u32,
    pub cores: *const f64,
    pub core_count: usize,
    pub frequency_ghz: f64,
    pub max_frequency_ghz: f64,
}

#[repr(C)]
pub struct MactopProcess {
    pub pid: u32,
    pub name: MactopString,
    pub cpu_percent: f64,
    pub resident_bytes: u64,
}

#[repr(C)]
pub struct MactopMemory {
    pub app_bytes: u64,
    pub wired_bytes: u64,
    pub compressed_bytes: u64,
    pub cached_bytes: u64,
    pub free_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_allocated_bytes: u64,
    pub pressure: u32,
    pub pressure_percent: u32,
    pub swap_in_bytes_per_sec: f64,
    pub swap_out_bytes_per_sec: f64,
}

#[repr(C)]
pub struct MactopSnapshot {
    pub machine: MactopString,
    pub tiers: *const MactopCpuTier,
    pub tier_count: usize,
    pub memory: MactopMemory,
    pub processes: *const MactopProcess,
    pub process_count: usize,
    pub time_seconds: f64,
    pub sequence: u64,
    pub reset: u8,
    pub delayed: u8,
}

pub struct MactopHandle {
    sampler: Sampler,
    update: Update,
    initial: bool,
    origin: Instant,
    tiers: Vec<MactopCpuTier>,
    processes: Vec<MactopProcess>,
    last_error: Vec<u8>,
}

impl MactopHandle {
    fn poll(&mut self) -> io::Result<Option<MactopSnapshot>> {
        if self.initial {
            self.initial = false;
        } else {
            match self.sampler.take()? {
                Some(update) => self.update = update?,
                None => return Ok(None),
            }
        }

        // Build views only after the owned snapshot is in its stable handle storage:
        // CompactString may keep short machine/process names inline.
        let snapshot = &self.update.snapshot;
        self.tiers.clear();
        self.tiers
            .extend(snapshot.cpu.iter().map(|tier| MactopCpuTier {
                kind: match tier.kind {
                    TierKind::Efficiency => 0,
                    TierKind::Performance => 1,
                    TierKind::Super => 2,
                },
                cores: tier.cores.as_ptr(),
                core_count: tier.cores.len(),
                frequency_ghz: tier.frequency_ghz.unwrap_or(f64::NAN),
                max_frequency_ghz: tier.max_frequency_ghz.unwrap_or(f64::NAN),
            }));
        self.processes.clear();
        self.processes
            .extend(snapshot.processes.iter().map(|process| MactopProcess {
                pid: process.pid,
                name: process.name.as_str().into(),
                cpu_percent: process.cpu_percent,
                resident_bytes: process.resident_bytes,
            }));
        let memory = &snapshot.memory;
        Ok(Some(MactopSnapshot {
            machine: snapshot.machine.as_str().into(),
            tiers: self.tiers.as_ptr(),
            tier_count: self.tiers.len(),
            memory: MactopMemory {
                app_bytes: memory.app,
                wired_bytes: memory.wired,
                compressed_bytes: memory.compressed,
                cached_bytes: memory.cached,
                free_bytes: memory.free,
                swap_used_bytes: memory.swap_used,
                swap_allocated_bytes: memory.swap_allocated,
                pressure: match memory.pressure {
                    Pressure::Normal => 0,
                    Pressure::Warning => 1,
                    Pressure::Critical => 2,
                },
                pressure_percent: memory.pressure_percent,
                swap_in_bytes_per_sec: memory.swap_in_bytes_per_sec,
                swap_out_bytes_per_sec: memory.swap_out_bytes_per_sec,
            },
            processes: self.processes.as_ptr(),
            process_count: self.processes.len(),
            time_seconds: self.update.at.duration_since(self.origin).as_secs_f64(),
            sequence: self.update.sequence,
            reset: u8::from(self.update.reset),
            delayed: u8::from(self.update.delayed),
        }))
    }

    fn set_error(&mut self, message: &str) {
        self.last_error.clear();
        self.last_error.extend(
            message
                .bytes()
                .map(|byte| if byte == 0 { b'?' } else { byte }),
        );
        self.last_error.push(0);
    }
}

unsafe fn write_startup_error(error: *mut c_char, capacity: usize, message: &str) {
    if error.is_null() || capacity == 0 {
        return;
    }
    let mut len = message.len().min(capacity - 1);
    while !message.is_char_boundary(len) {
        len -= 1;
    }
    for (offset, byte) in message.bytes().take(len).enumerate() {
        unsafe {
            error
                .add(offset)
                .write(if byte == 0 { b'?' } else { byte } as c_char)
        };
    }
    unsafe { error.add(len).write(0) };
}

/// Starts the shared sampler without falling back to demo metrics.
///
/// # Safety
/// If nonnull, `error` must be writable for `error_capacity` bytes. The returned
/// handle must be used serially and destroyed exactly once with `mactop_destroy`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mactop_start(
    demo: u8,
    error: *mut c_char,
    error_capacity: usize,
) -> *mut MactopHandle {
    match catch_unwind(|| {
        unsafe { write_startup_error(error, error_capacity, "") };
        match Sampler::start(demo != 0) {
            Ok((sampler, initial)) => Box::into_raw(Box::new(MactopHandle {
                sampler,
                origin: initial.at,
                update: initial,
                initial: true,
                tiers: Vec::new(),
                processes: Vec::new(),
                last_error: vec![0],
            })),
            Err(reason) => {
                unsafe { write_startup_error(error, error_capacity, &reason.to_string()) };
                ptr::null_mut()
            }
        }
    }) {
        Ok(handle) => handle,
        Err(_) => {
            unsafe { write_startup_error(error, error_capacity, "Rust panic starting metrics") };
            ptr::null_mut()
        }
    }
}

/// Returns 1 for a reading, 0 if none is ready, or -1 on error. Only 1 writes `out`.
///
/// # Safety
/// A nonnull `handle` must be live and exclusively accessed during this call.
/// A nonnull `out` must be aligned and writable for one `MactopSnapshot`. All
/// returned pointers borrow the handle until its next poll or destruction.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mactop_poll(handle: *mut MactopHandle, out: *mut MactopSnapshot) -> i32 {
    let Some(handle) = (unsafe { handle.as_mut() }) else {
        return -1;
    };
    match catch_unwind(AssertUnwindSafe(|| {
        handle.set_error("");
        if out.is_null() {
            handle.set_error("snapshot output pointer is null");
            return -1;
        }
        match handle.poll() {
            Ok(Some(snapshot)) => {
                unsafe { out.write(snapshot) };
                1
            }
            Ok(None) => 0,
            Err(reason) => {
                handle.set_error(&reason.to_string());
                -1
            }
        }
    })) {
        Ok(status) => status,
        Err(_) => {
            handle.set_error("Rust panic polling metrics");
            -1
        }
    }
}

/// Changes the shared worker's sampling interval without restarting its counters.
/// Returns 0 on success or -1 on error; inspect `mactop_last_error` on failure.
///
/// # Safety
/// A nonnull `handle` must be live and exclusively accessed during this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mactop_set_interval(
    handle: *mut MactopHandle,
    interval_milliseconds: u32,
) -> i32 {
    let Some(handle) = (unsafe { handle.as_mut() }) else {
        return -1;
    };
    match catch_unwind(AssertUnwindSafe(|| {
        handle.set_error("");
        if !matches!(interval_milliseconds, 250 | 500 | 1000 | 2000 | 5000) {
            handle.set_error("unsupported sampling interval");
            return -1;
        }
        match handle
            .sampler
            .command(Command::Interval(Duration::from_millis(u64::from(
                interval_milliseconds,
            )))) {
            Ok(()) => 0,
            Err(reason) => {
                handle.set_error(&reason.to_string());
                -1
            }
        }
    })) {
        Ok(status) => status,
        Err(_) => {
            handle.set_error("Rust panic changing sampling interval");
            -1
        }
    }
}

/// Borrows NUL-terminated error text until the next poll, interval change, or destruction.
///
/// # Safety
/// A nonnull `handle` must be live and not accessed by another call concurrently.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mactop_last_error(handle: *const MactopHandle) -> *const c_char {
    match unsafe { handle.as_ref() } {
        Some(handle) => handle.last_error.as_ptr().cast(),
        None => c"invalid mactop handle".as_ptr(),
    }
}

/// Stops and joins the sampler. A null handle is a no-op.
///
/// # Safety
/// A nonnull `handle` must come from `mactop_start`, be exclusively accessed, and
/// never be used again after this call, including for a second destruction.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mactop_destroy(handle: *mut MactopHandle) {
    if !handle.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| unsafe { drop(Box::from_raw(handle)) }));
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::CStr, mem::MaybeUninit, slice};

    use crate::metrics::{MetricsSource, MockSource};

    use super::*;

    #[test]
    fn public_demo_abi_preserves_the_complete_sampler_snapshot() {
        let expected = MockSource::default().sample().unwrap();
        let mut error = [0 as c_char; 128];
        let handle = unsafe { mactop_start(1, error.as_mut_ptr(), error.len()) };
        assert!(!handle.is_null());
        assert_eq!(error[0], 0);
        for invalid in [0, 1, 249, 251, u32::MAX] {
            assert_eq!(unsafe { mactop_set_interval(handle, invalid) }, -1);
            assert_eq!(
                unsafe { CStr::from_ptr(mactop_last_error(handle)) }.to_bytes(),
                b"unsupported sampling interval"
            );
        }
        assert_eq!(unsafe { mactop_set_interval(handle, 250) }, 0);
        let mut out = MaybeUninit::uninit();
        assert_eq!(unsafe { mactop_poll(handle, out.as_mut_ptr()) }, 1);
        let actual = unsafe { out.assume_init() };
        assert_eq!(actual.time_seconds, 0.0);
        assert_eq!(actual.sequence, 0);
        assert_eq!(actual.reset, 1);
        assert_eq!(actual.delayed, 0);
        assert_eq!(
            unsafe { slice::from_raw_parts(actual.machine.data, actual.machine.len) },
            expected.machine.as_bytes()
        );
        let tiers = unsafe { slice::from_raw_parts(actual.tiers, actual.tier_count) };
        assert_eq!(tiers.len(), expected.cpu.len());
        for (index, (actual, expected)) in tiers.iter().zip(&expected.cpu).enumerate() {
            assert_eq!(actual.kind, index as u32);
            assert_eq!(
                unsafe { slice::from_raw_parts(actual.cores, actual.core_count) },
                expected.cores
            );
            assert_eq!(actual.frequency_ghz, expected.frequency_ghz.unwrap());
            assert_eq!(
                actual.max_frequency_ghz,
                expected.max_frequency_ghz.unwrap()
            );
        }
        let processes = unsafe { slice::from_raw_parts(actual.processes, actual.process_count) };
        assert_eq!(processes.len(), expected.processes.len());
        for (actual, expected) in processes.iter().zip(&expected.processes) {
            assert_eq!(actual.pid, expected.pid);
            assert_eq!(
                unsafe { slice::from_raw_parts(actual.name.data, actual.name.len) },
                expected.name.as_bytes()
            );
            assert_eq!(actual.cpu_percent, expected.cpu_percent);
            assert_eq!(actual.resident_bytes, expected.resident_bytes);
        }
        let memory = actual.memory;
        assert_eq!(memory.app_bytes, expected.memory.app);
        assert_eq!(memory.wired_bytes, expected.memory.wired);
        assert_eq!(memory.compressed_bytes, expected.memory.compressed);
        assert_eq!(memory.cached_bytes, expected.memory.cached);
        assert_eq!(memory.free_bytes, expected.memory.free);
        assert_eq!(memory.swap_used_bytes, expected.memory.swap_used);
        assert_eq!(memory.swap_allocated_bytes, expected.memory.swap_allocated);
        assert_eq!(memory.pressure, 0);
        assert_eq!(memory.pressure_percent, 20);
        assert_eq!(
            memory.swap_in_bytes_per_sec,
            expected.memory.swap_in_bytes_per_sec
        );
        assert_eq!(
            memory.swap_out_bytes_per_sec,
            expected.memory.swap_out_bytes_per_sec
        );
        assert!(unsafe { CStr::from_ptr(mactop_last_error(handle)) }.is_empty());
        unsafe { mactop_destroy(handle) };
    }
}
