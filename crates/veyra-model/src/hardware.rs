//! Hardware detection for `veyra doctor` and `/model auto`.
//!
//! Detection is best-effort and never fails: probes that are unavailable on a
//! platform simply report nothing. External probes (nvidia-smi,
//! system_profiler, ...) run with a timeout.

use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Gpu {
    pub vendor: String,
    pub name: String,
    /// Dedicated VRAM in bytes, when known.
    pub vram_bytes: Option<u64>,
    pub vram_free_bytes: Option<u64>,
    pub driver: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HardwareInfo {
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub cpu: String,
    pub cpu_cores: usize,
    pub total_ram: u64,
    pub available_ram: u64,
    pub gpus: Vec<Gpu>,
    /// Apple Silicon: GPU shares system RAM.
    pub unified_memory: bool,
    pub metal: bool,
    pub cuda: bool,
    pub vulkan: bool,
    pub rocm: bool,
    /// Free disk space where Veyra stores models.
    pub disk_free: Option<u64>,
}

impl HardwareInfo {
    pub fn detect(models_dir: &std::path::Path) -> HardwareInfo {
        use sysinfo::{Disks, System};
        let mut sys = System::new();
        sys.refresh_memory();
        sys.refresh_cpu_all();
        let cpu = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();
        let mut info = HardwareInfo {
            os: std::env::consts::OS.to_string(),
            os_version: System::long_os_version().unwrap_or_default(),
            arch: std::env::consts::ARCH.to_string(),
            cpu,
            cpu_cores: sys.cpus().len(),
            total_ram: sys.total_memory(),
            available_ram: sys.available_memory(),
            ..Default::default()
        };

        // Disk with the longest mount point that prefixes the models dir.
        let target = existing_ancestor(models_dir);
        let disks = Disks::new_with_refreshed_list();
        info.disk_free = disks
            .list()
            .iter()
            .filter(|d| target.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .map(|d| d.available_space());

        match info.os.as_str() {
            "macos" => detect_macos(&mut info),
            "linux" => detect_linux(&mut info),
            "windows" => detect_windows(&mut info),
            _ => {}
        }
        if !info.cuda {
            detect_nvidia(&mut info);
        }
        info
    }

    /// Memory the model runtime can realistically use for weights + KV cache
    /// with full GPU offload (or CPU inference when there is no GPU).
    pub fn fast_memory_budget(&self) -> u64 {
        if self.unified_memory {
            // macOS lets the GPU wire roughly 65-75% of RAM by default.
            return (self.total_ram as f64 * 0.70) as u64;
        }
        if let Some(v) = self.gpus.iter().filter_map(|g| g.vram_bytes).max() {
            if v > 2_000_000_000 {
                return (v as f64 * 0.92) as u64;
            }
        }
        self.cpu_memory_budget()
    }

    /// RAM usable for CPU inference or for offloaded layers.
    pub fn cpu_memory_budget(&self) -> u64 {
        let by_total = (self.total_ram as f64 * 0.60) as u64;
        let by_avail = (self.available_ram as f64 * 0.85) as u64;
        by_total.max(by_avail).min(self.total_ram.saturating_sub(2_000_000_000))
    }

    pub fn has_discrete_gpu(&self) -> bool {
        !self.unified_memory && self.gpus.iter().any(|g| g.vram_bytes.unwrap_or(0) > 2_000_000_000)
    }

    pub fn accelerator(&self) -> &'static str {
        if self.metal {
            "Metal"
        } else if self.cuda {
            "CUDA"
        } else if self.rocm {
            "ROCm"
        } else if self.vulkan {
            "Vulkan"
        } else {
            "CPU"
        }
    }
}

fn existing_ancestor(p: &std::path::Path) -> std::path::PathBuf {
    let mut cur = p.to_path_buf();
    while !cur.exists() {
        if !cur.pop() {
            break;
        }
    }
    std::fs::canonicalize(&cur).unwrap_or(cur)
}

/// Run a probe with a timeout; returns stdout on success.
pub fn probe(cmd: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                use std::io::Read;
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return None,
        }
    }
}

