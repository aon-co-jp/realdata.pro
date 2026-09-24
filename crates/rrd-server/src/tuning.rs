//! 自動性能検査の状態と、GraphQL で見せる形(`rrd-tune` を使う)。
//!
//! - 起動時に構成(CPU・GPU・NPU)を調べ、前回の検査結果と構成が同じで、7日以内なら再利用する。
//!   構成が変わった(GPU を付け替えた等)・結果が無い・7日以上前なら、裏で自動的に再検査する。
//! - 処理の実行時に、処理の大きさから経路(CPU / GPU)を選び、使用実績(回数・時間)を数える。
//! - 「割り当ての割合」は検査結果から計算した**推奨**、「使用実績」は実際に選ばれた**結果**として別々に出す。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_graphql::SimpleObject;
use opencuda_core::GpuDevice;
use rrd_tune::{Profile, Workload};

/// 検査結果を再利用する最長の期間。
const MAX_AGE_SECS: u64 = 7 * 24 * 3600;

#[derive(Default)]
pub struct TuneState {
    pub profile: RwLock<Option<Profile>>,
    /// `rrd_tune` が返した順の、Vulkan で計算できる GPU
    pub gpus: RwLock<Vec<Arc<dyn GpuDevice>>>,
    /// "gemm/cpu" "raster/gpu0" などごとの (回数, 合計ミリ秒)
    pub usage: Mutex<BTreeMap<String, (u64, f64)>>,
    pub running: AtomicBool,
}

impl TuneState {
    pub fn record(&self, key: &str, ms: f64) {
        if let Ok(mut u) = self.usage.lock() {
            let e = u.entry(key.to_string()).or_insert((0, 0.0));
            e.0 += 1;
            e.1 += ms;
        }
    }
}

fn profile_path(data_dir: &Path) -> PathBuf {
    data_dir.join("tune_profile.json")
}

pub fn load_profile(data_dir: &Path) -> Option<Profile> {
    serde_json::from_slice(&std::fs::read(profile_path(data_dir)).ok()?).ok()
}

fn save_profile(data_dir: &Path, p: &Profile) {
    let write = || -> anyhow::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        let tmp = data_dir.join("tune_profile.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(p)?)?;
        std::fs::rename(&tmp, profile_path(data_dir))?;
        Ok(())
    };
    if let Err(e) = write() {
        eprintln!("realdata.pro: 性能検査の結果を保存できません: {e:#}");
    }
}

/// 構成を調べ、必要なら性能検査を(裏で)実行する。`force` なら結果があっても再検査する。
pub async fn ensure_profile(st: Arc<crate::schema::AppState>, force: bool) {
    if st.tune.running.swap(true, Ordering::SeqCst) {
        return;
    }
    let data_dir = st.data_dir.clone();
    let existing = st.tune.profile.read().ok().and_then(|p| p.clone());
    let joined = tokio::task::spawn_blocking(move || {
        let (inventory, gpus) = rrd_tune::inventory::detect();
        let fresh = existing.as_ref().is_some_and(|p| {
            p.fingerprint == inventory.fingerprint()
                && rrd_tune::now_unix().saturating_sub(p.created_unix) < MAX_AGE_SECS
        });
        if fresh && !force {
            (existing.expect("fresh なら存在する"), gpus, false)
        } else {
            let (p, g) = rrd_tune::benchmark_with(inventory, gpus);
            (p, g, true)
        }
    })
    .await;
    match joined {
        Ok((profile, gpus, benchmarked)) => {
            if benchmarked {
                save_profile(&data_dir, &profile);
                eprintln!(
                    "realdata.pro: 性能検査を実行しました({:.1}秒、決定 {} 件)",
                    profile.elapsed_ms / 1000.0,
                    profile.decisions.len()
                );
            } else {
                eprintln!(
                    "realdata.pro: 前回の性能検査の結果を再利用します(構成は変わっていません)"
                );
            }
            if let Ok(mut g) = st.tune.gpus.write() {
                *g = gpus;
            }
            if let Ok(mut p) = st.tune.profile.write() {
                *p = Some(profile);
            }
        }
        Err(e) => eprintln!("realdata.pro: 性能検査が異常終了しました: {e}"),
    }
    st.tune.running.store(false, Ordering::SeqCst);
}

