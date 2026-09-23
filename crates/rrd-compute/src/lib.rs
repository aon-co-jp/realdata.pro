//! rs-real-data 計算層。
//!
//! 行列演算は open-cuda の `opencuda_blas::sgemm` に任せる。デバイスは
//! `opencuda_core::GpuDevice` として受け取るため、CPUバックエンド
//! (`opencuda_cpu::CpuDevice`)でも、open-cuda が対応するGPUバックエンドでも
//! 同じコードで動く。GEMMはf32で実行し、連立方程式の求解はf64で行う。

use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
pub use opencuda_core::GpuDevice;
use rrd_core::DataFrame;

/// 既定の計算デバイス(open-cuda の CPU バックエンド)。
pub fn default_device() -> Arc<dyn GpuDevice> {
    opencuda_cpu::CpuDevice::new(0)
}

/// 重回帰の結果。
#[derive(Debug, Clone, PartialEq)]
pub struct OlsFit {
    pub target: String,
    pub features: Vec<String>,
    pub intercept: f64,
    /// features と同じ順の係数。
    pub coefficients: Vec<f64>,
    pub r_squared: f64,
    /// 学習に使った行数(欠損を含む行は除外)。
    pub n: usize,
}

impl OlsFit {
    /// 特徴量の値から予測する。
    pub fn predict(&self, x: &[f64]) -> f64 {
        self.intercept
            + self
                .coefficients
                .iter()
                .zip(x)
                .map(|(b, v)| b * v)
                .sum::<f64>()
    }
}

/// 行優先の行列 A (m×k) に対して AᵀA (k×k) を open-cuda の sgemm で計算する。
fn gram(device: &dyn GpuDevice, a: &[f32], m: usize, k: usize) -> Result<Vec<f32>> {
    // Aᵀ を明示的に作って sgemm(k×m · m×k)に渡す。
    let mut at = vec![0f32; k * m];
    for i in 0..m {
        for j in 0..k {
            at[j * m + i] = a[i * k + j];
        }
    }
    let mut c = vec![0f32; k * k];
    opencuda_blas::sgemm(device, k, m, k, 1.0, &at, a, 0.0, &mut c, None)
        .context("opencuda-blas sgemm (AᵀA) に失敗")?;
    Ok(c)
}

