//! Host resource samples for authenticated worker inventory.

use std::{
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use osdl_proto::v1::WorkerStatus;

const MIN_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(crate) struct WorkerTelemetry {
    last: Mutex<Option<Sample>>,
}

struct Sample {
    at: Instant,
    cpu: Option<CpuTime>,
    status: WorkerStatus,
}

#[derive(Clone, Copy)]
struct CpuTime {
    total: u64,
    idle: u64,
}

impl CpuTime {
    fn percent_since(self, previous: Self) -> Option<f32> {
        let total = self.total.checked_sub(previous.total)?;
        let idle = self.idle.checked_sub(previous.idle)?;
        let busy = total.checked_sub(idle)?;
        (total != 0).then(|| (100.0 * busy as f64 / total as f64) as f32)
    }
}

impl WorkerTelemetry {
    pub(crate) fn sample(&self, worker: &str) -> WorkerStatus {
        let mut last = self.last.lock().unwrap();
        let at = Instant::now();
        if let Some(previous) = last
            .as_ref()
            .filter(|item| at - item.at < MIN_SAMPLE_INTERVAL)
        {
            return previous.status.clone();
        }

        let (cpu, memory) = platform_sample();
        let cpu_percent = cpu.and_then(|current| current.percent_since(last.as_ref()?.cpu?));
        let sampled_at_ms = (cpu_percent.is_some() || memory.is_some())
            .then(|| {
                u64::try_from(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .ok()?
                        .as_millis(),
                )
                .ok()
            })
            .flatten();
        let status = WorkerStatus {
            id: worker.into(),
            available: true,
            sampled_at_ms,
            cpu_percent,
            memory_used_bytes: memory.map(|(used, _)| used),
            memory_total_bytes: memory.map(|(_, total)| total),
        };
        *last = Some(Sample {
            at,
            cpu,
            status: status.clone(),
        });
        status
    }
}

#[cfg(target_os = "linux")]
fn platform_sample() -> (Option<CpuTime>, Option<(u64, u64)>) {
    use std::{
        fs,
        io::{BufRead, BufReader},
    };

    let cpu = fs::File::open("/proc/stat").ok().and_then(|file| {
        let mut line = String::new();
        BufReader::new(file).read_line(&mut line).ok()?;
        parse_cpu(&line)
    });
    let memory = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| parse_memory(&text));
    (cpu, memory)
}

#[cfg(any(target_os = "linux", test))]
fn parse_cpu(line: &str) -> Option<CpuTime> {
    let mut fields = line.split_whitespace();
    (fields.next()? == "cpu").then_some(())?;
    let mut counters = [0u64; 8];
    for counter in &mut counters {
        *counter = fields.next()?.parse().ok()?;
    }
    let total = counters.into_iter().try_fold(0u64, u64::checked_add)?;
    let idle = counters[3].checked_add(counters[4])?;
    Some(CpuTime { total, idle })
}

#[cfg(any(target_os = "linux", test))]
fn parse_memory(contents: &str) -> Option<(u64, u64)> {
    let mut total = None;
    let mut available = None;
    for line in contents.lines() {
        let mut fields = line.split_whitespace();
        let destination = match fields.next() {
            Some("MemTotal:") => &mut total,
            Some("MemAvailable:") => &mut available,
            _ => continue,
        };
        let kilobytes: u64 = fields.next()?.parse().ok()?;
        (fields.next()? == "kB").then_some(())?;
        *destination = kilobytes.checked_mul(1024);
        if let (Some(total), Some(available)) = (total, available) {
            return Some((total.checked_sub(available)?, total));
        }
    }
    None
}

#[cfg(windows)]
fn platform_sample() -> (Option<CpuTime>, Option<(u64, u64)>) {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::{
            SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
            Threading::GetSystemTimes,
        },
    };
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    let mut idle = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut kernel = idle;
    let mut user = idle;
    // These APIs fill caller-owned fixed-size integer structures.
    let cpu = (unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } != 0)
        .then(|| {
            Some(CpuTime {
                total: ticks(kernel).checked_add(ticks(user))?,
                idle: ticks(idle),
            })
        })
        .flatten();
    // MEMORYSTATUSEX consists of integers and requires dwLength before the call.
    let mut memory: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    memory.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    let memory = (unsafe { GlobalMemoryStatusEx(&mut memory) } != 0)
        .then(|| {
            Some((
                memory.ullTotalPhys.checked_sub(memory.ullAvailPhys)?,
                memory.ullTotalPhys,
            ))
        })
        .flatten();
    (cpu, memory)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_sample() -> (Option<CpuTime>, Option<(u64, u64)>) {
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_inventory_caches_a_host_snapshot_without_fabricating_cpu() {
        let telemetry = WorkerTelemetry::default();
        let initial = telemetry.sample("fixture-worker");
        assert_eq!(initial.id, "fixture-worker");
        assert!(initial.available);
        assert_eq!(initial.cpu_percent, None);
        assert_eq!(telemetry.sample("fixture-worker"), initial);
    }

    #[test]
    fn cpu_uses_idle_deltas_without_counting_guest_twice() {
        let first = parse_cpu("cpu 10 0 5 45 5 0 0 0 2 0").unwrap();
        let second = parse_cpu("cpu 12 0 8 50 5 0 0 0 3 0").unwrap();
        assert_eq!(second.percent_since(first), Some(50.0));
        assert_eq!(first.percent_since(second), None);
        assert_eq!(first.percent_since(first), None);
    }

    #[test]
    fn memory_reports_bytes_and_preserves_unavailable_data() {
        assert_eq!(
            parse_memory("MemTotal: 2048 kB\nMemFree: 5 kB\nMemAvailable: 1024 kB"),
            Some((1048576, 2097152))
        );
        assert_eq!(parse_memory("MemTotal: 2048 kB\nMemFree: 5 kB"), None);
        assert_eq!(parse_memory("MemTotal: 2 kB\nMemAvailable: 3 kB"), None);
    }
}
