use std::{
    collections::HashMap,
    ffi::CStr,
    io,
    mem::{MaybeUninit, size_of},
    time::Instant,
};

use super::{Memory, MetricsSource, Pressure, Process, Snapshot};
use compact_str::CompactString;
use mach2::{
    mach_init::mach_host_self, mach_port::mach_port_deallocate, mach_time::mach_timebase_info,
    traps::mach_task_self,
};

mod cpu;

pub struct MacSource {
    cpu: cpu::Cpu,
    host: libc::mach_port_t,
    machine: CompactString,
    total_memory: u64,
    page_size: u64,
    timebase_ns: f64,
    previous_swap: Option<(Instant, u64, u64)>,
    pids: Vec<i32>,
    previous_processes: HashMap<u32, ProcessCounter>,
}

struct ProcessCounter {
    started: (u64, u64),
    cpu_ticks: u64,
    at: Instant,
}

impl MacSource {
    pub fn new() -> io::Result<Self> {
        let cpu = cpu::Cpu::new()?;
        let machine = sysctl_string(c"machdep.cpu.brand_string")?.into();
        let total_memory = sysctl::<u64>(c"hw.memsize")?;
        // sysconf reports the VM page size, not an assumed 4 KiB page.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size <= 0 {
            return Err(io::Error::last_os_error());
        }
        let mut timebase = mach_timebase_info { numer: 0, denom: 0 };
        if unsafe { mach_timebase_info(&mut timebase) } != 0 || timebase.denom == 0 {
            return Err(io::Error::other("cannot read the Mach CPU timebase"));
        }
        let process_count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if process_count <= 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            cpu,
            host: unsafe { mach_host_self() },
            machine,
            total_memory,
            page_size: page_size as u64,
            timebase_ns: timebase.numer as f64 / timebase.denom as f64,
            previous_swap: None,
            pids: vec![0; process_count as usize + 64],
            previous_processes: HashMap::new(),
        })
    }

    fn memory(&mut self) -> io::Result<Memory> {
        let mut stats = MaybeUninit::<libc::vm_statistics64>::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let result = unsafe {
            libc::host_statistics64(
                self.host,
                libc::HOST_VM_INFO64,
                stats.as_mut_ptr().cast(),
                &mut count,
            )
        };
        if result != 0 {
            return Err(io::Error::other(format!("host_statistics64: {result}")));
        }
        // The revision-1 fields through internal_page_count are required.
        let required =
            std::mem::offset_of!(libc::vm_statistics64, internal_page_count) + size_of::<u32>();
        if (count as usize) * size_of::<i32>() < required {
            return Err(io::Error::other("macOS returned incomplete VM statistics"));
        }
        let stats = unsafe { stats.assume_init() };
        let at = Instant::now();
        let swap = sysctl::<libc::xsw_usage>(c"vm.swapusage")?;
        let pressure = match sysctl::<i32>(c"kern.memorystatus_vm_pressure_level")? {
            1 => Pressure::Normal,
            2 => Pressure::Warning,
            4 => Pressure::Critical,
            level => {
                return Err(io::Error::other(format!(
                    "unknown memory pressure level {level}"
                )));
            }
        };
        let pressure_percent = 100_u32
            .checked_sub(sysctl::<u32>(c"kern.memorystatus_level")?)
            .ok_or_else(|| io::Error::other("invalid available-memory percentage"))?;
        let (swap_in, swap_out) = self.previous_swap.map_or((0.0, 0.0), |(then, ins, outs)| {
            let seconds = at.duration_since(then).as_secs_f64();
            (
                rate(stats.swapins, ins, seconds) * self.page_size as f64,
                rate(stats.swapouts, outs, seconds) * self.page_size as f64,
            )
        });
        self.previous_swap = Some((at, stats.swapins, stats.swapouts));

        let bytes = |pages: u32| u64::from(pages) * self.page_size;
        let app = bytes(
            stats
                .internal_page_count
                .saturating_sub(stats.purgeable_count),
        );
        let compressed = bytes(stats.compressor_page_count);
        let cached = bytes(stats.external_page_count) + bytes(stats.purgeable_count);
        // File-backed pages include speculative pages, as does free_count.
        let free = bytes(stats.free_count.saturating_sub(stats.speculative_count));
        let reclaimable_and_app = app + compressed + cached + free;
        if reclaimable_and_app > self.total_memory {
            return Err(io::Error::other("inconsistent physical memory counters"));
        }
        // Include reserved/unmanaged RAM in non-reclaimable memory, not "free".
        // This is wired pages plus the installed RAM outside the VM page queues.
        let wired = self.total_memory - reclaimable_and_app;
        Ok(Memory {
            app,
            wired,
            compressed,
            cached,
            free,
            pressure,
            pressure_percent,
            swap_used: swap.xsu_used,
            swap_allocated: swap.xsu_total,
            swap_in_bytes_per_sec: swap_in,
            swap_out_bytes_per_sec: swap_out,
        })
    }

    fn processes(&mut self) -> io::Result<Vec<Process>> {
        let count = loop {
            let count = unsafe {
                libc::proc_listallpids(
                    self.pids.as_mut_ptr().cast(),
                    size_of_val(self.pids.as_slice()) as i32,
                )
            };
            if count <= 0 {
                return Err(io::Error::last_os_error());
            }
            if (count as usize) < self.pids.len() {
                break count as usize;
            }
            self.pids.resize(self.pids.len() * 2, 0);
        };
        let observed_since = Instant::now();
        let mut processes = Vec::with_capacity(self.previous_processes.len());
        for pid in self.pids[..count].iter().copied().filter(|pid| *pid > 0) {
            let mut info = MaybeUninit::<libc::proc_taskallinfo>::zeroed();
            let size = size_of::<libc::proc_taskallinfo>() as i32;
            if unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDTASKALLINFO,
                    0,
                    info.as_mut_ptr().cast(),
                    size,
                )
            } != size
            {
                // Processes can exit or deny inspection between enumeration and read.
                continue;
            }
            let info = unsafe { info.assume_init() };
            let started = (info.pbsd.pbi_start_tvsec, info.pbsd.pbi_start_tvusec);
            let cpu_ticks = info
                .ptinfo
                .pti_total_user
                .saturating_add(info.ptinfo.pti_total_system);
            let at = Instant::now();
            let cpu_percent = self
                .previous_processes
                .insert(
                    pid as u32,
                    ProcessCounter {
                        started,
                        cpu_ticks,
                        at,
                    },
                )
                .filter(|old| old.started == started)
                .map_or(0.0, |old| {
                    rate(
                        cpu_ticks,
                        old.cpu_ticks,
                        at.duration_since(old.at).as_secs_f64(),
                    ) * self.timebase_ns
                        / 1e7
                });
            let name = if info.pbsd.pbi_name[0] == 0 {
                &info.pbsd.pbi_comm[..]
            } else {
                &info.pbsd.pbi_name[..]
            };
            let name =
                unsafe { std::slice::from_raw_parts(name.as_ptr().cast::<u8>(), name.len()) };
            let name = &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())];
            let mut sanitized = CompactString::with_capacity(name.len());
            sanitized.extend(
                String::from_utf8_lossy(name)
                    .chars()
                    .map(|c| if c.is_control() { ' ' } else { c }),
            );
            processes.push(Process {
                pid: pid as u32,
                name: sanitized,
                cpu_percent,
                resident_bytes: info.ptinfo.pti_resident_size,
            });
        }
        self.previous_processes
            .retain(|_, counter| counter.at >= observed_since);
        Ok(processes)
    }
}

