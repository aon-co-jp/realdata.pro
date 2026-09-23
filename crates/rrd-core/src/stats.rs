//! 基本統計量。入力は欠損を除いた数値列。

/// 算術平均。空なら None。
pub fn mean(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    Some(v.iter().sum::<f64>() / v.len() as f64)
}

/// 標本標準偏差(n-1で割る)。要素数2未満なら None。
pub fn std_dev(v: &[f64]) -> Option<f64> {
    if v.len() < 2 {
        return None;
    }
    let m = mean(v)?;
    let ss: f64 = v.iter().map(|x| (x - m).powi(2)).sum();
    Some((ss / (v.len() - 1) as f64).sqrt())
}

pub fn min(v: &[f64]) -> Option<f64> {
    v.iter().copied().reduce(f64::min)
}

pub fn max(v: &[f64]) -> Option<f64> {
    v.iter().copied().reduce(f64::max)
}

pub fn sum(v: &[f64]) -> f64 {
    v.iter().sum()
}

/// 分位点(線形補間、q は 0.0〜1.0)。空なら None。
pub fn quantile(v: &[f64], q: f64) -> Option<f64> {
    if v.is_empty() || !(0.0..=1.0).contains(&q) {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let pos = q * (s.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    Some(s[lo] + (s[hi] - s[lo]) * (pos - lo as f64))
}

pub fn median(v: &[f64]) -> Option<f64> {
    quantile(v, 0.5)
}

/// ピアソン相関係数。長さ不一致・分散0なら None。
pub fn correlation(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() != y.len() || x.len() < 2 {
        return None;
    }
    let (mx, my) = (mean(x)?, mean(y)?);
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx).powi(2);
        syy += (b - my).powi(2);
    }
    if sxx == 0.0 || syy == 0.0 {
        return None;
    }
    Some(sxy / (sxx * syy).sqrt())
}

/// 単回帰 y = a + b x の最小二乗解 (a, b)。
pub fn linear_regression(x: &[f64], y: &[f64]) -> Option<(f64, f64)> {
    if x.len() != y.len() || x.len() < 2 {
        return None;
    }
    let (mx, my) = (mean(x)?, mean(y)?);
    let sxx: f64 = x.iter().map(|a| (a - mx).powi(2)).sum();
    if sxx == 0.0 {
        return None;
    }
    let sxy: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
    let b = sxy / sxx;
    Some((my - b * mx, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_stats() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(mean(&v), Some(2.5));
        assert_eq!(median(&v), Some(2.5));
        assert_eq!(min(&v), Some(1.0));
        assert_eq!(max(&v), Some(4.0));
        assert!((std_dev(&v).unwrap() - 1.2909944487).abs() < 1e-9);
        assert_eq!(quantile(&v, 0.25), Some(1.75));
        assert_eq!(mean(&[]), None);
    }

    #[test]
    fn regression_and_corr() {
        let x = [1.0, 2.0, 3.0];
        let y = [3.0, 5.0, 7.0];
        let (a, b) = linear_regression(&x, &y).unwrap();
        assert!((a - 1.0).abs() < 1e-12 && (b - 2.0).abs() < 1e-12);
        assert!((correlation(&x, &y).unwrap() - 1.0).abs() < 1e-12);
    }
}
