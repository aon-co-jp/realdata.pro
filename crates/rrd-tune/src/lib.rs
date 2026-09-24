//! realdata.pro の自動性能検査と、実測に基づく経路選択。
//!
//! 1. **一覧**: CPU(命令セット)・GPU・NPU を列挙する(`inventory`)。
//! 2. **実測**: 処理の種類(行列積・グラフ描画)ごとに、大きさを変えて各経路の時間を測る。
//!    GPU の結果は CPU の結果と突き合わせ、一致しなければ(速くても)採用しない。
//! 3. **決定**: 大きさごとに、正しく動いた経路のうち最速のものを選ぶ(`Decision`)。
//! 4. **割り当て割合**: 決定から「どの資源にどれだけ割り当てるか」を計算する(`Shares`)。
//! 5. **実行時の選択**: `Router` が、処理の大きさから経路を返す。プロファイルが無ければ CPU。
//!
//! 「AI による検査」は、実測値の解釈・異常の指摘・改善の提案に使う(aruaru-llm、`explain_prompt`)。
//! 経路の**選択そのものは実測の数値だけで決める**(AI の回答で変えない)。
//!
//! 動かせない資源は割り当てない: NPU は検出だけを報告し、GPU は Vulkan で実際に計算できたものだけを使う。

pub mod inventory;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use opencuda_core::GpuDevice;
use serde::{Deserialize, Serialize};

pub use inventory::{Inventory, NpuInfo};

/// GEMM 用シェーダー(open-cuda の `matmul_bench` と同じ、1スレッド1出力要素の素朴な実装)。
pub const SGEMM_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sgemm_naive.spv"));

/// 1つの経路の時間がこれを超えたら、それ以上大きい形状は試さない(遅い GPU で検査が終わらなくなるのを防ぐ)。
const SLOW_LIMIT_MS: f64 = 4000.0;
const REPS: usize = 3;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Workload {
    /// 行列積(重回帰などの計算)。work = m×k×n
    Gemm,
    /// グラフ描画。work = 画素数
    Raster,
}