impl MetricsSource for MacSource {
    fn sample(&mut self) -> io::Result<Snapshot> {
        let cpu = self.cpu.sample()?;
        let memory = self.memory()?;
        let processes = self.processes()?;
        Ok(Snapshot {
            machine: self.machine.clone(),
            cpu,
            memory,
            processes,
        })
    }
}

impl Drop for MacSource {
    fn drop(&mut self) {
        unsafe {
            mach_port_deallocate(mach_task_self(), self.host);
        }
    }
}

pub(super) fn sysctl<T>(name: &CStr) -> io::Result<T> {
    let mut value = MaybeUninit::<T>::zeroed();
    let mut size = size_of::<T>();
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if size != size_of::<T>() {
        return Err(io::Error::other(format!(
            "unexpected size for {}",
            name.to_string_lossy()
        )));
    }
    Ok(unsafe { value.assume_init() })
}

fn sysctl_string(name: &CStr) -> io::Result<String> {
    let mut buffer = [0u8; 256];
    let mut size = buffer.len();
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(String::from_utf8_lossy(&buffer[..size])
        .trim_end_matches('\0')
        .to_string())
}

fn rate(current: u64, previous: u64, seconds: f64) -> f64 {
    if seconds > 0.0 {
        current.saturating_sub(previous) as f64 / seconds
    } else {
        0.0
    }
}
