use std::collections::HashSet;
use std::error::Error;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;

const SAMPLE_BUFFER_CAPACITY: usize = 256;
const DISK_SECTOR_BYTES: u64 = 512;

#[derive(Debug, Clone, PartialEq)]
struct Sample {
    timestamp_utc: String,
    elapsed_ms: u128,
    cpu_usage_percent: f64,
    memory_usage_percent: f64,
    memory_used_bytes: u64,
    memory_total_bytes: u64,
    disk_read_bytes_per_sec: f64,
    disk_write_bytes_per_sec: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CpuCounters {
    total: u64,
    idle: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MemoryUsage {
    used_bytes: u64,
    total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DiskCounters {
    read_sectors: u64,
    written_sectors: u64,
}

pub fn run(
    output_path: &Path,
    interval: Duration,
    duration: Duration,
) -> Result<(), Box<dyn Error>> {
    if interval.is_zero() || duration.is_zero() {
        return Err(String::from("interval and duration must be greater than zero").into());
    }

    let (sender, receiver) = sync_channel(SAMPLE_BUFFER_CAPACITY);
    let output_path = output_path.to_path_buf();
    let writer = thread::spawn(move || write_samples(&output_path, receiver));
    let sampler = thread::spawn(move || collect_samples(interval, duration, sender));

    let sampler_result = join_thread(sampler);
    let writer_result = join_thread(writer);
    sampler_result?;
    writer_result?;
    Ok(())
}

fn join_thread(
    thread: thread::JoinHandle<Result<(), Box<dyn Error + Send + Sync>>>,
) -> Result<(), Box<dyn Error>> {
    let result = thread
        .join()
        .map_err(|_| String::from("system_monitor worker thread panicked"))?;
    result.map_err(|error| error.to_string())?;
    Ok(())
}

fn collect_samples(
    interval: Duration,
    duration: Duration,
    sender: std::sync::mpsc::SyncSender<Sample>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let start = Instant::now();
    let end = start + duration;
    let mut previous_cpu = read_cpu_counters()?;
    let disk_devices = physical_block_devices()?;
    let mut previous_disk = read_disk_counters(&disk_devices)?;
    let mut previous_sample_time = start;
    let mut next_sample = start + interval;

    while next_sample <= end {
        let now = Instant::now();
        if now < next_sample {
            thread::sleep(next_sample - now);
        }

        let current_cpu = read_cpu_counters()?;
        let memory = read_memory_usage()?;
        let current_disk = read_disk_counters(&disk_devices)?;
        let sample_time = Instant::now();
        let elapsed = sample_time.duration_since(start);
        let (disk_read_bytes_per_sec, disk_write_bytes_per_sec) = disk_throughput(
            previous_disk,
            current_disk,
            sample_time - previous_sample_time,
        )?;
        sender
            .send(Sample {
                timestamp_utc: Utc::now().to_rfc3339(),
                elapsed_ms: elapsed.as_millis(),
                cpu_usage_percent: cpu_usage_percent(previous_cpu, current_cpu)?,
                memory_usage_percent: memory_usage_percent(memory),
                memory_used_bytes: memory.used_bytes,
                memory_total_bytes: memory.total_bytes,
                disk_read_bytes_per_sec,
                disk_write_bytes_per_sec,
            })
            .map_err(|_| String::from("system_monitor CSV writer stopped unexpectedly"))?;
        previous_cpu = current_cpu;
        previous_disk = current_disk;
        previous_sample_time = sample_time;

        next_sample += interval;
        while next_sample <= Instant::now() {
            next_sample += interval;
        }
    }

    Ok(())
}

fn write_samples(
    output_path: &Path,
    receiver: Receiver<Sample>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let mut writer = csv::Writer::from_writer(File::create(output_path)?);
    writer.write_record([
        "timestamp_utc",
        "elapsed_ms",
        "cpu_usage_percent",
        "memory_usage_percent",
        "memory_used_bytes",
        "memory_total_bytes",
        "disk_read_bytes_per_sec",
        "disk_write_bytes_per_sec",
    ])?;
    for sample in receiver {
        writer.write_record([
            sample.timestamp_utc,
            sample.elapsed_ms.to_string(),
            sample.cpu_usage_percent.to_string(),
            sample.memory_usage_percent.to_string(),
            sample.memory_used_bytes.to_string(),
            sample.memory_total_bytes.to_string(),
            sample.disk_read_bytes_per_sec.to_string(),
            sample.disk_write_bytes_per_sec.to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

fn read_cpu_counters() -> Result<CpuCounters, Box<dyn Error + Send + Sync>> {
    let mut contents = String::new();
    File::open("/proc/stat")?.read_to_string(&mut contents)?;
    parse_cpu_counters(&contents)
}

fn parse_cpu_counters(contents: &str) -> Result<CpuCounters, Box<dyn Error + Send + Sync>> {
    let line = contents
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or_else(|| String::from("/proc/stat does not contain aggregate CPU counters"))?;
    let values = line
        .split_whitespace()
        .skip(1)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() < 5 {
        return Err(String::from("/proc/stat aggregate CPU counters are incomplete").into());
    }

    Ok(CpuCounters {
        total: values.iter().sum(),
        idle: values[3] + values[4],
    })
}

fn cpu_usage_percent(
    previous: CpuCounters,
    current: CpuCounters,
) -> Result<f64, Box<dyn Error + Send + Sync>> {
    let total_delta = current
        .total
        .checked_sub(previous.total)
        .ok_or_else(|| String::from("/proc/stat CPU total counter moved backwards"))?;
    let idle_delta = current
        .idle
        .checked_sub(previous.idle)
        .ok_or_else(|| String::from("/proc/stat CPU idle counter moved backwards"))?;
    if total_delta == 0 {
        return Err(
            String::from("CPU counters did not advance during the sampling interval").into(),
        );
    }

    Ok((1.0 - idle_delta as f64 / total_delta as f64) * 100.0)
}

fn read_memory_usage() -> Result<MemoryUsage, Box<dyn Error + Send + Sync>> {
    let mut contents = String::new();
    File::open("/proc/meminfo")?.read_to_string(&mut contents)?;
    parse_memory_usage(&contents)
}

fn parse_memory_usage(contents: &str) -> Result<MemoryUsage, Box<dyn Error + Send + Sync>> {
    let total_kib = meminfo_value(contents, "MemTotal")?;
    let available_kib = meminfo_value(contents, "MemAvailable")?;
    let total_bytes = total_kib
        .checked_mul(1024)
        .ok_or_else(|| String::from("MemTotal is too large to represent in bytes"))?;
    let available_bytes = available_kib
        .checked_mul(1024)
        .ok_or_else(|| String::from("MemAvailable is too large to represent in bytes"))?;

    Ok(MemoryUsage {
        used_bytes: total_bytes.saturating_sub(available_bytes),
        total_bytes,
    })
}

fn meminfo_value(contents: &str, key: &str) -> Result<u64, Box<dyn Error + Send + Sync>> {
    let line = contents
        .lines()
        .find(|line| line.starts_with(&format!("{key}:")))
        .ok_or_else(|| format!("/proc/meminfo does not contain {key}"))?;
    line.split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("/proc/meminfo {key} value is missing"))?
        .parse::<u64>()
        .map_err(Into::into)
}

fn memory_usage_percent(memory: MemoryUsage) -> f64 {
    if memory.total_bytes == 0 {
        return 0.0;
    }
    memory.used_bytes as f64 / memory.total_bytes as f64 * 100.0
}

fn physical_block_devices() -> Result<HashSet<String>, Box<dyn Error + Send + Sync>> {
    let devices = fs::read_dir("/sys/block")?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !is_virtual_block_device(name))
        .collect::<HashSet<_>>();
    if devices.is_empty() {
        return Err(String::from("no physical block devices found in /sys/block").into());
    }
    Ok(devices)
}

fn is_virtual_block_device(name: &str) -> bool {
    ["loop", "ram", "zram", "dm-", "md", "sr"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn read_disk_counters(
    devices: &HashSet<String>,
) -> Result<DiskCounters, Box<dyn Error + Send + Sync>> {
    let mut contents = String::new();
    File::open("/proc/diskstats")?.read_to_string(&mut contents)?;
    parse_disk_counters(&contents, devices)
}

fn parse_disk_counters(
    contents: &str,
    devices: &HashSet<String>,
) -> Result<DiskCounters, Box<dyn Error + Send + Sync>> {
    let mut counters = DiskCounters {
        read_sectors: 0,
        written_sectors: 0,
    };
    let mut found_device = false;

    for line in contents.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(name) = fields.get(2) else {
            continue;
        };
        if !devices.contains(*name) {
            continue;
        }
        if fields.len() < 10 {
            return Err(format!("/proc/diskstats entry for {name} is incomplete").into());
        }
        counters.read_sectors = counters
            .read_sectors
            .checked_add(fields[5].parse::<u64>()?)
            .ok_or_else(|| String::from("disk read sector counter overflow"))?;
        counters.written_sectors = counters
            .written_sectors
            .checked_add(fields[9].parse::<u64>()?)
            .ok_or_else(|| String::from("disk write sector counter overflow"))?;
        found_device = true;
    }

    if !found_device {
        return Err(String::from("/proc/diskstats has no selected physical block devices").into());
    }
    Ok(counters)
}

fn disk_throughput(
    previous: DiskCounters,
    current: DiskCounters,
    elapsed: Duration,
) -> Result<(f64, f64), Box<dyn Error + Send + Sync>> {
    if elapsed.is_zero() {
        return Err(String::from("disk sampling interval must be greater than zero").into());
    }
    let read_sectors = current
        .read_sectors
        .checked_sub(previous.read_sectors)
        .ok_or_else(|| String::from("disk read sector counter moved backwards"))?;
    let written_sectors = current
        .written_sectors
        .checked_sub(previous.written_sectors)
        .ok_or_else(|| String::from("disk write sector counter moved backwards"))?;
    let seconds = elapsed.as_secs_f64();

    Ok((
        read_sectors as f64 * DISK_SECTOR_BYTES as f64 / seconds,
        written_sectors as f64 * DISK_SECTOR_BYTES as f64 / seconds,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::test_support::temp_path;
    use std::fs;

    #[test]
    fn parses_cpu_counters_and_calculates_usage() {
        let previous = parse_cpu_counters("cpu  100 20 30 700 50 0 0 0\n").unwrap();
        let current = parse_cpu_counters("cpu  200 20 30 780 50 20 0 0\n").unwrap();

        assert_eq!(
            previous,
            CpuCounters {
                total: 900,
                idle: 750
            }
        );
        assert_eq!(cpu_usage_percent(previous, current).unwrap(), 60.0);
    }

    #[test]
    fn parses_memory_usage() {
        let memory =
            parse_memory_usage("MemTotal:       1000 kB\nMemAvailable:    250 kB\n").unwrap();

        assert_eq!(memory.used_bytes, 750 * 1024);
        assert_eq!(memory.total_bytes, 1000 * 1024);
        assert_eq!(memory_usage_percent(memory), 75.0);
    }

    #[test]
    fn writes_csv_samples() {
        let output = temp_path("system_monitor.csv");
        let (sender, receiver) = sync_channel(1);
        sender
            .send(Sample {
                timestamp_utc: String::from("2026-01-01T00:00:00+00:00"),
                elapsed_ms: 1000,
                cpu_usage_percent: 12.5,
                memory_usage_percent: 25.0,
                memory_used_bytes: 256,
                memory_total_bytes: 1024,
                disk_read_bytes_per_sec: 512.0,
                disk_write_bytes_per_sec: 1024.0,
            })
            .unwrap();
        drop(sender);

        write_samples(&output, receiver).unwrap();

        assert_eq!(
            fs::read_to_string(&output).unwrap(),
            "timestamp_utc,elapsed_ms,cpu_usage_percent,memory_usage_percent,memory_used_bytes,memory_total_bytes,disk_read_bytes_per_sec,disk_write_bytes_per_sec\n2026-01-01T00:00:00+00:00,1000,12.5,25,256,1024,512,1024\n"
        );
        fs::remove_file(output).unwrap();
    }

    #[test]
    fn parses_disk_counters_for_physical_devices_only() {
        let devices = HashSet::from([String::from("sda"), String::from("nvme0n1")]);
        let counters = parse_disk_counters(
            concat!(
                "7 0 loop0 0 0 100 0 0 0 200 0 0 0 0\n",
                "8 0 sda 0 0 20 0 0 0 30 0 0 0 0\n",
                "8 1 sda1 0 0 40 0 0 0 50 0 0 0 0\n",
                "259 0 nvme0n1 0 0 10 0 0 0 15 0 0 0 0\n"
            ),
            &devices,
        )
        .unwrap();

        assert_eq!(
            counters,
            DiskCounters {
                read_sectors: 30,
                written_sectors: 45,
            }
        );
    }

    #[test]
    fn calculates_disk_throughput() {
        let throughput = disk_throughput(
            DiskCounters {
                read_sectors: 10,
                written_sectors: 20,
            },
            DiskCounters {
                read_sectors: 30,
                written_sectors: 50,
            },
            Duration::from_secs(2),
        )
        .unwrap();

        assert_eq!(throughput, (5120.0, 7680.0));
    }
}
