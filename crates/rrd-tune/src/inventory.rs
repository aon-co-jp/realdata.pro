//! 計算資源の一覧(CPU・GPU・NPU)。
//!
//! - CPU: open-cpu の実行時判定(AVX-512 / AVX2 / FMA など)と論理コア数。
//! - GPU: open-cuda の Vulkan 実デバイス列挙。
//! - NPU: OS が認識している名前を**報告するだけ**。open-cuda / open-directx / aruaru-llm のいずれにも
//!   NPU へ計算を渡す実行経路は無いため、`usable = false` とし、割り当ての対象にしない。

use std::sync::Arc;

use opencuda_core::GpuDevice;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CpuInfo {
    pub name: String,
    pub arch: String,
    pub logical_cores: usize,
    /// 実行時に使える命令セット(例: "AVX-512F" "AVX2" "FMA")
    pub features: Vec<String>,
    /// この CPU で使える最速の SIMD 経路
    pub best_simd: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GpuInfo {
    pub name: String,
    pub api: String,
    pub memory_mb: u64,
    /// 実際に計算を実行できるか(できない場合は理由を `note` に)
    pub usable: bool,
    pub note: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct NpuInfo {
    pub name: String,
    pub source: String,
    pub usable: bool,
    pub note: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Inventory {
    pub cpu: CpuInfo,
    pub gpus: Vec<GpuInfo>,
    pub npus: Vec<NpuInfo>,
}

impl CpuInfo {
    /// 行列積などで使われる SIMD の短い名前。
    pub fn isa_short(&self) -> &'static str {
        if self.features.iter().any(|f| f == "AVX-512F") {
            "AVX-512"
        } else if self.features.iter().any(|f| f == "AVX2") {
            "AVX2"
        } else {
            "SIMD なし"
        }
    }
}

impl Inventory {
    /// 構成が変わったか(CPU 名・GPU 名・NPU 名)を見分けるための短い文字列。
    pub fn fingerprint(&self) -> String {
        // デバッグビルドの実測は CPU 側が最適化されず実際より遅く出るため、ビルドの種類も構成の一部とみなす
        // (本番の release で動かしたとき、必ず検査し直される)
        let build = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        let mut parts = vec![
            format!("build:{build}"),
            format!("cpu:{}:{}", self.cpu.name, self.cpu.best_simd),
        ];
        parts.extend(
            self.gpus
                .iter()
                .map(|g| format!("gpu:{}:{}", g.api, g.name)),
        );
        parts.extend(self.npus.iter().map(|n| format!("npu:{}", n.name)));
        parts.join("|")
    }
}

#[cfg(target_os = "windows")]
fn cpu_name() -> String {
    std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default()
}

#[cfg(not(target_os = "windows"))]
fn cpu_name() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_default()
}

pub fn detect_cpu() -> CpuInfo {
    let caps = open_cpu::detect();
    let mut features = Vec::new();
    for (on, name) in [
        (caps.avx512f, "AVX-512F"),
        (caps.avx512bw, "AVX-512BW"),
        (caps.avx512vl, "AVX-512VL"),
        (caps.avx2, "AVX2"),
        (caps.fma, "FMA"),
        (caps.bmi2, "BMI2"),
        (caps.popcnt, "POPCNT"),
    ] {
        if on {
            features.push(name.to_string());
        }
    }
    CpuInfo {
        name: cpu_name(),
        arch: std::env::consts::ARCH.to_string(),
        logical_cores: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        features,
        best_simd: rrd_render::Simd::detect().label().to_string(),
    }
}

/// 使える Vulkan GPU デバイス(計測に使うもの)と、一覧用の情報を返す。
///
/// open-cuda の `enumerate_real(n)` は「n 番目の Vulkan デバイス 1 台」を返す(範囲外はエラー)。
/// そのため 0 番から順に、エラーになるまで開く。
pub fn detect_gpus() -> (Vec<GpuInfo>, Vec<Arc<dyn GpuDevice>>) {
    let mut infos = Vec::new();
    let mut usable: Vec<Arc<dyn GpuDevice>> = Vec::new();
    for index in 0..8 {
        match opencuda_vulkan::real::enumerate_real(index) {
            Ok(devs) => {
                for d in devs {
                    let i = d.info();
                    let ok = d.supports_spirv();
                    infos.push(GpuInfo {
                        name: i.name.clone(),
                        api: "Vulkan".into(),
                        memory_mb: i.total_memory / (1024 * 1024),
                        usable: ok,
                        note: if ok {
                            String::new()
                        } else {
                            "SPIR-V のコンピュートを実行できません".into()
                        },
                    });
                    if ok {
                        usable.push(d);
                    }
                }
            }
            Err(e) => {
                // 0 番で失敗したら、Vulkan ローダーが無い・GPU が無い環境。CPU だけで動く。
                if index == 0 {
                    infos.push(GpuInfo {
                        name: "(Vulkan デバイスなし)".into(),
                        api: "Vulkan".into(),
                        memory_mb: 0,
                        usable: false,
                        note: format!("{e:#}").chars().take(200).collect(),
                    });
                }
                break;
            }
        }
    }
    (infos, usable)
}
/// OS が認識している NPU の名前(見つかった分だけ)。
pub fn detect_npus() -> Vec<NpuInfo> {
    let mut names: Vec<(String, &'static str)> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let out = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName -match 'NPU|Neural|AI Boost|Hexagon|Ryzen AI|VPU' } | Select-Object -ExpandProperty FriendlyName",
            ])
            .output();
        if let Ok(o) = out {
            for l in String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
            {
                names.push((l.to_string(), "Windows デバイス一覧"));
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(rd) = std::fs::read_dir("/sys/class/accel") {
            for e in rd.flatten() {
                names.push((
                    format!("accel/{}", e.file_name().to_string_lossy()),
                    "/sys/class/accel",
                ));
            }
        }
    }
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|(name, source)| NpuInfo {
            name,
            source: source.into(),
            usable: false,
            note: "検出のみ。NPU へ計算を渡す実行経路が open-cuda / open-directx / aruaru-llm に無いため、割り当てません".into(),
        })
        .collect()
}

pub fn detect() -> (Inventory, Vec<Arc<dyn GpuDevice>>) {
    let (gpus, devices) = detect_gpus();
    (
        Inventory {
            cpu: detect_cpu(),
            gpus,
            npus: detect_npus(),
        },
        devices,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_info_is_filled_and_fingerprint_changes_with_hardware() {
        let c = detect_cpu();
        assert!(c.logical_cores >= 1 && !c.best_simd.is_empty());
        let mut inv = Inventory {
            cpu: c,
            ..Default::default()
        };
        let a = inv.fingerprint();
        inv.gpus.push(GpuInfo {
            name: "X".into(),
            api: "Vulkan".into(),
            ..Default::default()
        });
        assert_ne!(a, inv.fingerprint());
    }
}
