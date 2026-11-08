// Native channel discovery follows macmon's MIT-licensed IOReport research.
// See THIRD_PARTY_NOTICES.md. IOReport is a private Apple API; validate its output.
use std::{
    ffi::{CStr, CString, c_void},
    io, ptr,
};

use compact_str::CompactString;
use core_foundation_sys::{array::*, base::*, data::*, dictionary::*, string::*};

use super::{sysctl, sysctl_string};
use crate::metrics::{CpuTier, TierKind};

struct Owned(CFTypeRef);

impl Owned {
    fn new(value: CFTypeRef, operation: &str) -> io::Result<Self> {
        if value.is_null() {
            Err(io::Error::other(format!("{operation} returned no data")))
        } else {
            Ok(Self(value))
        }
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0);
        }
    }
}

struct IoObject(u32);
impl Drop for IoObject {
    fn drop(&mut self) {
        unsafe {
            IOObjectRelease(self.0);
        }
    }
}

struct Tier {
    kind: TierKind,
    count: usize,
    family: &'static str,
    frequencies: Vec<f64>,
}

#[derive(Default)]
struct CoreResidency {
    name: CompactString,
    active: f64,
    total: f64,
    weighted: f64,
    active_states: usize,
}

impl CoreResidency {
    fn add_state(&mut self, state: &str, residency: i64, frequencies: &[f64]) -> io::Result<()> {
        if residency < 0 {
            return Err(io::Error::other("CPU residency counter reset"));
        }
        let residency = residency as f64;
        self.total += residency;
        if state == "IDLE" || state == "DOWN" {
            return Ok(());
        }
        if !state.starts_with('V') || !state.contains('P') {
            return Err(io::Error::other(format!("unsupported CPU state {state}")));
        }
        self.active += residency;
        if let Some(frequency) = frequencies.get(self.active_states) {
            self.weighted += residency * frequency;
        }
        self.active_states += 1;
        Ok(())
    }
}

