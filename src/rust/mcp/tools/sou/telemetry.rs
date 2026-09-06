use once_cell::sync::Lazy;
use serde::Serialize;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 当前 Sanshu 进程的资源采样结果；平台不具备对应能力时保留空值。
#[derive(Debug, Clone, Serialize)]
pub struct ResourceUsageSnapshot {
    pub cpu_percent: Option<f64>,
    pub system_cpu_percent: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub memory_available_bytes: Option<u64>,
    pub memory_total_bytes: Option<u64>,
    pub gpu_percent: Option<f64>,
    pub gpu_memory_bytes: Option<u64>,
    pub gpu_memory_total_bytes: Option<u64>,
    pub cpu_provider: Option<String>,
    pub gpu_provider: Option<String>,
    pub sampled_at: String,
    pub message: String,
}

/// 中文说明：供 embedding 资源策略使用的轻量快照，不把系统指标写入持久状态。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ResourcePressureSnapshot {
    pub process_cpu_percent: Option<f64>,
    pub system_cpu_percent: Option<f64>,
    pub process_memory_bytes: Option<u64>,
    pub memory_available_bytes: Option<u64>,
    pub memory_total_bytes: Option<u64>,
    pub gpu_percent: Option<f64>,
    pub gpu_memory_bytes: Option<u64>,
    pub gpu_memory_total_bytes: Option<u64>,
}

#[derive(Clone, Copy)]
struct CpuTimes {
    process_ticks: u64,
    system_ticks: u64,
    system_total_ticks: u64,
}

static LAST_CPU_TIMES: Lazy<Mutex<Option<CpuTimes>>> = Lazy::new(|| Mutex::new(None));
static LAST_GPU_SAMPLE: Lazy<Mutex<Option<(Instant, GpuSnapshot)>>> =
    Lazy::new(|| Mutex::new(None));

#[derive(Clone)]
struct GpuSnapshot {
    percent: Option<f64>,
    memory_bytes: Option<u64>,
    memory_total_bytes: Option<u64>,
    provider: Option<String>,
    message: String,
}

pub fn snapshot() -> ResourceUsageSnapshot {
    let pressure = pressure_snapshot();
    let mut unavailable = Vec::new();
    if pressure.process_cpu_percent.is_none() {
        unavailable.push("CPU");
    }
    if pressure.process_memory_bytes.is_none() {
        unavailable.push("内存");
    }
    if pressure.gpu_percent.is_none() {
        unavailable.push("GPU");
    }

    let message = if unavailable.is_empty() {
        format!(
            "已采样 CPU、进程内存和 {} GPU",
            if pressure.gpu_memory_total_bytes.is_some() {
                "nvidia-smi"
            } else {
                "可用"
            }
        )
    } else {
        format!(
            "已采样可用进程指标；{} 未提供 ({})",
            unavailable.join("、"),
            if pressure.gpu_percent.is_some() {
                "部分 GPU 指标可用"
            } else {
                "未检测到 nvidia-smi"
            }
        )
    };

    ResourceUsageSnapshot {
        cpu_percent: pressure.process_cpu_percent,
        system_cpu_percent: pressure.system_cpu_percent,
        memory_bytes: pressure.process_memory_bytes,
        memory_available_bytes: pressure.memory_available_bytes,
        memory_total_bytes: pressure.memory_total_bytes,
        gpu_percent: pressure.gpu_percent,
        gpu_memory_bytes: pressure.gpu_memory_bytes,
        gpu_memory_total_bytes: pressure.gpu_memory_total_bytes,
        cpu_provider: Some(cpu_provider().to_string()),
        gpu_provider: pressure
            .gpu_memory_total_bytes
            .is_some()
            .then(|| "nvidia-smi".to_string()),
        sampled_at: chrono::Utc::now().to_rfc3339(),
        message,
    }
}

pub(crate) fn pressure_snapshot() -> ResourcePressureSnapshot {
    let (process_cpu_percent, system_cpu_percent) = sample_cpu_percent();
    let (memory_available_bytes, memory_total_bytes) = system_memory_bytes();
    let gpu = sample_gpu();
    ResourcePressureSnapshot {
        process_cpu_percent,
        system_cpu_percent,
        process_memory_bytes: process_memory_bytes(),
        memory_available_bytes,
        memory_total_bytes,
        gpu_percent: gpu.percent,
        gpu_memory_bytes: gpu.memory_bytes,
        gpu_memory_total_bytes: gpu.memory_total_bytes,
    }
}

