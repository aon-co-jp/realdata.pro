//! realdata.pro のグラフ描画。
//!
//! グラフ(棒・円)を三角形メッシュにし、次のどちらかで描いて PNG にする。
//!
//! 1. **GPU**: open-directx の Vulkan 描画(`render_indexed_scene_with_depth_and_read_back`)。
//!    シェーダーは open-directx の `directx-shader-translate` で DXBC から SPIR-V に変換したものを使う。
//! 2. **CPU**: GPU が無い、または GPU 描画に失敗した場合。`raster` の SIMD ラスタライザで描く
//!    (AVX-512 / AVX2 / スカラー)。
//!
//! どちらの経路も、縦横2倍で描いてから 2×2 平均で縮小する(ジャギーを抑える)。
//! 文字(軸ラベル・凡例)はこの画像には描かない。画面側で HTML として表示する。

mod raster;

use std::time::Instant;

pub use raster::Simd;

/// 画素座標(左上原点、y は下向き)の三角形と、その色。
#[derive(Clone, Copy, Debug)]
pub struct Tri {
    pub p: [[f32; 2]; 3],
    pub rgba: [u8; 4],
}

/// 描画の指定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// GPU を試し、使えなければ CPU。
    Auto,
    Gpu,
    Cpu,
}

/// 描画結果。
pub struct Rendered {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// 実際に使った経路の説明(例: "GPU: NVIDIA GeForce GT 730"、"CPU: AVX2(8画素並列)")。
    pub backend: String,
    /// GPU を試したが使えなかった理由(CPU に切り替えた場合)。
    pub fallback_reason: Option<String>,
    pub millis: f64,
}

pub const PALETTE: [[u8; 4]; 8] = [
    [0x2f, 0x6f, 0xde, 255],
    [0xe8, 0x74, 0x3b, 255],
    [0x19, 0xa9, 0x79, 255],
    [0xd6, 0x4f, 0x8c, 255],
    [0x94, 0x5e, 0xcf, 255],
    [0xc9, 0xa2, 0x27, 255],
    [0x13, 0xa4, 0xb4, 255],
    [0x8a, 0x8f, 0x99, 255],
];
const BG: [u8; 4] = [255, 255, 255, 255];
const GRID: [u8; 4] = [0xdf, 0xe3, 0xea, 255];
const AXIS: [u8; 4] = [0x5f, 0x68, 0x78, 255];

/// 画像サイズの上限(描画は縦横2倍で行う)。
pub const MAX_SIDE: u32 = 2048;
const SS: u32 = 2;

fn quad(x0: f32, y0: f32, x1: f32, y1: f32, rgba: [u8; 4]) -> [Tri; 2] {
    [
        Tri {
            p: [[x0, y0], [x1, y0], [x1, y1]],
            rgba,
        },
        Tri {
            p: [[x0, y0], [x1, y1], [x0, y1]],
            rgba,
        },
    ]
}

/// 棒グラフのメッシュ(背景・目盛り線・軸・棒)。負の値は 0 の線より下に伸ばす。
pub fn bar_mesh(values: &[f64], w: u32, h: u32) -> Vec<Tri> {
    let (w, h) = (w as f32, h as f32);
    let mut t: Vec<Tri> = quad(0.0, 0.0, w, h, BG).to_vec();
    let (l, r, top, bottom) = (w * 0.06, w * 0.97, h * 0.06, h * 0.94);
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    let max = finite.iter().copied().fold(0.0_f64, f64::max);
    let min = finite.iter().copied().fold(0.0_f64, f64::min);
    let span = if max - min > 0.0 { max - min } else { 1.0 };
    let y_of = |v: f64| bottom - ((v - min) / span) as f32 * (bottom - top);
    let line = (h / 400.0).max(1.0);
    for i in 1..=4 {
        let y = top + (bottom - top) * i as f32 / 4.0;
        t.extend(quad(l, y - line / 2.0, r, y + line / 2.0, GRID));
    }
    let n = values.len().max(1) as f32;
    let slot = (r - l) / n;
    let zero = y_of(0.0);
    for (i, v) in values.iter().enumerate() {
        if !v.is_finite() {
            continue;
        }
        let x0 = l + slot * (i as f32 + 0.15);
        let x1 = l + slot * (i as f32 + 0.85);
        let y = y_of(*v);
        t.extend(quad(x0, y.min(zero), x1, y.max(zero), PALETTE[0]));
    }
    t.extend(quad(l, zero - line, r, zero + line, AXIS));
    t.extend(quad(l - line, top, l + line, bottom, AXIS));
    t
}

