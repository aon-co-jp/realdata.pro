//! CPU ラスタライザ(GPU が無い環境向け)。
//!
//! 三角形ごとにエッジ関数 E(x, y) = A·x + B·y + C を画素中心で評価し、3辺とも 0 以上の画素を塗る。
//! 実行時に CPU を判定し、AVX-512 なら16画素、AVX2 なら8画素を1命令でまとめて判定・書き込みする。
//! どちらも無ければスカラーで処理する。
//! スカラー版と SIMD 版は、計算の順序と丸めを揃えている。行定数 `B·y + C` を先に求めてから、
//! `A·x` に足す。FMA は使わない。これにより、どの経路でも画素単位で同じ結果になる(テストで確認)。

/// 画素中心で評価するための、1つの三角形の3辺の係数。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Edges {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub c: [f32; 3],
    /// 画面内に切り詰めた外接矩形 [x0, x1) × [y0, y1)。
    pub x0: usize,
    pub x1: usize,
    pub y0: usize,
    pub y1: usize,
}

impl Edges {
    /// 画素座標の三角形から係数を作る。面積0や画面外なら None。
    pub fn new(p: [[f32; 2]; 3], w: usize, h: usize) -> Option<Edges> {
        let area =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
        if area == 0.0 || !area.is_finite() {
            return None;
        }
        // 内側で E ≥ 0 になる向きに揃える。
        let p = if area > 0.0 { p } else { [p[0], p[2], p[1]] };
        let mut e = Edges {
            a: [0.0; 3],
            b: [0.0; 3],
            c: [0.0; 3],
            x0: 0,
            x1: 0,
            y0: 0,
            y1: 0,
        };
        for i in 0..3 {
            let (s, t) = (p[i], p[(i + 1) % 3]);
            // E(q) = (t.x - s.x)(q.y - s.y) - (t.y - s.y)(q.x - s.x)
            e.a[i] = -(t[1] - s[1]);
            e.b[i] = t[0] - s[0];
            e.c[i] = (t[1] - s[1]) * s[0] - (t[0] - s[0]) * s[1];
        }
        let minx = p
            .iter()
            .map(|q| q[0])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0);
        let maxx = p
            .iter()
            .map(|q| q[0])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min(w as f32);
        let miny = p
            .iter()
            .map(|q| q[1])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0);
        let maxy = p
            .iter()
            .map(|q| q[1])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min(h as f32);
        if minx >= maxx || miny >= maxy {
            return None;
        }
        (e.x0, e.x1, e.y0, e.y1) = (minx as usize, maxx as usize, miny as usize, maxy as usize);
        Some(e)
    }
}

/// 使う命令セット。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Simd {
    Avx512,
    Avx2,
    Scalar,
}

impl Simd {
    /// 実行中の CPU で使える最速のもの(open-cpu の実行時判定)。
    /// 環境変数 `RRD_SIMD=avx2|scalar` で下位の経路に固定できる(比較・切り分け用)。
    pub fn detect() -> Simd {
        let forced = std::env::var("RRD_SIMD")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let caps = open_cpu::detect();
        let best = if cfg!(target_arch = "x86_64") && caps.avx512f {
            Simd::Avx512
        } else if cfg!(target_arch = "x86_64") && caps.avx2 {
            Simd::Avx2
        } else {
            Simd::Scalar
        };
        match (forced.as_str(), best) {
            ("scalar", _) => Simd::Scalar,
            ("avx2", Simd::Avx512 | Simd::Avx2) => Simd::Avx2,
            _ => best,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Simd::Avx512 => "AVX-512(16画素並列)",
            Simd::Avx2 => "AVX2(8画素並列)",
            Simd::Scalar => "スカラー",
        }
    }
}