fn sample_cpu_percent() -> (Option<f64>, Option<f64>) {
    let current = match read_cpu_times() {
        Some(value) => value,
        None => return (None, None),
    };
    let percentages = LAST_CPU_TIMES.lock().ok().and_then(|mut previous| {
        let result = previous.and_then(|last| {
            let process_delta = current.process_ticks.saturating_sub(last.process_ticks);
            let active_delta = current.system_ticks.saturating_sub(last.system_ticks);
            let total_delta = current
                .system_total_ticks
                .saturating_sub(last.system_total_ticks);
            if active_delta == 0 || total_delta == 0 {
                None
            } else {
                let cpu_count = std::thread::available_parallelism()
                    .map(|value| value.get() as f64)
                    .unwrap_or(1.0);
                let process_percent =
                    ((process_delta as f64 / active_delta as f64) * 100.0 * cpu_count)
                        .clamp(0.0, 100.0);
                let system_percent =
                    ((active_delta as f64 / total_delta as f64) * 100.0).clamp(0.0, 100.0);
                Some((process_percent, system_percent))
            }
        });
        *previous = Some(current);
        result
    });
    let (process_percent, system_percent) = percentages
        .map(|(process, system)| (Some(process), Some(system)))
        .unwrap_or((None, None));
    (process_percent, system_percent)
}

fn cpu_provider() -> &'static str {
    #[cfg(windows)]
    {
        "GetProcessTimes"
    }
    #[cfg(target_os = "linux")]
    {
        "/proc"
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        "unavailable"
    }
}

#[cfg(windows)]
#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[cfg(windows)]
#[repr(C)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

#[cfg(windows)]
#[repr(C)]
struct MemoryStatusEx {
    dw_length: u32,
    dw_memory_load: u32,
    ull_total_phys: u64,
    ull_avail_phys: u64,
    ull_total_page_file: u64,
    ull_avail_page_file: u64,
    ull_total_virtual: u64,
    ull_avail_virtual: u64,
    ull_avail_extended_virtual: u64,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn GetProcessTimes(
        process: *mut std::ffi::c_void,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
}

#[cfg(windows)]
#[link(name = "psapi")]
extern "system" {
    fn GetProcessMemoryInfo(
        process: *mut std::ffi::c_void,
        counters: *mut ProcessMemoryCounters,
        size: u32,
    ) -> i32;
}

#[cfg(windows)]
fn file_time_value(value: &FileTime) -> u64 {
    ((value.high as u64) << 32) | value.low as u64
}

#[cfg(windows)]
fn read_cpu_times() -> Option<CpuTimes> {
    // 中文说明：Windows 使用进程时间与系统活动时间的差值计算本进程 CPU 占用。
    unsafe {
        let process = GetCurrentProcess();
        let mut creation = FileTime { low: 0, high: 0 };
        let mut exit = FileTime { low: 0, high: 0 };
        let mut kernel = FileTime { low: 0, high: 0 };
        let mut user = FileTime { low: 0, high: 0 };
        let mut idle = FileTime { low: 0, high: 0 };
        let mut system_kernel = FileTime { low: 0, high: 0 };
        let mut system_user = FileTime { low: 0, high: 0 };
        if GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) == 0
            || GetSystemTimes(&mut idle, &mut system_kernel, &mut system_user) == 0
        {
            return None;
        }
        let system_total = file_time_value(&system_kernel)
            .saturating_add(file_time_value(&system_user));
        let system_active = system_total.saturating_sub(file_time_value(&idle));
        Some(CpuTimes {
            process_ticks: file_time_value(&kernel).saturating_add(file_time_value(&user)),
            system_ticks: system_active,
            system_total_ticks: system_total,
        })
    }
}