fn detect_macos(info: &mut HardwareInfo) {
    let apple_silicon = info.arch == "aarch64";
    info.unified_memory = apple_silicon;
    info.metal = apple_silicon;
    if let Some(json) = probe("system_profiler", &["SPDisplaysDataType", "-json"], Duration::from_secs(8)) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            if let Some(arr) = v.get("SPDisplaysDataType").and_then(|a| a.as_array()) {
                for g in arr {
                    let name = g
                        .get("sppci_model")
                        .or_else(|| g.get("_name"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("GPU")
                        .to_string();
                    let vendor = g
                        .get("spdisplays_vendor")
                        .and_then(|x| x.as_str())
                        .unwrap_or(if apple_silicon { "Apple" } else { "" })
                        .trim_start_matches("sppci_vendor_")
                        .to_string();
                    let vram = g
                        .get("spdisplays_vram")
                        .or_else(|| g.get("spdisplays_vram_shared"))
                        .and_then(|x| x.as_str())
                        .and_then(parse_size);
                    info.gpus.push(Gpu {
                        vendor,
                        name,
                        vram_bytes: if apple_silicon { None } else { vram },
                        ..Default::default()
                    });
                }
            }
        }
    }
    if !apple_silicon && !info.gpus.is_empty() {
        // Intel Macs with AMD GPUs still use Metal in llama.cpp.
        info.metal = true;
    }
}

fn detect_linux(info: &mut HardwareInfo) {
    info.vulkan = probe("vulkaninfo", &["--summary"], Duration::from_secs(5)).is_some()
        || [
            "/usr/lib/x86_64-linux-gnu/libvulkan.so.1",
            "/usr/lib/aarch64-linux-gnu/libvulkan.so.1",
            "/usr/lib64/libvulkan.so.1",
            "/usr/lib/libvulkan.so.1",
        ]
        .iter()
        .any(|p| std::path::Path::new(p).exists());
    if let Some(out) = probe("rocm-smi", &["--showproductname", "--showmeminfo", "vram", "--json"], Duration::from_secs(5)) {
        info.rocm = true;
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&out) {
            if let Some(obj) = v.as_object() {
                for (_, card) in obj {
                    let name = card.get("Card series").or_else(|| card.get("Card model")).and_then(|x| x.as_str()).unwrap_or("AMD GPU");
                    let vram = card
                        .get("VRAM Total Memory (B)")
                        .and_then(|x| x.as_str())
                        .and_then(|s| s.parse().ok());
                    info.gpus.push(Gpu {
                        vendor: "AMD".into(),
                        name: name.into(),
                        vram_bytes: vram,
                        ..Default::default()
                    });
                }
            }
        }
    } else if std::path::Path::new("/opt/rocm").exists() {
        info.rocm = true;
    }
    if info.gpus.is_empty() {
        if let Some(out) = probe("lspci", &[], Duration::from_secs(5)) {
            for line in out.lines() {
                let l = line.to_ascii_lowercase();
                if l.contains("vga") || l.contains("3d controller") || l.contains("display controller") {
                    let vendor = if l.contains("nvidia") {
                        "NVIDIA"
                    } else if l.contains("amd") || l.contains("ati") {
                        "AMD"
                    } else if l.contains("intel") {
                        "Intel"
                    } else {
                        "unknown"
                    };
                    let name = line.split_once(": ").map(|x| x.1).unwrap_or(line).to_string();
                    info.gpus.push(Gpu {
                        vendor: vendor.into(),
                        name,
                        ..Default::default()
                    });
                }
            }
        }
    }
}