impl Workload {
    pub fn label(self) -> &'static str {
        match self {
            Workload::Gemm => "行列積(重回帰など)",
            Workload::Raster => "グラフ描画",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BenchResult {
    pub workload: Workload,
    /// 例: "8×20000×8" "1280×720"
    pub size: String,
    pub work: f64,
    /// "cpu" / "gpu0" / 参考: "cpu-scalar"
    pub backend: String,
    /// 中央値(ミリ秒)。実行できなかった・遅すぎた場合は None
    pub ms: Option<f64>,
    /// CPU の結果と一致したか(CPU 自身は常に true)
    pub ok: bool,
    /// 経路の候補か(参考値の cpu-scalar は false)
    pub candidate: bool,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub workload: Workload,
    pub size: String,
    pub work: f64,
    pub backend: String,
    pub ms: f64,
    /// 2 番目に速い経路との比(1.0 なら差なし。候補が1つだけなら None)
    pub margin: Option<f64>,
}

/// 資源ごとの割り当て割合(%)。
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Shares {
    /// 大きさの区分を同じ重みで数えた割合(小さい処理が多い実際の使い方に近い)
    pub by_runs: BTreeMap<String, f64>,
    /// 処理量(work)で重み付けした割合(大きい処理を重く見る)
    pub by_work: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub fingerprint: String,
    pub created_unix: u64,
    pub inventory: Inventory,
    pub results: Vec<BenchResult>,
    pub decisions: Vec<Decision>,
    /// 処理の種類ごとの割り当て割合
    pub shares: BTreeMap<Workload, Shares>,
    pub elapsed_ms: f64,
    pub notes: Vec<String>,
}

fn lcg(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        })
        .collect()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// 最大絶対誤差が、参照の最大絶対値に対して十分小さいか。
fn close(a: &[f32], reference: &[f32]) -> bool {
    if a.len() != reference.len() {
        return false;
    }
    let scale = reference.iter().fold(1.0_f32, |m, x| m.max(x.abs()));
    a.iter()
        .zip(reference)
        .all(|(x, y)| (x - y).abs() <= 2e-3 * scale)
}

fn time_ms(mut f: impl FnMut() -> bool) -> Option<(f64, bool)> {
    let mut times = Vec::new();
    let mut all_ok = true;
    for i in 0..=REPS {
        let t = Instant::now();
        let ok = f();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        if i == 0 {
            // 1回目は初期化を含むため捨てる。ただし遅すぎる経路は繰り返さない。
            if ms > SLOW_LIMIT_MS {
                return Some((ms, ok));
            }
            all_ok &= ok;
            continue;
        }
        all_ok &= ok;
        times.push(ms);
    }
    Some((median(times), all_ok))
}

/// GEMM の実測。`gpus` は Vulkan で実際に計算できるデバイス。
fn bench_gemm(cpu: &Arc<dyn GpuDevice>, gpus: &[Arc<dyn GpuDevice>], out: &mut Vec<BenchResult>) {
    // 重回帰の正規方程式(k×m · m×k、k は説明変数の数+1)に近い細長い形状と、GPU に有利な正方形
    let shapes: [(usize, usize, usize); 6] = [
        (8, 1_000, 8),
        (8, 20_000, 8),
        (16, 100_000, 16),
        (128, 128, 128),
        (512, 512, 512),
        (1024, 1024, 1024),
    ];
    let mut too_slow: Vec<String> = Vec::new();
    for (m, k, n) in shapes {
        let (a, b) = (lcg(m * k, 1), lcg(k * n, 2));
        let work = (m * k * n) as f64;
        let size = format!("{m}×{k}×{n}");
        let mut reference = vec![0f32; m * n];
        let cpu_res = time_ms(|| {
            opencuda_blas::sgemm(&**cpu, m, k, n, 1.0, &a, &b, 0.0, &mut reference, None).is_ok()
        });
        out.push(BenchResult {
            workload: Workload::Gemm,
            size: size.clone(),
            work,
            backend: "cpu".into(),
            ms: cpu_res.map(|r| r.0),
            ok: cpu_res.is_some_and(|r| r.1),
            candidate: true,
            note: String::new(),
        });
        for (i, gpu) in gpus.iter().enumerate() {
            let id = format!("gpu{i}");
            if too_slow.contains(&id) {
                out.push(BenchResult {
                    workload: Workload::Gemm,
                    size: size.clone(),
                    work,
                    backend: id,
                    ms: None,
                    ok: false,
                    candidate: true,
                    note: "小さい形状で既に遅すぎたため省略".into(),
                });
                continue;
            }
            let mut c = vec![0f32; m * n];
            let res = time_ms(|| {
                opencuda_blas::sgemm(&**gpu, m, k, n, 1.0, &a, &b, 0.0, &mut c, Some(SGEMM_SPIRV))
                    .is_ok()
            });
            let (ms, ran_ok) = res.unwrap_or((f64::NAN, false));
            let matches = ran_ok && close(&c, &reference);
            let mut note = String::new();
            if !ran_ok {
                note = "実行に失敗".into();
            } else if !matches {
                note = "CPU の結果と一致しないため採用しません".into();
            }
            if ms > SLOW_LIMIT_MS {
                too_slow.push(id.clone());
                note = format!("{:.0} ms かかったため、これより大きい形状は省略します", ms);
            }
            out.push(BenchResult {
                workload: Workload::Gemm,
                size: size.clone(),
                work,
                backend: id,
                ms: ms.is_finite().then_some(ms),
                ok: matches,
                candidate: true,
                note,
            });
        }
    }
}

/// 描画の実測(円グラフ)。
fn bench_raster(gpu_available: bool, out: &mut Vec<BenchResult>) {
    let sizes: [(u32, u32); 3] = [(320, 180), (1280, 720), (2048, 1152)];
    let values: Vec<f64> = (1..=6).map(f64::from).collect();
    let simd = rrd_render::Simd::detect();
    let mut gpu_too_slow = false;
    for (w, h) in sizes {
        let Some(mesh) = rrd_render::pie_mesh(&values, w, h) else {
            continue;
        };
        let work = f64::from(w) * f64::from(h);
        let size = format!("{w}×{h}");
        let reference = rrd_render::render_cpu(&mesh, w, h, rrd_render::Simd::Scalar);
        // 参考: スカラー(SIMD の効果を見せるためだけ。候補にはしない)
        if simd != rrd_render::Simd::Scalar {
            let r = time_ms(|| {
                !rrd_render::render_cpu(&mesh, w, h, rrd_render::Simd::Scalar).is_empty()
            });
            out.push(BenchResult {
                workload: Workload::Raster,
                size: size.clone(),
                work,
                backend: "cpu-scalar".into(),
                ms: r.map(|x| x.0),
                ok: true,
                candidate: false,
                note: "参考(SIMD なし)".into(),
            });
        }
        let r = time_ms(|| rrd_render::render_cpu(&mesh, w, h, simd) == reference);
        out.push(BenchResult {
            workload: Workload::Raster,
            size: size.clone(),
            work,
            backend: "cpu".into(),
            ms: r.map(|x| x.0),
            ok: r.is_some_and(|x| x.1),
            candidate: true,
            note: simd.label().into(),
        });
        if gpu_available && gpu_too_slow {
            out.push(BenchResult {
                workload: Workload::Raster,
                size: size.clone(),
                work,
                backend: "gpu0".into(),
                ms: None,
                ok: false,
                candidate: true,
                note: "小さい大きさで既に遅すぎたため省略".into(),
            });
        } else if gpu_available {
            let mut same = true;
            let r = time_ms(|| match rrd_render::render_gpu(&mesh, w, h) {
                Ok((rgba, _)) => {
                    let differ = rgba
                        .chunks(4)
                        .zip(reference.chunks(4))
                        .filter(|(a, b)| a != b)
                        .count();
                    same &= (differ as f64) / (rgba.len() as f64 / 4.0) < 0.01;
                    true
                }
                Err(_) => false,
            });
            let (ms, ran) = r.unwrap_or((f64::NAN, false));
            gpu_too_slow = ms > SLOW_LIMIT_MS;
            let note = if !ran {
                "実行に失敗"
            } else if !same {
                "CPU の画像と大きく異なるため採用しません"
            } else {
                ""
            };
            out.push(BenchResult {
                workload: Workload::Raster,
                size,
                work,
                backend: "gpu0".into(),
                ms: ms.is_finite().then_some(ms),
                ok: ran && same,
                candidate: true,
                note: note.into(),
            });
        }
    }
}

/// 実測結果から、大きさごとの最速経路(正しく動いたもの)を決める。
pub fn decide(results: &[BenchResult]) -> Vec<Decision> {
    let mut by_case: BTreeMap<(Workload, u64, String), Vec<&BenchResult>> = BTreeMap::new();
    for r in results
        .iter()
        .filter(|r| r.candidate && r.ok && r.ms.is_some())
    {
        by_case
            .entry((r.workload, r.work as u64, r.size.clone()))
            .or_default()
            .push(r);
    }
    let mut out = Vec::new();
    for ((workload, _, size), mut rs) in by_case {
        rs.sort_by(|a, b| a.ms.unwrap().total_cmp(&b.ms.unwrap()));
        let best = rs[0];
        out.push(Decision {
            workload,
            size,
            work: best.work,
            backend: best.backend.clone(),
            ms: best.ms.unwrap(),
            margin: rs
                .get(1)
                .map(|second| second.ms.unwrap() / best.ms.unwrap()),
        });
    }
    out.sort_by_key(|d| (d.workload, d.work.to_bits()));
    out
}

/// 決定から、資源ごとの割り当て割合(%)を計算する。
pub fn compute_shares(decisions: &[Decision]) -> BTreeMap<Workload, Shares> {
    let mut out: BTreeMap<Workload, Shares> = BTreeMap::new();
    for w in [Workload::Gemm, Workload::Raster] {
        let ds: Vec<&Decision> = decisions.iter().filter(|d| d.workload == w).collect();
        if ds.is_empty() {
            continue;
        }
        let mut s = Shares::default();
        let total_work: f64 = ds.iter().map(|d| d.work).sum();
        for d in &ds {
            *s.by_runs.entry(d.backend.clone()).or_insert(0.0) += 100.0 / ds.len() as f64;
            *s.by_work.entry(d.backend.clone()).or_insert(0.0) += 100.0 * d.work / total_work;
        }
        out.insert(w, s);
    }
    out
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 性能検査を実行する(数秒〜数十秒。呼び出し側はブロッキング用のスレッドで実行すること)。
pub fn run_benchmark() -> (Profile, Vec<Arc<dyn GpuDevice>>) {
    let (inventory, gpus) = inventory::detect();
    benchmark_with(inventory, gpus)
}

/// 構成の一覧(`inventory::detect` の結果)を受け取って検査する。
pub fn benchmark_with(
    inventory: Inventory,
    gpus: Vec<Arc<dyn GpuDevice>>,
) -> (Profile, Vec<Arc<dyn GpuDevice>>) {
    let start = Instant::now();
    let cpu: Arc<dyn GpuDevice> = opencuda_cpu::CpuDevice::new(0);
    let mut results = Vec::new();
    bench_gemm(&cpu, &gpus, &mut results);
    bench_raster(!gpus.is_empty(), &mut results);
    let decisions = decide(&results);
    let shares = compute_shares(&decisions);
    let mut notes = Vec::new();
    if gpus.is_empty() {
        notes.push("使える GPU(Vulkan)が見つからないため、CPU だけで検査しました".into());
    }
    if cfg!(debug_assertions) {
        notes.push("デバッグビルドで検査したため、CPU が実際より大幅に遅く出ています。本番(release)で検査し直してください".into());
    }
    if !inventory.npus.is_empty() {
        notes.push("NPU は検出のみで、割り当ての対象にはしていません".into());
    }
    let profile = Profile {
        fingerprint: inventory.fingerprint(),
        created_unix: now_unix(),
        inventory,
        results,
        decisions,
        shares,
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
        notes,
    };
    (profile, gpus)
}

/// 実行時の経路の選択。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Cpu,
    /// GPU の番号(`run_benchmark` が返したデバイス列の添字)
    Gpu(usize),
}

impl Route {
    pub fn id(&self) -> String {
        match self {
            Route::Cpu => "cpu".into(),
            Route::Gpu(i) => format!("gpu{i}"),
        }
    }