/// 円グラフのメッシュ。値は 0 以上であること(負・非有限は None)。
pub fn pie_mesh(values: &[f64], w: u32, h: u32) -> Option<Vec<Tri>> {
    if values.is_empty() || values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return None;
    }
    let total: f64 = values.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let (w, h) = (w as f32, h as f32);
    let mut t: Vec<Tri> = quad(0.0, 0.0, w, h, BG).to_vec();
    let (cx, cy, rad) = (w / 2.0, h / 2.0, w.min(h) * 0.45);
    let mut a0 = -std::f64::consts::FRAC_PI_2;
    for (i, v) in values.iter().enumerate() {
        let sweep = v / total * std::f64::consts::TAU;
        // 1度あたり1分割以上(大きい扇形でも滑らかにする)
        let segs = ((sweep.to_degrees()).ceil() as usize).max(1);
        let rgba = PALETTE[i % PALETTE.len()];
        for s in 0..segs {
            let s0 = a0 + sweep * s as f64 / segs as f64;
            let s1 = a0 + sweep * (s + 1) as f64 / segs as f64;
            let p = |a: f64| [cx + rad * a.cos() as f32, cy + rad * a.sin() as f32];
            t.push(Tri {
                p: [[cx, cy], p(s0), p(s1)],
                rgba,
            });
        }
        a0 += sweep;
    }
    Some(t)
}

fn pack(c: [u8; 4]) -> u32 {
    u32::from_le_bytes(c)
}

/// CPU(SIMD)で描く。戻り値は縦横 SS 倍のキャンバスの RGBA。
pub fn render_cpu(tris: &[Tri], w: u32, h: u32, simd: Simd) -> Vec<u8> {
    let (cw, ch) = ((w * SS) as usize, (h * SS) as usize);
    let mut buf = vec![0u32; cw * ch];
    for t in tris {
        let p = t.p.map(|q| [q[0] * SS as f32, q[1] * SS as f32]);
        if let Some(e) = raster::Edges::new(p, cw, ch) {
            raster::fill(&mut buf, cw, &e, pack(t.rgba), simd);
        }
    }
    buf.into_iter().flat_map(u32::to_le_bytes).collect()
}

// open-directx のパススルー用シェーダー(fxc でコンパイル済みの DXBC)
const TRIANGLE_VS_DXBC: &[u8] = include_bytes!(
    "../../../.deps/open-directx/crates/directx-shader-translate/shaders/triangle_vs.dxbc"
);
const TRIANGLE_PS_DXBC: &[u8] = include_bytes!(
    "../../../.deps/open-directx/crates/directx-shader-translate/shaders/triangle_ps.dxbc"
);