fn detect_windows(info: &mut HardwareInfo) {
    info.vulkan = std::path::Path::new(r"C:\Windows\System32\vulkan-1.dll").exists();
    let ps = "Get-CimInstance Win32_VideoController | Select-Object Name,AdapterCompatibility,AdapterRAM,DriverVersion | ConvertTo-Json -Compress";
    if let Some(out) = probe("powershell", &["-NoProfile", "-NonInteractive", "-Command", ps], Duration::from_secs(10)) {
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap_or_default();
        let items = match v {
            serde_json::Value::Array(a) => a,
            o @ serde_json::Value::Object(_) => vec![o],
            _ => vec![],
        };
        for g in items {
            let name = g.get("Name").and_then(|x| x.as_str()).unwrap_or("GPU").to_string();
            if name.contains("Basic Display") || name.contains("Remote") {
                continue;
            }
            // AdapterRAM is a 32-bit field and wraps above 4 GB; trust it only
            // when nvidia-smi is unavailable and treat it as a lower bound.
            let vram = g.get("AdapterRAM").and_then(|x| x.as_u64()).filter(|v| *v > 0);
            info.gpus.push(Gpu {
                vendor: g.get("AdapterCompatibility").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                name,
                vram_bytes: vram,
                driver: g.get("DriverVersion").and_then(|x| x.as_str()).map(str::to_string),
                ..Default::default()
            });
        }
    }
}

fn detect_nvidia(info: &mut HardwareInfo) {
    let Some(out) = probe(
        "nvidia-smi",
        &["--query-gpu=name,memory.total,memory.free,driver_version", "--format=csv,noheader,nounits"],
        Duration::from_secs(8),
    ) else {
        return;
    };
    let mut found = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if parts.len() >= 4 {
            found.push(Gpu {
                vendor: "NVIDIA".into(),
                name: parts[0].to_string(),
                vram_bytes: parts[1].parse::<u64>().ok().map(|mib| mib * 1024 * 1024),
                vram_free_bytes: parts[2].parse::<u64>().ok().map(|mib| mib * 1024 * 1024),
                driver: Some(parts[3].to_string()),
            });
        }
    }
    if !found.is_empty() {
        info.cuda = true;
        info.gpus.retain(|g| !g.vendor.to_ascii_lowercase().contains("nvidia"));
        info.gpus.extend(found);
    }
}

/// Parse sizes like "8 GB", "1536 MB".
fn parse_size(s: &str) -> Option<u64> {
    let mut parts = s.split_whitespace();
    let n: f64 = parts.next()?.parse().ok()?;
    let mult = match parts.next()?.to_ascii_uppercase().as_str() {
        "GB" => 1u64 << 30,
        "MB" => 1 << 20,
        "KB" => 1 << 10,
        _ => return None,
    };
    Some((n * mult as f64) as u64)
}

pub fn format_bytes(n: u64) -> String {
    let gb = n as f64 / (1u64 << 30) as f64;
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", n as f64 / (1u64 << 20) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_something_on_this_machine() {
        let h = HardwareInfo::detect(&std::env::temp_dir());
        assert!(!h.os.is_empty());
        assert!(h.total_ram > 0);
        assert!(h.cpu_cores > 0);
        assert!(h.fast_memory_budget() > 0);
    }

    #[test]
    fn budgets() {
        let mac = HardwareInfo {
            unified_memory: true,
            total_ram: 16 << 30,
            available_ram: 8 << 30,
            ..Default::default()
        };
        assert_eq!(mac.fast_memory_budget(), ((16u64 << 30) as f64 * 0.70) as u64);
        let pc = HardwareInfo {
            total_ram: 32 << 30,
            available_ram: 20 << 30,
            gpus: vec![Gpu {
                vendor: "NVIDIA".into(),
                vram_bytes: Some(24 << 30),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(pc.has_discrete_gpu());
        assert!(pc.fast_memory_budget() > 22 << 30);
        let cpu = HardwareInfo {
            total_ram: 8 << 30,
            available_ram: 6 << 30,
            ..Default::default()
        };
        assert!(cpu.fast_memory_budget() < 6 << 30);
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("8 GB"), Some(8 << 30));
        assert_eq!(parse_size("1536 MB"), Some(1536 << 20));
        assert_eq!(format_bytes(20419565568), "19.0 GB");
    }
}