    fn parse(id: &str) -> Route {
        id.strip_prefix("gpu")
            .and_then(|n| n.parse().ok())
            .map_or(Route::Cpu, Route::Gpu)
    }
}

/// 処理の大きさから経路を返す。プロファイルが無い、または該当する実測が無ければ CPU。
pub fn choose(profile: Option<&Profile>, workload: Workload, work: f64) -> Route {
    let Some(p) = profile else { return Route::Cpu };
    let ds: Vec<&Decision> = p
        .decisions
        .iter()
        .filter(|d| d.workload == workload)
        .collect();
    // 対数の距離が最も近い実測点の決定を使う(その大きさの近くで実際に速かった経路)
    let target = work.max(1.0).ln();
    ds.iter()
        .min_by(|a, b| {
            (a.work.max(1.0).ln() - target)
                .abs()
                .total_cmp(&(b.work.max(1.0).ln() - target).abs())
        })
        .map_or(Route::Cpu, |d| Route::parse(&d.backend))
}

/// AI(aruaru-llm)に渡す、実測値の解釈用の依頼文。数値は入れるが、機微な情報は含まない。
pub fn explain_prompt(p: &Profile) -> String {
    let inv = &p.inventory;
    let mut s = String::from(
        "あなたはコンピュータ性能の専門家です。次の実測結果を、非専門家にもわかる日本語で解説してください。\n\
         構成: (1) 使える資源の要約 (2) 処理ごとに、どの資源が速かったかとその理由 (3) 気になる点(遅い・不一致・未使用の資源) \
         (4) 改善の提案(2〜3個)。書かれていない数値は作らないでください。\n\n",
    );
    s.push_str(&format!(
        "CPU: {} / {}スレッド / 命令: {} / 最速の描画経路: {}\n",
        inv.cpu.name,
        inv.cpu.logical_cores,
        inv.cpu.features.join(" "),
        inv.cpu.best_simd
    ));
    for g in &inv.gpus {
        s.push_str(&format!(
            "GPU: {} ({}, {}MB, 使用可={}) {}\n",
            g.name, g.api, g.memory_mb, g.usable, g.note
        ));
    }
    for n in &inv.npus {
        s.push_str(&format!(
            "NPU: {} (使用可={}) {}\n",
            n.name, n.usable, n.note
        ));
    }
    if inv.gpus.is_empty() && inv.npus.is_empty() {
        s.push_str("GPU / NPU: 検出なし\n");
    }
    s.push_str("\n実測(処理, 大きさ, 経路, 中央値ms, CPUと一致, 備考):\n");
    for r in &p.results {
        s.push_str(&format!(
            "- {}, {}, {}, {}, {}, {}\n",
            r.workload.label(),
            r.size,
            r.backend,
            r.ms.map_or("なし".to_string(), |m| format!("{m:.2}")),
            r.ok,
            r.note
        ));
    }
    s.push_str("\n決定(処理, 大きさ, 採用した経路):\n");
    for d in &p.decisions {
        s.push_str(&format!(
            "- {}, {}, {} ({:.2}ms)\n",
            d.workload.label(),
            d.size,
            d.backend,
            d.ms
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(
        w: Workload,
        size: &str,
        work: f64,
        backend: &str,
        ms: Option<f64>,
        ok: bool,
    ) -> BenchResult {
        BenchResult {
            workload: w,
            size: size.into(),
            work,
            backend: backend.into(),
            ms,
            ok,
            candidate: true,
            note: String::new(),
        }
    }

    #[test]
    fn decision_picks_fastest_correct_backend() {
        let results = vec![
            r(Workload::Gemm, "small", 1e3, "cpu", Some(1.0), true),
            r(Workload::Gemm, "small", 1e3, "gpu0", Some(50.0), true),
            r(Workload::Gemm, "big", 1e9, "cpu", Some(900.0), true),
            r(Workload::Gemm, "big", 1e9, "gpu0", Some(100.0), true),
            // 速くても結果が違えば採用しない
            r(Workload::Gemm, "wrong", 1e6, "cpu", Some(10.0), true),
            r(Workload::Gemm, "wrong", 1e6, "gpu0", Some(1.0), false),
        ];
        let d = decide(&results);
        let pick = |s: &str| d.iter().find(|x| x.size == s).unwrap().backend.clone();
        assert_eq!(
            (
                pick("small").as_str(),
                pick("big").as_str(),
                pick("wrong").as_str()
            ),
            ("cpu", "gpu0", "cpu")
        );
        assert_eq!(
            d.iter().find(|x| x.size == "big").unwrap().margin,
            Some(9.0)
        );
        assert_eq!(
            d.iter().find(|x| x.size == "wrong").unwrap().margin,
            None,
            "候補が1つなら比は無い"
        );
    }

    #[test]
    fn shares_sum_to_100_and_router_uses_nearest_measured_size() {
        let results = vec![
            r(Workload::Gemm, "s", 1e3, "cpu", Some(1.0), true),
            r(Workload::Gemm, "s", 1e3, "gpu0", Some(9.0), true),
            r(Workload::Gemm, "l", 1e9, "cpu", Some(900.0), true),
            r(Workload::Gemm, "l", 1e9, "gpu0", Some(100.0), true),
        ];
        let decisions = decide(&results);
        let shares = compute_shares(&decisions);
        let g = &shares[&Workload::Gemm];
        assert!((g.by_runs.values().sum::<f64>() - 100.0).abs() < 1e-9);
        assert!((g.by_work.values().sum::<f64>() - 100.0).abs() < 1e-9);
        assert_eq!(g.by_runs["cpu"], 50.0);
        assert!(g.by_work["gpu0"] > 99.0, "処理量で見ると大きい処理が GPU");
        let p = Profile {
            fingerprint: String::new(),
            created_unix: 0,
            inventory: Inventory::default(),
            results,
            decisions,
            shares,
            elapsed_ms: 0.0,
            notes: vec![],
        };
        assert_eq!(choose(Some(&p), Workload::Gemm, 5e3), Route::Cpu);
        assert_eq!(choose(Some(&p), Workload::Gemm, 3e8), Route::Gpu(0));
        assert_eq!(
            choose(Some(&p), Workload::Raster, 1e6),
            Route::Cpu,
            "実測が無い処理は CPU"
        );
        assert_eq!(choose(None, Workload::Gemm, 1e12), Route::Cpu);
    }

    #[test]
    fn close_compares_relative_to_scale() {
        assert!(close(&[100.0, 1.0], &[100.1, 1.0]));
        assert!(!close(&[100.0, 1.0], &[100.0, 3.0]));
        assert!(!close(&[1.0], &[1.0, 2.0]));
    }

    #[test]
    fn real_benchmark_runs_and_cpu_results_are_correct() {
        let (p, _gpus) = run_benchmark();
        assert!(!p.decisions.is_empty());
        assert!(
            p.results
                .iter()
                .filter(|x| x.backend == "cpu")
                .all(|x| x.ok && x.ms.is_some()),
            "CPU 経路は常に正しく動く"
        );
        for w in [Workload::Gemm, Workload::Raster] {
            assert!(p.shares[&w].by_runs.values().sum::<f64>() > 99.9);
        }
        for x in &p.results {
            eprintln!(
                "  {:?} {:>14} {:<10} {:>10} ok={} {}",
                x.workload,
                x.size,
                x.backend,
                x.ms.map_or("—".into(), |m| format!("{m:.2}ms")),
                x.ok,
                x.note
            );
        }
        eprintln!(
            "GPU: {:?}",
            p.inventory
                .gpus
                .iter()
                .map(|g| format!(
                    "{} {}MB usable={} [{}]",
                    g.name, g.memory_mb, g.usable, g.note
                ))
                .collect::<Vec<_>>()
        );
        eprintln!(
            "検査 {:.1} 秒 / 決定: {:?}",
            p.elapsed_ms / 1000.0,
            p.decisions
                .iter()
                .map(|d| format!("{} {} → {}", d.workload.label(), d.size, d.backend))
                .collect::<Vec<_>>()
        );
        assert!(explain_prompt(&p).contains("実測"));
    }
}