/// 経路の選択。実測の結果で決める。動作確認用に、環境変数 `RRD_COMPUTE=cpu|gpu` で固定できる
/// (`gpu` は GPU が使えるときだけ。未設定または `auto` なら実測に従う)。
pub fn route(st: &crate::schema::AppState, workload: Workload, work: f64) -> rrd_tune::Route {
    match std::env::var("RRD_COMPUTE")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "cpu" => rrd_tune::Route::Cpu,
        "gpu" if st.tune.gpus.read().is_ok_and(|g| !g.is_empty()) => rrd_tune::Route::Gpu(0),
        _ => {
            let p = st.tune.profile.read().ok();
            rrd_tune::choose(p.as_deref().and_then(|x| x.as_ref()), workload, work)
        }
    }
}
/// 起動後に検査を始めるか(初回・構成の変更・7日経過)を、定期実行から呼んで判定する。
pub fn is_stale(st: &crate::schema::AppState) -> bool {
    match st.tune.profile.read().ok().and_then(|p| p.clone()) {
        None => true,
        Some(p) => rrd_tune::now_unix().saturating_sub(p.created_unix) >= MAX_AGE_SECS,
    }
}

// ───────────── GraphQL で見せる形 ─────────────

#[derive(SimpleObject)]
pub struct CpuView {
    pub name: String,
    pub arch: String,
    pub logical_cores: usize,
    pub features: Vec<String>,
    pub best_simd: String,
}

#[derive(SimpleObject)]
pub struct DeviceView {
    /// "GPU" / "NPU"
    pub kind: String,
    pub name: String,
    pub detail: String,
    /// 実際に計算を割り当てられるか
    pub usable: bool,
    pub note: String,
}

#[derive(SimpleObject)]
pub struct BenchView {
    pub workload: String,
    pub size: String,
    pub backend: String,
    pub backend_label: String,
    pub ms: Option<f64>,
    pub ok: bool,
    pub candidate: bool,
    pub note: String,
}

#[derive(SimpleObject)]
pub struct DecisionView {
    pub workload: String,
    pub size: String,
    pub backend: String,
    pub backend_label: String,
    pub ms: f64,
    pub margin: Option<f64>,
}

#[derive(SimpleObject)]
pub struct ShareEntry {
    pub backend: String,
    pub backend_label: String,
    pub percent: f64,
}

#[derive(SimpleObject)]
pub struct ShareView {
    pub workload: String,
    pub by_runs: Vec<ShareEntry>,
    pub by_work: Vec<ShareEntry>,
}

#[derive(SimpleObject)]
pub struct UsageEntry {
    pub workload: String,
    pub backend: String,
    pub backend_label: String,
    pub calls: u64,
    pub total_ms: f64,
    /// 呼び出し回数に対する割合(%)
    pub percent: f64,
}

#[derive(SimpleObject)]
pub struct ProfileView {
    /// 検査を実行中か
    pub running: bool,
    /// 検査結果がまだ無いか(初回の検査中など)
    pub available: bool,
    pub created_unix: u64,
    pub elapsed_ms: f64,
    pub cpu: Option<CpuView>,
    pub devices: Vec<DeviceView>,
    pub results: Vec<BenchView>,
    pub decisions: Vec<DecisionView>,
    pub shares: Vec<ShareView>,
    pub usage: Vec<UsageEntry>,
    pub notes: Vec<String>,
}

/// "gpu0" → "GPU 0: <名前>"、"cpu" → "CPU(<最速の SIMD>)"。
fn backend_label(id: &str, p: &Profile) -> String {
    match id {
        "cpu" => format!(
            "CPU({}・{}スレッド)",
            p.inventory.cpu.isa_short(),
            p.inventory.cpu.logical_cores
        ),
        "cpu-scalar" => "CPU(SIMD なし・参考)".to_string(),
        g if g.starts_with("gpu") => {
            let idx: usize = g[3..].parse().unwrap_or(0);
            let usable: Vec<&_> = p.inventory.gpus.iter().filter(|x| x.usable).collect();
            format!(
                "GPU {idx}: {}",
                usable.get(idx).map_or("?", |x| x.name.as_str())
            )
        }
        other => other.to_string(),
    }
}

fn workload_label(w: Workload) -> String {
    w.label().to_string()
}