/// GPU(open-directx / Vulkan)で描く。戻り値は (RGBA, デバイス名)。
pub fn render_gpu(tris: &[Tri], w: u32, h: u32) -> Result<(Vec<u8>, String), String> {
    use directx_graphics_vulkan::{
        enumerate_graphics_devices, render_indexed_scene_with_depth_and_read_back, Vertex,
    };
    use directx_shader_translate::spirv_gen::{translate_pixel_shader, translate_vertex_shader};

    let devices = enumerate_graphics_devices().map_err(|e| e.to_string())?;
    let dev = devices
        .first()
        .ok_or("Vulkan のグラフィックスデバイスがありません")?;
    // ドライバの名前にベンダー名が含まれることが多いので、重複させない
    let device_name = if dev
        .name
        .to_ascii_lowercase()
        .contains(&dev.vendor.to_ascii_lowercase())
    {
        dev.name.clone()
    } else {
        format!("{} {}", dev.vendor, dev.name).trim().to_string()
    };
    let vs = translate_vertex_shader(TRIANGLE_VS_DXBC)
        .map_err(|e| format!("頂点シェーダーの変換に失敗: {e:?}"))?;
    let ps = translate_pixel_shader(TRIANGLE_PS_DXBC)
        .map_err(|e| format!("ピクセルシェーダーの変換に失敗: {e:?}"))?;

    let (cw, ch) = ((w * SS) as f32, (h * SS) as f32);
    let n = tris.len().max(1) as f32;
    let mut vertices = Vec::with_capacity(tris.len() * 3);
    for (i, t) in tris.iter().enumerate() {
        // 後に描く三角形ほど手前(深度テストは LESS)。CPU 経路の「後勝ち」と同じ見え方にする。
        let z = 0.95 - 0.9 * (i as f32 + 1.0) / (n + 1.0);
        let color = t.rgba.map(|c| c as f32 / 255.0);
        for q in t.p {
            // 画素座標 → Vulkan の NDC(y=-1 が最上段、ビューポートは反転なし)
            let x = q[0] * SS as f32 / cw * 2.0 - 1.0;
            let y = q[1] * SS as f32 / ch * 2.0 - 1.0;
            vertices.push(Vertex {
                pos: [x, y, z],
                color,
            });
        }
    }
    let indices: Vec<u32> = (0..vertices.len() as u32).collect();
    let px = render_indexed_scene_with_depth_and_read_back(
        &vs.spirv_words,
        &ps.spirv_words,
        &vertices,
        &indices,
        w * SS,
        h * SS,
    )
    .map_err(|e| e.to_string())?;
    let rgba = px.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
    Ok((rgba, device_name))
}

/// 縦横 SS 倍のキャンバスを 2×2 平均で縮小する。
fn downsample(big: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h, bw) = (w as usize, h as usize, (w * SS) as usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let at =
                    |dx: usize, dy: usize| big[((2 * y + dy) * bw + 2 * x + dx) * 4 + c] as u32;
                out[(y * w + x) * 4 + c] =
                    ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
            }
        }
    }
    out
}

pub fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())?;
    wr.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// メッシュを描いて PNG にする。