impl Tier {
    fn reading(&self, mut residencies: Vec<CoreResidency>) -> io::Result<CpuTier> {
        // Only duplicates before the first invalid interval take precedence over it.
        let valid_count = residencies
            .iter()
            .position(|core| core.total <= 0.0)
            .unwrap_or(residencies.len());
        let valid = &mut residencies[..valid_count];
        valid.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        if valid.windows(2).any(|pair| pair[0].name == pair[1].name) {
            return Err(io::Error::other("duplicate native CPU channel"));
        }
        if valid_count != residencies.len() {
            return Err(io::Error::other("CPU residency interval unavailable"));
        }
        if residencies.len() != self.count {
            return Err(io::Error::other(format!(
                "{}: expected {} core channels, received {}",
                self.kind.label(),
                self.count,
                residencies.len()
            )));
        }
        let active: f64 = residencies.iter().map(|core| core.active).sum();
        let weighted: f64 = residencies
            .iter()
            .map(|core| {
                if core.active_states == self.frequencies.len() {
                    core.weighted
                } else {
                    f64::NAN
                }
            })
            .sum();
        residencies.sort_unstable_by(|a, b| {
            core_order(&a.name, self.family)
                .cmp(&core_order(&b.name, self.family))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(CpuTier {
            kind: self.kind,
            cores: residencies
                .iter()
                .map(|core| (100.0 * core.active / core.total).clamp(0.0, 100.0))
                .collect(),
            frequency_ghz: (active > 0.0 && weighted.is_finite()).then(|| weighted / active),
            max_frequency_ghz: self.frequencies.last().copied(),
        })
    }
}

pub(super) struct Cpu {
    subscription: Owned,
    subscribed: Owned,
    previous: Owned,
    tiers: Vec<Tier>,
    string_buffer: Vec<u8>,
}

impl Cpu {
    pub fn new() -> io::Result<Self> {
        if std::env::consts::ARCH != "aarch64" {
            return Err(io::Error::other("live CPU metrics require Apple Silicon"));
        }
        let levels = sysctl::<u32>(c"hw.nperflevels")?;
        let mut tiers = Vec::new();
        for i in (0..levels).rev() {
            let name = CString::new(format!("hw.perflevel{i}.name")).unwrap();
            let count = CString::new(format!("hw.perflevel{i}.physicalcpu")).unwrap();
            let kind = match sysctl_string(&name)?.as_str() {
                "Efficiency" => TierKind::Efficiency,
                "Performance" => TierKind::Performance,
                "Super" => TierKind::Super,
                name => return Err(io::Error::other(format!("unsupported CPU tier {name}"))),
            };
            let count = sysctl::<u32>(&count)? as usize;
            if count > 0 {
                tiers.push(Tier {
                    kind,
                    count,
                    family: "",
                    frequencies: Vec::new(),
                });
            }
        }
        if tiers.is_empty() {
            return Err(io::Error::other("no CPU tiers discovered"));
        }
        let has_super = tiers.iter().any(|tier| tier.kind == TierKind::Super);
        let (low, high) = frequencies()?;
        let tier_count = tiers.len();
        for (i, tier) in tiers.iter_mut().enumerate() {
            tier.family = match tier.kind {
                TierKind::Efficiency => "ECPU",
                TierKind::Performance if has_super => "MCPU",
                TierKind::Performance | TierKind::Super => "PCPU",
            };
            // Do not assume a three-tier chip's middle and highest clocks match.
            tier.frequencies = if i == tier_count - 1 {
                high.clone()
            } else if i == 0 {
                low.clone()
            } else {
                Vec::new()
            };
        }
        unsafe {
            let group = cf_string("CPU Stats")?;
            let subgroup = cf_string("CPU Core Performance States")?;
            let available = Owned::new(
                IOReportCopyChannelsInGroup(group.0.cast(), subgroup.0.cast(), 0, 0, 0).cast(),
                "IOReportCopyChannelsInGroup",
            )?;
            if channels(available.0)?.is_empty() {
                return Err(io::Error::other(
                    "no native CPU residency channels available",
                ));
            }
            let requested = Owned::new(
                CFDictionaryCreateMutableCopy(ptr::null(), 0, available.0.cast()).cast(),
                "channel dictionary",
            )?;
            let mut subscribed = ptr::null_mut();
            let raw_subscription = IOReportCreateSubscription(
                ptr::null(),
                requested.0.cast_mut().cast(),
                &mut subscribed,
                0,
                ptr::null(),
            );
            // Adopt both out parameters before returning either error.
            let subscription = Owned::new(raw_subscription, "IOReportCreateSubscription");
            let subscribed = Owned::new(subscribed.cast(), "subscribed CPU channels");
            let (subscription, subscribed) = (subscription?, subscribed?);
            let previous = capture(&subscription, &subscribed)?;
            Ok(Self {
                subscription,
                subscribed,
                previous,
                tiers,
                string_buffer: Vec::new(),
            })
        }
    }

    pub fn sample(&mut self) -> io::Result<Vec<CpuTier>> {
        let current = capture(&self.subscription, &self.subscribed)?;
        let delta = unsafe {
            Owned::new(
                IOReportCreateSamplesDelta(self.previous.0.cast(), current.0.cast(), ptr::null())
                    .cast(),
                "CPU sample delta",
            )
        };
        self.previous = current;
        let delta = delta?;
        let mut residencies: Vec<Vec<CoreResidency>> =
            self.tiers.iter().map(|_| Vec::new()).collect();
        for channel in channels(delta.0)? {
            let name = string(
                unsafe { IOReportChannelGetChannelName(channel) },
                &mut self.string_buffer,
            )?;
            let Some(index) = self
                .tiers
                .iter()
                .position(|tier| name.contains(tier.family))
            else {
                continue;
            };
            let count = unsafe { IOReportStateGetCount(channel) };
            if !(1..=128).contains(&count) {
                return Err(io::Error::other("unsupported CPU state count"));
            }
            let mut core = CoreResidency {
                name: name.into(),
                ..CoreResidency::default()
            };
            for i in 0..count {
                let state = string(
                    unsafe { IOReportStateGetNameForIndex(channel, i) },
                    &mut self.string_buffer,
                )?;
                let residency = unsafe { IOReportStateGetResidency(channel, i) };
                core.add_state(state, residency, &self.tiers[index].frequencies)?;
            }
            residencies[index].push(core);
        }
        self.tiers
            .iter()
            .zip(residencies)
            .map(|(tier, cores)| tier.reading(cores))
            .collect()
    }
}

fn core_order(name: &str, family: &str) -> (u32, u32, u32) {
    let die = name
        .strip_prefix("DIE_")
        .and_then(|s| s.split('_').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let suffix = name
        .split_once(family)
        .map(|(_, suffix)| suffix)
        .unwrap_or("");
    let (cluster, core) = suffix.split_once("_CPU").unwrap_or(("0", suffix));
    (die, cluster.parse().unwrap_or(0), core.parse().unwrap_or(0))
}

fn capture(subscription: &Owned, subscribed: &Owned) -> io::Result<Owned> {
    unsafe {
        Owned::new(
            IOReportCreateSamples(subscription.0, subscribed.0.cast_mut().cast(), ptr::null())
                .cast(),
            "IOReportCreateSamples",
        )
    }
}

fn frequencies() -> io::Result<(Vec<f64>, Vec<f64>)> {
    unsafe {
        let mut iterator = 0;
        let matching = IOServiceMatching(c"AppleARMIODevice".as_ptr());
        if matching.is_null() || IOServiceGetMatchingServices(0, matching, &mut iterator) != 0 {
            return Err(io::Error::other(
                "cannot enumerate native CPU frequency tables",
            ));
        }
        let iterator = IoObject(iterator);
        let (mut low, mut high) = (Vec::new(), Vec::new());
        loop {
            let entry = IOIteratorNext(iterator.0);
            if entry == 0 {
                break;
            }
            let entry = IoObject(entry);
            let mut name = [0i8; 128];
            if IORegistryEntryGetName(entry.0, name.as_mut_ptr()) != 0 {
                continue;
            }
            let name = CStr::from_ptr(name.as_ptr()).to_bytes();
            if name != b"pmgr" && name != b"pmgr-child" {
                continue;
            }
            let mut properties = ptr::null_mut();
            if IORegistryEntryCreateCFProperties(entry.0, &mut properties, ptr::null(), 0) != 0 {
                continue;
            }
            let properties = Owned::new(properties.cast(), "CPU frequency properties")?;
            let mut keys = (
                "voltage-states1-sram".to_string(),
                "voltage-states5-sram".to_string(),
            );
            if let Some(clusters) = data(properties.0, "acc-clusters")? {
                let mut clusters: Vec<_> = clusters.chunks_exact(8).map(|c| (c[1], c[0])).collect();
                clusters.sort_unstable();
                clusters.dedup();
                if clusters.len() >= 2 {
                    keys = (
                        format!("voltage-states{}-sram", clusters[clusters.len() - 2].1),
                        format!("voltage-states{}-sram", clusters[clusters.len() - 1].1),
                    );
                }
            }
            for (target, legacy, discovered) in [
                (&mut low, "voltage-states1-sram", keys.0),
                (&mut high, "voltage-states5-sram", keys.1),
            ] {
                if !target.is_empty() {
                    continue;
                }
                let values = data(properties.0, legacy)?.or(data(properties.0, &discovered)?);
                if let Some(values) = values.filter(|v| !v.is_empty() && v.len() % 8 == 0) {
                    let raw: Vec<_> = values
                        .chunks_exact(8)
                        .map(|v| u32::from_le_bytes(v[..4].try_into().unwrap()))
                        .collect();
                    // CPU DVFS tables moved from Hz to kHz with M4. Both encode GHz here.
                    let divisor = if raw.iter().copied().max().unwrap_or(0) > 100_000_000 {
                        1e9
                    } else {
                        1e6
                    };
                    if raw.iter().all(|f| *f > 0) && raw.windows(2).all(|p| p[0] <= p[1]) {
                        *target = raw.into_iter().map(|v| v as f64 / divisor).collect();
                    }
                }
            }
        }
        Ok((low, high))
    }
}

fn cf_string(value: &str) -> io::Result<Owned> {
    unsafe {
        Owned::new(
            CFStringCreateWithBytes(
                ptr::null(),
                value.as_ptr(),
                value.len() as isize,
                kCFStringEncodingUTF8,
                0,
            )
            .cast(),
            "CFStringCreateWithBytes",
        )
    }
}

fn string(value: CFStringRef, buffer: &mut Vec<u8>) -> io::Result<&str> {
    unsafe {
        if value.is_null() || CFGetTypeID(value.cast()) != CFStringGetTypeID() {
            return Err(io::Error::other("invalid native CPU string"));
        }
        let size =
            CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), kCFStringEncodingUTF8) + 1;
        buffer.resize(size as usize, 0);
        if CFStringGetCString(
            value,
            buffer.as_mut_ptr().cast(),
            size,
            kCFStringEncodingUTF8,
        ) == 0
        {
            return Err(io::Error::other("cannot decode native CPU string"));
        }
        CStr::from_ptr(buffer.as_ptr().cast())
            .to_str()
            .map_err(|_| io::Error::other("cannot decode native CPU string"))
    }
}

fn get(dictionary: CFTypeRef, key: &str) -> io::Result<CFTypeRef> {
    unsafe {
        if dictionary.is_null() || CFGetTypeID(dictionary) != CFDictionaryGetTypeID() {
            return Err(io::Error::other("invalid native CPU dictionary"));
        }
        Ok(CFDictionaryGetValue(dictionary.cast(), cf_string(key)?.0))
    }
}

fn data(dictionary: CFTypeRef, key: &str) -> io::Result<Option<Vec<u8>>> {
    let value = get(dictionary, key)?;
    unsafe {
        if value.is_null() {
            return Ok(None);
        }
        if CFGetTypeID(value) != CFDataGetTypeID() {
            return Err(io::Error::other("invalid native DVFS table"));
        }
        let len = CFDataGetLength(value.cast()) as usize;
        if len == 0 {
            return Ok(Some(Vec::new()));
        }
        Ok(Some(
            std::slice::from_raw_parts(CFDataGetBytePtr(value.cast()), len).to_vec(),
        ))
    }
}

fn channels(dictionary: CFTypeRef) -> io::Result<Vec<CFDictionaryRef>> {
    let values = get(dictionary, "IOReportChannels")?;
    unsafe {
        if values.is_null() || CFGetTypeID(values) != CFArrayGetTypeID() {
            return Err(io::Error::other("invalid native CPU channel array"));
        }
        (0..CFArrayGetCount(values.cast()))
            .map(|i| {
                let value = CFArrayGetValueAtIndex(values.cast(), i);
                if value.is_null() || CFGetTypeID(value) != CFDictionaryGetTypeID() {
                    Err(io::Error::other("invalid native CPU channel"))
                } else {
                    Ok(value.cast())
                }
            })
            .collect()
    }
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const i8) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(
        port: u32,
        matching: CFDictionaryRef,
        iterator: *mut u32,
    ) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IORegistryEntryGetName(entry: u32, name: *mut i8) -> i32;
    fn IORegistryEntryCreateCFProperties(
        entry: u32,
        properties: *mut CFMutableDictionaryRef,
        allocator: CFAllocatorRef,
        options: u32,
    ) -> i32;
}