/// 1つの三角形を `color`(RGBA を詰めた u32)で塗る。
pub(crate) fn fill(buf: &mut [u32], w: usize, e: &Edges, color: u32, simd: Simd) {
    for y in e.y0..e.y1 {
        let py = y as f32 + 0.5;
        let rc = [
            e.b[0] * py + e.c[0],
            e.b[1] * py + e.c[1],
            e.b[2] * py + e.c[2],
        ];
        let row = &mut buf[y * w..(y + 1) * w];
        let mut x = e.x0;
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: Simd::detect() が実行時に該当命令セットの存在を確認した場合のみ選ばれる。
            match simd {
                Simd::Avx512 => x = unsafe { x86::row_avx512(row, x, e.x1, &e.a, &rc, color) },
                Simd::Avx2 => x = unsafe { x86::row_avx2(row, x, e.x1, &e.a, &rc, color) },
                Simd::Scalar => {}
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        let _ = simd;
        row_scalar(row, x, e.x1, &e.a, &rc, color);
    }
}

#[inline]
fn row_scalar(row: &mut [u32], from: usize, to: usize, a: &[f32; 3], rc: &[f32; 3], color: u32) {
    for (x, px_out) in row.iter_mut().enumerate().take(to).skip(from) {
        let px = x as f32 + 0.5;
        if a[0] * px + rc[0] >= 0.0 && a[1] * px + rc[1] >= 0.0 && a[2] * px + rc[2] >= 0.0 {
            *px_out = color;
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use std::arch::x86_64::*;

    /// 8画素ずつ処理し、処理し終えた位置を返す(残りはスカラーで処理)。
    #[target_feature(enable = "avx2")]
    pub unsafe fn row_avx2(
        row: &mut [u32],
        mut x: usize,
        to: usize,
        a: &[f32; 3],
        rc: &[f32; 3],
        color: u32,
    ) -> usize {
        let lanes = _mm256_setr_ps(0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0);
        let half = _mm256_set1_ps(0.5);
        let zero = _mm256_setzero_ps();
        let (a0, a1, a2) = (
            _mm256_set1_ps(a[0]),
            _mm256_set1_ps(a[1]),
            _mm256_set1_ps(a[2]),
        );
        let (r0, r1, r2) = (
            _mm256_set1_ps(rc[0]),
            _mm256_set1_ps(rc[1]),
            _mm256_set1_ps(rc[2]),
        );
        let col = _mm256_castsi256_ps(_mm256_set1_epi32(color as i32));
        while x + 8 <= to {
            // px = (x + lane) + 0.5 — スカラー版の `x as f32 + 0.5` と同じ値(整数部は f32 で正確)
            let px = _mm256_add_ps(_mm256_add_ps(_mm256_set1_ps(x as f32), lanes), half);
            let e0 = _mm256_add_ps(_mm256_mul_ps(a0, px), r0);
            let e1 = _mm256_add_ps(_mm256_mul_ps(a1, px), r1);
            let e2 = _mm256_add_ps(_mm256_mul_ps(a2, px), r2);
            let m = _mm256_and_ps(
                _mm256_and_ps(
                    _mm256_cmp_ps::<_CMP_GE_OQ>(e0, zero),
                    _mm256_cmp_ps::<_CMP_GE_OQ>(e1, zero),
                ),
                _mm256_cmp_ps::<_CMP_GE_OQ>(e2, zero),
            );
            let p = row.as_mut_ptr().add(x) as *mut __m256i;
            let cur = _mm256_castsi256_ps(_mm256_loadu_si256(p));
            _mm256_storeu_si256(p, _mm256_castps_si256(_mm256_blendv_ps(cur, col, m)));
            x += 8;
        }
        x
    }

    /// 16画素ずつ処理し、マスク付きストアで内側の画素だけを書き込む。
    #[target_feature(enable = "avx512f")]
    pub unsafe fn row_avx512(
        row: &mut [u32],
        mut x: usize,
        to: usize,
        a: &[f32; 3],
        rc: &[f32; 3],
        color: u32,
    ) -> usize {
        let lanes = _mm512_setr_ps(
            0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0,
        );
        let half = _mm512_set1_ps(0.5);
        let zero = _mm512_setzero_ps();
        let (a0, a1, a2) = (
            _mm512_set1_ps(a[0]),
            _mm512_set1_ps(a[1]),
            _mm512_set1_ps(a[2]),
        );
        let (r0, r1, r2) = (
            _mm512_set1_ps(rc[0]),
            _mm512_set1_ps(rc[1]),
            _mm512_set1_ps(rc[2]),
        );
        let col = _mm512_set1_epi32(color as i32);
        while x + 16 <= to {
            let px = _mm512_add_ps(_mm512_add_ps(_mm512_set1_ps(x as f32), lanes), half);
            let e0 = _mm512_add_ps(_mm512_mul_ps(a0, px), r0);
            let e1 = _mm512_add_ps(_mm512_mul_ps(a1, px), r1);
            let e2 = _mm512_add_ps(_mm512_mul_ps(a2, px), r2);
            let m = _mm512_cmp_ps_mask::<_CMP_GE_OQ>(e0, zero)
                & _mm512_cmp_ps_mask::<_CMP_GE_OQ>(e1, zero)
                & _mm512_cmp_ps_mask::<_CMP_GE_OQ>(e2, zero);
            _mm512_mask_storeu_epi32(row.as_mut_ptr().add(x) as *mut i32, m, col);
            x += 16;
        }
        x
    }
}