pub fn render(tris: &[Tri], w: u32, h: u32, backend: Backend) -> Result<Rendered, String> {
    if !(16..=MAX_SIDE).contains(&w) || !(16..=MAX_SIDE).contains(&h) {
        return Err(format!(
            "画像サイズは 16〜{MAX_SIDE} ピクセルで指定してください"
        ));
    }
    let start = Instant::now();
    let mut fallback_reason = None;
    let gpu = match backend {
        Backend::Cpu => None,
        Backend::Gpu | Backend::Auto => match render_gpu(tris, w, h) {
            Ok(v) => Some(v),
            Err(e) if backend == Backend::Gpu => {
                return Err(format!("GPU で描画できませんでした: {e}"))
            }
            Err(e) => {
                fallback_reason = Some(e);
                None
            }
        },
    };
    let (big, label) = match gpu {
        Some((rgba, name)) => (rgba, format!("GPU(open-directx / Vulkan): {name}")),
        None => {
            let simd = Simd::detect();
            (
                render_cpu(tris, w, h, simd),
                format!("CPU: {}", simd.label()),
            )
        }
    };
    let png = encode_png(&downsample(&big, w, h), w, h)?;
    Ok(Rendered {
        png,
        width: w,
        height: h,
        backend: label,
        fallback_reason,
        millis: start.elapsed().as_secs_f64() * 1000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_meshes() -> Vec<(Vec<Tri>, u32, u32)> {
        vec![
            (
                bar_mesh(&[450000.0, 250000.0, 75000.0, -30000.0, 120000.0], 333, 211),
                333,
                211,
            ),
            (
                pie_mesh(&[58.1, 32.3, 9.7, 3.3, 1.0], 257, 257).unwrap(),
                257,
                257,
            ),
        ]
    }

    #[test]
    fn simd_paths_match_scalar_pixel_for_pixel() {
        let best = Simd::detect();
        for (tris, w, h) in sample_meshes() {
            let scalar = render_cpu(&tris, w, h, Simd::Scalar);
            let caps = open_cpu::detect();
            if caps.avx2 {
                assert!(
                    render_cpu(&tris, w, h, Simd::Avx2) == scalar,
                    "AVX2 とスカラーの結果が異なる"
                );
            }
            if caps.avx512f {
                assert!(
                    render_cpu(&tris, w, h, Simd::Avx512) == scalar,
                    "AVX-512 とスカラーの結果が異なる"
                );
            }
        }
        eprintln!("この CPU の最速経路: {}", best.label());
    }

    #[test]
    fn bar_and_pie_colors_land_where_expected() {
        let tris = bar_mesh(&[10.0, 5.0], 200, 100);
        let img = render_cpu(&tris, 200, 100, Simd::detect());
        let px = |x: usize, y: usize| {
            let i = (y * 400 + x) * 4; // 2倍のキャンバス
            [img[i], img[i + 1], img[i + 2], img[i + 3]]
        };
        assert_eq!(px(4, 4), BG, "左上は背景");
        assert_eq!(px(100, 170), PALETTE[0], "1本目の棒の中");
        assert_eq!(px(300, 60), BG, "2本目の棒より上は背景");
        let pie = render_cpu(
            &pie_mesh(&[1.0, 1.0], 100, 100).unwrap(),
            100,
            100,
            Simd::detect(),
        );
        let at = |x: usize, y: usize| {
            let i = (y * 200 + x) * 4;
            [pie[i], pie[i + 1], pie[i + 2], pie[i + 3]]
        };
        assert_eq!(
            at(150, 100),
            PALETTE[0],
            "12時から時計回りの前半(右側)が1色目"
        );
        assert_eq!(at(50, 100), PALETTE[1], "左側が2色目");
        assert!(pie_mesh(&[1.0, -1.0], 10, 10).is_none());
    }

    #[test]
    fn cpu_render_produces_valid_png() {
        let r = render(&bar_mesh(&[1.0, 2.0, 3.0], 64, 48), 64, 48, Backend::Cpu).unwrap();
        assert_eq!(&r.png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(r.backend.starts_with("CPU"));
        assert!(render(&[], 8, 8, Backend::Cpu).is_err());
    }

    /// 実 GPU がある環境でのみ意味を持つ。GPU と CPU の画像がほぼ一致することを確かめる
    /// (三角形の境界の画素は、ラスタライズ規則の違いでわずかに異なり得る)。
    #[test]
    fn gpu_matches_cpu_when_available() {
        for (tris, w, h) in sample_meshes() {
            let (gpu, name) = match render_gpu(&tris, w, h) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("GPU なし/使用不可のため比較を省略: {e}");
                    return;
                }
            };
            let cpu = render_cpu(&tris, w, h, Simd::Scalar);
            let total = (w * SS * h * SS) as usize;
            let differ = (0..total)
                .filter(|i| gpu[i * 4..i * 4 + 4] != cpu[i * 4..i * 4 + 4])
                .count();
            let ratio = differ as f64 / total as f64;
            eprintln!(
                "GPU({name}) と CPU の差: {differ}/{total} 画素({:.3}%)",
                ratio * 100.0
            );
            assert!(
                ratio < 0.01,
                "GPU と CPU の画像が大きく異なる({:.3}%)",
                ratio * 100.0
            );
        }
    }
}
