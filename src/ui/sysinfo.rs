use sysinfo::System;

pub struct SysInfo {
    pub hostname: String,
    pub uptime:   String,
    pub cpu:      String,
    pub memory:   String,
    pub gpu:      String,
}

pub fn gather() -> SysInfo {
    let mut sys = System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();

    // sysinfo needs two samples separated by MINIMUM_CPU_UPDATE_INTERVAL (~200 ms)
    // to produce accurate per-CPU usage.
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_all();

    let hostname = System::host_name()
        .unwrap_or_else(|| "unknown".into())
        .split('.')
        .next()
        .unwrap_or("unknown")
        .to_string();
    let uptime   = format_uptime(System::uptime());

    let cpu_brand = sys.cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_else(|| "Unknown".into());
    let cpu_cores = sys.cpus().len();
    let cpu_freq  = sys.cpus().first().map(|c| c.frequency()).unwrap_or(0); // MHz
    let cpu_usage = sys.global_cpu_usage();
    let cpu = format!(
        "{} · {} cores · {} MHz · {:.1}% load",
        cpu_brand, cpu_cores, cpu_freq, cpu_usage
    );

    let used_gb  = sys.used_memory()  as f64 / 1_073_741_824.0;
    let total_gb = sys.total_memory() as f64 / 1_073_741_824.0;
    let pct      = if total_gb > 0.0 { used_gb / total_gb * 100.0 } else { 0.0 };
    let memory = format!("{:.2} GB / {:.2} GB ({:.1}%)", used_gb, total_gb, pct);

    let gpu = detect_gpu();

    SysInfo { hostname, uptime, cpu, memory, gpu }
}

fn format_uptime(secs: u64) -> String {
    let days  = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins  = (secs % 3600) / 60;
    match (days, hours, mins) {
        (0, 0, m) => format!("{}m", m),
        (0, h, m) => format!("{}h {}m", h, m),
        (d, h, m) => format!("{}d {}h {}m", d, h, m),
    }
}

// ── GPU detection ─────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn detect_gpu() -> String {
    let out = std::process::Command::new("system_profiler")
        .args(["SPDisplaysDataType", "-detailLevel", "basic"])
        .output()
        .ok();
    if let Some(out) = out {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let t = line.trim();
            if let Some(val) = t.strip_prefix("Chipset Model:").or_else(|| t.strip_prefix("Model:")) {
                let v = val.trim();
                if !v.is_empty() { return v.to_string(); }
            }
        }
    }
    "N/A".into()
}

#[cfg(target_os = "windows")]
fn detect_gpu() -> String {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("wmic")
        .args(["path", "win32_VideoController", "get", "name", "/value"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .ok();
    if let Some(out) = out {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(val) = line.strip_prefix("Name=") {
                let v = val.trim();
                if !v.is_empty() { return v.to_string(); }
            }
        }
    }
    "N/A".into()
}

#[cfg(target_os = "linux")]
fn detect_gpu() -> String {
    if let Ok(out) = std::process::Command::new("lspci").output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let lower = line.to_lowercase();
            if lower.contains("vga") || lower.contains("3d controller") || lower.contains("display controller") {
                // Format: "00:02.0 VGA compatible controller: Intel Corporation [device] (rev 07)"
                // Strip bus address + controller type, keep device name.
                if let Some(after_first_colon) = line.splitn(2, ':').nth(1) {
                    if let Some(device) = after_first_colon.splitn(2, ':').nth(1) {
                        let v = device.trim();
                        if !v.is_empty() { return v.to_string(); }
                    }
                }
            }
        }
    }
    "N/A".into()
}

// ── Formatted output lines ────────────────────────────────────────────────────

const B: &str = "\x1B[1m";   // bold
const G: &str = "\x1B[32m"; // green
const R: &str = "\x1B[0m";  // reset

pub fn format_line(info: &SysInfo) -> String {
    format!(
        "{B}{G}[sysinfo]{R} \
         {B}Host:{R} {} · \
         {B}Up:{R} {} · \
         {B}CPU:{R} {} · \
         {B}Mem:{R} {} · \
         {B}GPU:{R} {}",
        info.hostname, info.uptime, info.cpu, info.memory, info.gpu
    )
}