pub fn view(st: &crate::schema::AppState) -> ProfileView {
    let running = st.tune.running.load(Ordering::SeqCst);
    let usage_raw = st.tune.usage.lock().map(|u| u.clone()).unwrap_or_default();
    let Some(p) = st.tune.profile.read().ok().and_then(|p| p.clone()) else {
        return ProfileView {
            running,
            available: false,
            created_unix: 0,
            elapsed_ms: 0.0,
            cpu: None,
            devices: vec![],
            results: vec![],
            decisions: vec![],
            shares: vec![],
            usage: vec![],
            notes: vec![],
        };
    };
    let c = &p.inventory.cpu;
    let mut devices: Vec<DeviceView> = p
        .inventory
        .gpus
        .iter()
        .map(|g| DeviceView {
            kind: "GPU".into(),
            name: g.name.clone(),
            detail: format!("{} / {}MB", g.api, g.memory_mb),
            usable: g.usable,
            note: g.note.clone(),
        })
        .collect();
    devices.extend(p.inventory.npus.iter().map(|n| DeviceView {
        kind: "NPU".into(),
        name: n.name.clone(),
        detail: n.source.clone(),
        usable: n.usable,
        note: n.note.clone(),
    }));
    let entries = |m: &BTreeMap<String, f64>| -> Vec<ShareEntry> {
        let mut v: Vec<ShareEntry> = m
            .iter()
            .map(|(k, x)| ShareEntry {
                backend: k.clone(),
                backend_label: backend_label(k, &p),
                percent: (*x * 10.0).round() / 10.0,
            })
            .collect();
        v.sort_by(|a, b| b.percent.total_cmp(&a.percent));
        v
    };
    let mut usage: Vec<UsageEntry> = Vec::new();
    for w in [Workload::Gemm, Workload::Raster] {
        let prefix = match w {
            Workload::Gemm => "gemm/",
            Workload::Raster => "raster/",
        };
        let rows: Vec<(&String, &(u64, f64))> = usage_raw
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .collect();
        let total: u64 = rows.iter().map(|(_, v)| v.0).sum();
        for (k, (calls, ms)) in rows {
            let id = &k[prefix.len()..];
            usage.push(UsageEntry {
                workload: workload_label(w),
                backend: id.to_string(),
                backend_label: backend_label(id, &p),
                calls: *calls,
                total_ms: (*ms * 10.0).round() / 10.0,
                percent: if total > 0 {
                    (*calls as f64 * 1000.0 / total as f64).round() / 10.0
                } else {
                    0.0
                },
            });
        }
    }
    ProfileView {
        running,
        available: true,
        created_unix: p.created_unix,
        elapsed_ms: p.elapsed_ms,
        cpu: Some(CpuView {
            name: c.name.clone(),
            arch: c.arch.clone(),
            logical_cores: c.logical_cores,
            features: c.features.clone(),
            best_simd: c.best_simd.clone(),
        }),
        devices,
        results: p
            .results
            .iter()
            .map(|r| BenchView {
                workload: workload_label(r.workload),
                size: r.size.clone(),
                backend: r.backend.clone(),
                backend_label: backend_label(&r.backend, &p),
                ms: r.ms.map(|m| (m * 100.0).round() / 100.0),
                ok: r.ok,
                candidate: r.candidate,
                note: r.note.clone(),
            })
            .collect(),
        decisions: p
            .decisions
            .iter()
            .map(|d| DecisionView {
                workload: workload_label(d.workload),
                size: d.size.clone(),
                backend: d.backend.clone(),
                backend_label: backend_label(&d.backend, &p),
                ms: (d.ms * 100.0).round() / 100.0,
                margin: d.margin.map(|m| (m * 100.0).round() / 100.0),
            })
            .collect(),
        shares: p
            .shares
            .iter()
            .map(|(w, s)| ShareView {
                workload: workload_label(*w),
                by_runs: entries(&s.by_runs),
                by_work: entries(&s.by_work),
            })
            .collect(),
        usage,
        notes: p.notes.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_accumulates_and_view_without_profile_is_unavailable() {
        let st = crate::schema::AppState::new(
            rrd_compute::default_device(),
            "http://127.0.0.1:1".into(),
            std::env::temp_dir().join("rrd-tuning-test"),
        );
        st.tune.record("gemm/cpu", 2.0);
        st.tune.record("gemm/cpu", 4.0);
        assert_eq!(st.tune.usage.lock().unwrap()["gemm/cpu"], (2, 6.0));
        let v = view(&st);
        assert!(!v.available && v.decisions.is_empty());
    }
}