#[cfg(target_os = "linux")]
fn read_cpu_times() -> Option<CpuTimes> {
    let process = std::fs::read_to_string("/proc/self/stat").ok()?;
    let process_tail = process.rsplit_once(')')?.1;
    let process_fields = process_tail.split_whitespace().collect::<Vec<_>>();
    let user_ticks = process_fields.get(11)?.parse::<u64>().ok()?;
    let system_ticks = process_fields.get(12)?.parse::<u64>().ok()?;

    let system = std::fs::read_to_string("/proc/stat").ok()?;
    let cpu_line = system.lines().find(|line| line.starts_with("cpu "))?;
    let values = cpu_line
        .split_whitespace()
        .skip(1)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()?;
    let total = values.iter().copied().sum::<u64>();
    let idle =
        values.get(3).copied().unwrap_or_default() + values.get(4).copied().unwrap_or_default();
    Some(CpuTimes {
        process_ticks: user_ticks.saturating_add(system_ticks),
        system_ticks: total.saturating_sub(idle),
        system_total_ticks: total,
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
fn read_cpu_times() -> Option<CpuTimes> {
    None
}

#[cfg(windows)]
fn process_memory_bytes() -> Option<u64> {
    // 中文说明：工作集大小比模型文件大小更能反映当前进程实际占用的物理内存。
    unsafe {
        let process = GetCurrentProcess();
        let mut counters = ProcessMemoryCounters {
            cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
            page_fault_count: 0,
            peak_working_set_size: 0,
            working_set_size: 0,
            quota_peak_paged_pool_usage: 0,
            quota_paged_pool_usage: 0,
            quota_peak_non_paged_pool_usage: 0,
            quota_non_paged_pool_usage: 0,
            pagefile_usage: 0,
            peak_pagefile_usage: 0,
        };
        let counter_size = counters.cb;
        (GetProcessMemoryInfo(process, &mut counters, counter_size) != 0)
            .then_some(counters.working_set_size as u64)
    }
}

#[cfg(target_os = "linux")]
fn process_memory_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:").map(str::trim))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    Some(value.saturating_mul(1024))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_memory_bytes() -> Option<u64> {
    None
}

#[cfg(windows)]
fn system_memory_bytes() -> (Option<u64>, Option<u64>) {
    // 中文说明：使用系统可用物理内存判断是否应暂停后台语义索引。
    unsafe {
        let mut status = MemoryStatusEx {
            dw_length: std::mem::size_of::<MemoryStatusEx>() as u32,
            dw_memory_load: 0,
            ull_total_phys: 0,
            ull_avail_phys: 0,
            ull_total_page_file: 0,
            ull_avail_page_file: 0,
            ull_total_virtual: 0,
            ull_avail_virtual: 0,
            ull_avail_extended_virtual: 0,
        };
        if GlobalMemoryStatusEx(&mut status) == 0 {
            return (None, None);
        }
        (Some(status.ull_avail_phys), Some(status.ull_total_phys))
    }
}

#[cfg(target_os = "linux")]
fn system_memory_bytes() -> (Option<u64>, Option<u64>) {
    let content = match std::fs::read_to_string("/proc/meminfo") {
        Ok(value) => value,
        Err(_) => return (None, None),
    };
    let parse_kib = |name: &str| {
        content.lines().find_map(|line| {
            let value = line.strip_prefix(name)?.split_whitespace().next()?;
            value
                .parse::<u64>()
                .ok()
                .map(|value| value.saturating_mul(1024))
        })
    };
    (parse_kib("MemAvailable:"), parse_kib("MemTotal:"))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn system_memory_bytes() -> (Option<u64>, Option<u64>) {
    (None, None)
}

fn sample_gpu() -> GpuSnapshot {
    if let Ok(cache) = LAST_GPU_SAMPLE.lock() {
        if let Some((sampled_at, snapshot)) = cache.as_ref() {
            if sampled_at.elapsed() < Duration::from_secs(2) {
                return snapshot.clone();
            }
        }
    }

    let snapshot = query_nvidia_smi().unwrap_or_else(|error| GpuSnapshot {
        percent: None,
        memory_bytes: None,
        memory_total_bytes: None,
        provider: None,
        message: error,
    });
    if let Ok(mut cache) = LAST_GPU_SAMPLE.lock() {
        *cache = Some((Instant::now(), snapshot.clone()));
    }
    snapshot
}

fn query_nvidia_smi() -> Result<GpuSnapshot, String> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu,memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .map_err(|_| "未检测到 nvidia-smi".to_string())?;
    if !output.status.success() {
        return Err("nvidia-smi 未返回可用 GPU 指标".to_string());
    }
    // line 与后续字段切片会持续借用解码结果，因此需将其保留到解析结束。
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|value| !value.trim().is_empty())
        .ok_or_else(|| "nvidia-smi 输出为空".to_string())?;
    let values = line.split(',').map(str::trim).collect::<Vec<_>>();
    let utilization = values
        .first()
        .ok_or_else(|| "GPU 利用率字段缺失".to_string())?
        .parse::<f64>()
        .map_err(|_| "GPU 利用率格式无效".to_string())?;
    let used_mib = values
        .get(1)
        .ok_or_else(|| "GPU 已用显存字段缺失".to_string())?
        .parse::<u64>()
        .map_err(|_| "GPU 已用显存格式无效".to_string())?;
    let total_mib = values
        .get(2)
        .ok_or_else(|| "GPU 总显存字段缺失".to_string())?
        .parse::<u64>()
        .map_err(|_| "GPU 总显存格式无效".to_string())?;
    Ok(GpuSnapshot {
        percent: Some(utilization.clamp(0.0, 100.0)),
        memory_bytes: Some(used_mib.saturating_mul(1024 * 1024)),
        memory_total_bytes: Some(total_mib.saturating_mul(1024 * 1024)),
        provider: Some("nvidia-smi".to_string()),
        message: "nvidia-smi".to_string(),
    })
}