#[link(name = "IOReport")]
unsafe extern "C" {
    fn IOReportCopyChannelsInGroup(
        group: CFStringRef,
        subgroup: CFStringRef,
        a: u64,
        b: u64,
        c: u64,
    ) -> CFDictionaryRef;
    fn IOReportCreateSubscription(
        allocator: *const c_void,
        requested: CFMutableDictionaryRef,
        subscribed: *mut CFMutableDictionaryRef,
        channel: u64,
        options: CFTypeRef,
    ) -> CFTypeRef;
    fn IOReportCreateSamples(
        subscription: CFTypeRef,
        channels: CFMutableDictionaryRef,
        options: CFTypeRef,
    ) -> CFDictionaryRef;
    fn IOReportCreateSamplesDelta(
        previous: CFDictionaryRef,
        current: CFDictionaryRef,
        options: CFTypeRef,
    ) -> CFDictionaryRef;
    fn IOReportChannelGetChannelName(channel: CFDictionaryRef) -> CFStringRef;
    fn IOReportStateGetCount(channel: CFDictionaryRef) -> i32;
    fn IOReportStateGetNameForIndex(channel: CFDictionaryRef, index: i32) -> CFStringRef;
    fn IOReportStateGetResidency(channel: CFDictionaryRef, index: i32) -> i64;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(frequencies: Vec<f64>, cores: &[(&str, [i64; 4])]) -> io::Result<CpuTier> {
        let tier = Tier {
            kind: TierKind::Efficiency,
            count: cores.len(),
            family: "ECPU",
            frequencies,
        };
        tier.reading(
            cores
                .iter()
                .map(|(name, residencies)| {
                    let mut core = CoreResidency {
                        name: (*name).into(),
                        ..CoreResidency::default()
                    };
                    for (state, &residency) in ["DOWN", "IDLE", "V0P0", "V1P0"]
                        .into_iter()
                        .zip(residencies)
                    {
                        core.add_state(state, residency, &tier.frequencies)?;
                    }
                    Ok(core)
                })
                .collect::<io::Result<_>>()?,
        )
    }

    #[test]
    fn residency_becomes_activity_and_active_weighted_frequency() {
        let tier = reading(vec![1.0, 2.0], &[("ECPU0", [25, 50, 10, 15])]).unwrap();

        assert_eq!(tier.kind, TierKind::Efficiency);
        assert_eq!(tier.cores, [25.0]);
        assert_eq!(tier.active_percent(), 25.0);
        assert_eq!(tier.frequency_ghz, Some(1.6));
        assert_eq!(tier.max_frequency_ghz, Some(2.0));
    }

    #[test]
    fn missing_or_mismatched_frequencies_preserve_activity() {
        for frequencies in [vec![], vec![1.0], vec![1.0, 2.0, 3.0]] {
            let tier = reading(frequencies, &[("ECPU0", [25, 50, 10, 15])]).unwrap();

            assert_eq!(tier.cores, [25.0]);
            assert_eq!(tier.active_percent(), 25.0);
            assert_eq!(tier.frequency_ghz, None);
        }
    }

    #[test]
    fn idle_interval_has_no_activity_or_current_frequency() {
        let tier = reading(vec![1.0, 2.0], &[("ECPU0", [0, 100, 0, 0])]).unwrap();

        assert_eq!(tier.cores, [0.0]);
        assert_eq!(tier.active_percent(), 0.0);
        assert_eq!(tier.frequency_ghz, None);
        assert_eq!(tier.max_frequency_ghz, Some(2.0));
    }

    #[test]
    fn tier_frequency_weights_active_residency_across_cores() {
        let tier = reading(
            vec![1.0, 2.0],
            &[("ECPU1", [0, 25, 0, 75]), ("ECPU0", [25, 50, 10, 15])],
        )
        .unwrap();

        assert_eq!(tier.cores, [25.0, 75.0]);
        assert_eq!(tier.active_percent(), 50.0);
        assert_eq!(tier.frequency_ghz, Some(1.9));
        assert_eq!(tier.max_frequency_ghz, Some(2.0));
    }

    #[test]
    fn invalid_states_are_rejected_even_without_residency() {
        for state in ["", "P0V0", "V0", "IDLE0", "DOWN0"] {
            let error = CoreResidency::default()
                .add_state(state, 0, &[1.0])
                .unwrap_err();
            assert_eq!(error.to_string(), format!("unsupported CPU state {state}"));
        }
        for state in ["DOWN", "IDLE", "V0P0", "V12P3"] {
            CoreResidency::default()
                .add_state(state, 0, &[1.0])
                .unwrap();
        }
    }

    #[test]
    fn negative_residencies_and_zero_intervals_are_rejected() {
        for residencies in [
            [-1, 100, 0, 0],
            [0, -1, 100, 0],
            [0, 0, -1, 100],
            [0, 0, 100, -1],
        ] {
            assert_eq!(
                reading(vec![1.0, 2.0], &[("ECPU0", residencies)])
                    .unwrap_err()
                    .to_string(),
                "CPU residency counter reset"
            );
        }
        assert_eq!(
            reading(vec![1.0, 2.0], &[("ECPU0", [0; 4])])
                .unwrap_err()
                .to_string(),
            "CPU residency interval unavailable"
        );
    }

    #[test]
    fn duplicate_full_channel_names_are_rejected() {
        assert_eq!(
            reading(vec![1.0, 2.0], &[("DIE_0_ECPU0_CPU0", [0, 50, 25, 25]); 2],)
                .unwrap_err()
                .to_string(),
            "duplicate native CPU channel"
        );
    }

    #[test]
    fn full_channel_identity_and_numeric_order_are_preserved() {
        let tier = reading(
            vec![1.0, 2.0],
            &[
                ("DIE_10_ECPU2_CPU0", [0, 0, 100, 0]),
                ("DIE_2_ECPU10_CPU0", [0, 10, 90, 0]),
                ("DIE_2_ECPU2_CPU10", [0, 20, 80, 0]),
                ("DIE_2_ECPU2_CPU2", [0, 30, 70, 0]),
                ("DIE_2_ECPU2_CPU0", [0, 40, 60, 0]),
            ],
        )
        .unwrap();

        assert_eq!(tier.cores, [60.0, 70.0, 80.0, 90.0, 100.0]);
        assert_eq!(tier.frequency_ghz, Some(1.0));
    }
}
