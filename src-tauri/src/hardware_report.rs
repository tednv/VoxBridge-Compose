use crate::gpu_info;
use std::sync::{Mutex, OnceLock};

fn cpu_times() -> Option<(u64, u64)> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::GetSystemTimes;

        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()? };
        let value = |time: FILETIME| {
            ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64
        };
        return Some((value(idle), value(kernel).saturating_add(value(user))));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let line = std::fs::read_to_string("/proc/stat")
            .ok()?
            .lines()
            .next()?
            .to_string();
        let values: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|value| value.parse().ok())
            .collect();
        let idle = values.get(3).copied().unwrap_or(0)
            + values.get(4).copied().unwrap_or(0);
        return Some((idle, values.iter().copied().sum()));
    }
}

pub(crate) fn system_cpu_usage_percent() -> Option<f64> {
    static PREVIOUS: OnceLock<Mutex<Option<(u64, u64)>>> = OnceLock::new();
    let current = cpu_times()?;
    let mut previous = PREVIOUS.get_or_init(|| Mutex::new(None)).lock().ok()?;
    let result = previous.and_then(|(idle, total)| {
        let total_delta = current.1.saturating_sub(total);
        let idle_delta = current.0.saturating_sub(idle);
        (total_delta > 0).then(|| {
            (100.0 * (1.0 - idle_delta as f64 / total_delta as f64)).clamp(0.0, 100.0)
        })
    });
    *previous = Some(current);
    result
}

#[cfg(target_os = "windows")]
pub(crate) fn system_memory_bytes() -> Option<(u64, u64)> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut status).ok()? };
    Some((status.ullTotalPhys, status.ullAvailPhys))
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn system_memory_bytes() -> Option<(u64, u64)> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    let read_kib = |key: &str| {
        contents
            .lines()
            .find(|line| line.starts_with(key))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u64>().ok())
            .map(|value| value.saturating_mul(1024))
    };
    Some((read_kib("MemTotal:")?, read_kib("MemAvailable:")?))
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / 1_073_741_824.0
}

#[cfg(target_os = "windows")]
fn cpu_model_name() -> String {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "/v",
            "ProcessorNameString",
        ])
        .output();

    if let Ok(output) = output {
        let text = String::from_utf8_lossy(&output.stdout);
        if let Some(line) = text.lines().find(|l| l.contains("ProcessorNameString")) {
            if let Some(value) = line.split("REG_SZ").nth(1) {
                let name = value.trim();
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
    }
    "Unknown CPU".to_string()
}

#[cfg(not(target_os = "windows"))]
fn cpu_model_name() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "Unknown CPU".to_string())
}

#[cfg(target_os = "windows")]
fn os_version_string() -> String {
    std::process::Command::new("cmd")
        .args(["/C", "ver"])
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Windows (version unknown)".to_string())
}

#[cfg(not(target_os = "windows"))]
fn os_version_string() -> String {
    std::env::consts::OS.to_string()
}

/// Builds a plain-text report of this machine's relevant transcription hardware/software,
/// for the user to paste into a GitHub issue. This is how maintainers can eventually build
/// known-good GPU "profiles" per hardware without needing every user's setup in front of
/// them — see the graceful GPU->CPU fallback in `local_whisper::ensure_model_loaded_with_fallback`,
/// which this report complements (the fallback keeps the app working today; this report is
/// how the underlying GPU support gets fixed over time).
pub fn build_hardware_report(last_gpu_error: Option<&str>) -> String {
    let cpu = cpu_model_name();
    let os = os_version_string();
    let vulkan_available = gpu_info::vulkan_runtime_available();
    let gpu = gpu_info::get_primary_gpu_vram_info();

    let mut report = String::new();
    report.push_str("VoxBridge Compose hardware report\n");
    report.push_str("========================\n");
    report.push_str(&format!("App version: {}\n", env!("CARGO_PKG_VERSION")));
    report.push_str(&format!("OS: {}\n", os));
    report.push_str(&format!("CPU: {}\n", cpu));
    if let Some((total, available)) = system_memory_bytes() {
        report.push_str(&format!(
            "System memory: {:.1} GB total, {:.1} GB used, {:.1} GB free\n",
            gib(total),
            gib(total.saturating_sub(available)),
            gib(available)
        ));
    }
    report.push_str(&format!("Vulkan runtime available: {}\n", vulkan_available));
    match gpu {
        Some(info) => {
            report.push_str(&format!(
                "GPU: {}\nGraphics memory: {:.1} GB dedicated, {:.1} GB used, {:.1} GB free\n",
                info.adapter_name,
                gib(info.dedicated_vram_bytes),
                gib(info.current_usage_bytes),
                gib(info.available_vram_bytes)
            ));
        }
        None => report.push_str("GPU: not detected\n"),
    }

    match last_gpu_error {
        Some(error) => {
            report.push_str("\nMost recent GPU transcription failure:\n");
            report.push_str(error);
            report.push('\n');
        }
        None => {
            report.push_str(
                "\nNo recent GPU failure recorded this session (report generated manually, \
                 or GPU worked fine — attach details of what you observed below).\n",
            );
        }
    }

    report.push_str(
        "\n(Generated by VoxBridge Compose's \"Report hardware\" tool. Pasting this into a GitHub issue \
         helps maintainers build a known-good GPU profile for this hardware.)\n",
    );

    report
}