/// 対称正定値行列の連立方程式を部分ピボット付きガウス消去で解く(f64)。
fn solve(mut a: Vec<f64>, mut b: Vec<f64>, n: usize) -> Result<Vec<f64>> {
    for col in 0..n {
        let piv = (col..n)
            .max_by(|&x, &y| a[x * n + col].abs().total_cmp(&a[y * n + col].abs()))
            .unwrap();
        if a[piv * n + col].abs() < 1e-9 {
            bail!("特徴量が線形従属(多重共線性)のため解けません");
        }
        if piv != col {
            for j in 0..n {
                a.swap(col * n + j, piv * n + j);
            }
            b.swap(col, piv);
        }
        for r in col + 1..n {
            let f = a[r * n + col] / a[col * n + col];
            for j in col..n {
                a[r * n + j] -= f * a[col * n + j];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|j| a[r * n + j] * x[j]).sum();
        x[r] = (b[r] - s) / a[r * n + r];
    }
    Ok(x)
}

/// y ~ 切片 + Σ bᵢ xᵢ の最小二乗(正規方程式)。欠損を含む行は除外する。
pub fn ols(
    device: &dyn GpuDevice,
    df: &DataFrame,
    target: &str,
    features: &[&str],
) -> Result<OlsFit> {
    if features.is_empty() {
        bail!("説明変数を1つ以上指定してください");
    }
    let ycol = df.column(target).map_err(|e| anyhow!("{e}"))?;
    let xcols = features
        .iter()
        .map(|f| df.column(f).map_err(|e| anyhow!("{e}")))
        .collect::<Result<Vec<_>>>()?;
    for c in xcols.iter().chain([&ycol]) {
        if !c.is_numeric() {
            bail!("{}は数値列ではありません", c.name);
        }
    }

    // 各特徴量を平均で中心化してから切片列を加える(f32 GEMMの桁落ちを抑える)。
    let rows: Vec<usize> = (0..df.height())
        .filter(|&i| !ycol.is_null(i) && xcols.iter().all(|c| !c.is_null(i)))
        .collect();
    let m = rows.len();
    let p = features.len();
    if m <= p + 1 {
        bail!("有効な行数({m})が説明変数の数に対して少なすぎます");
    }
    let val = |c: &rrd_core::Column, i: usize| c.get(i).as_f64().unwrap();
    let means: Vec<f64> = xcols
        .iter()
        .map(|c| rows.iter().map(|&i| val(c, i)).sum::<f64>() / m as f64)
        .collect();
    let ymean = rows.iter().map(|&i| val(ycol, i)).sum::<f64>() / m as f64;

    // 拡大行列 [x₁..x_p, y](中心化済み)を作り、その Gram 行列から XᵀX と Xᵀy を取り出す。
    let k = p + 1;
    let mut a = vec![0f32; m * k];
    for (r, &i) in rows.iter().enumerate() {
        for (j, c) in xcols.iter().enumerate() {
            a[r * k + j] = (val(c, i) - means[j]) as f32;
        }
        a[r * k + p] = (val(ycol, i) - ymean) as f32;
    }
    let g = gram(device, &a, m, k)?;
    let xtx: Vec<f64> = (0..p)
        .flat_map(|r| (0..p).map(move |c| (r, c)))
        .map(|(r, c)| g[r * k + c] as f64)
        .collect();
    let xty: Vec<f64> = (0..p).map(|r| g[r * k + p] as f64).collect();
    let coefficients = solve(xtx, xty, p)?;
    let intercept = ymean
        - coefficients
            .iter()
            .zip(&means)
            .map(|(b, m)| b * m)
            .sum::<f64>();

    // 決定係数は f64 で元データから計算する。
    let mut ss_res = 0.0;
    let mut ss_tot = 0.0;
    for &i in &rows {
        let x: Vec<f64> = xcols.iter().map(|c| val(c, i)).collect();
        let pred = intercept + coefficients.iter().zip(&x).map(|(b, v)| b * v).sum::<f64>();
        let y = val(ycol, i);
        ss_res += (y - pred).powi(2);
        ss_tot += (y - ymean).powi(2);
    }
    let r_squared = if ss_tot == 0.0 {
        1.0
    } else {
        1.0 - ss_res / ss_tot
    };

    Ok(OlsFit {
        target: target.to_string(),
        features: features.iter().map(|s| s.to_string()).collect(),
        intercept,
        coefficients,
        r_squared,
        n: m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rrd_core::csv::read_csv_str;

    #[test]
    fn recovers_exact_plane() {
        // y = 3 + 2a - 0.5b(ノイズなし)
        let mut s = String::from("a,b,y\n");
        for i in 0..20 {
            let a = i as f64;
            let b = ((i * 7) % 11) as f64;
            s.push_str(&format!("{a},{b},{}\n", 3.0 + 2.0 * a - 0.5 * b));
        }
        let df = read_csv_str(&s).unwrap();
        let fit = ols(&*default_device(), &df, "y", &["a", "b"]).unwrap();
        assert!((fit.intercept - 3.0).abs() < 1e-3, "{fit:?}");
        assert!((fit.coefficients[0] - 2.0).abs() < 1e-4);
        assert!((fit.coefficients[1] + 0.5).abs() < 1e-4);
        assert!(fit.r_squared > 0.999_999);
        assert!((fit.predict(&[1.0, 2.0]) - 4.0).abs() < 1e-3);
    }

    #[test]
    fn skips_nulls_and_rejects_collinear() {
        let df = read_csv_str("a,b,y\n1,2,3\n2,4,5\n3,6,7\n4,8,\n5,10,11\n").unwrap();
        assert!(ols(&*default_device(), &df, "y", &["a", "b"]).is_err());
        let fit = ols(&*default_device(), &df, "y", &["a"]).unwrap();
        assert_eq!(fit.n, 4);
        assert!((fit.coefficients[0] - 2.0).abs() < 1e-4);
    }
}
