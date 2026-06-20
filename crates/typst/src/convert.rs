//! Typst の `Pixmap`（プリマルチプライド）から正準形式 [`Rgba`]（ストレート
//! アルファ）への変換と、fit 用の ppp（pixels-per-pt）算出。

use paladocs_core::SizePt;
use paladocs_render::{PixelSize, Rgba};
use tiny_skia::Pixmap;

use crate::EngineError;

/// tiny-skia の [`Pixmap`]（プリマルチプライド RGBA8）を正準形式 [`Rgba`]
/// （RGBA8・**ストレート（非プリマルチプライド）アルファ**・row-major・左上原点）
/// へ変換する。
///
/// 各画素は [`PremultipliedColorU8::demultiply`](tiny_skia::PremultipliedColorU8::demultiply)
/// でストレート化する。`tiny-skia` のメモリ順は RGBA であり、正準形式の `R,G,B,A`
/// と一致する。
pub(crate) fn pixmap_to_rgba(pixmap: &Pixmap) -> Result<Rgba, EngineError> {
    let size = PixelSize {
        w: pixmap.width(),
        h: pixmap.height(),
    };
    let mut data = Vec::with_capacity(size.byte_len());
    for px in pixmap.pixels() {
        let c = px.demultiply();
        data.push(c.red());
        data.push(c.green());
        data.push(c.blue());
        data.push(c.alpha());
    }
    Rgba::new(size, data).map_err(|e| EngineError::Render(e.to_string()))
}

/// ページ（pt）を pixel ビューポートにアスペクト保持で収めるときの ppp を返す。
///
/// `paladocs_render::fit` でビューポート内の充填矩形を求め、`ppp = fit.w /
/// page_pt.w` とする。`page_pt` の幅または高さが非正のときはゼロ除算を避けて
/// `1.0` を返す。返り値は常に正。
pub(crate) fn ppp_for_fit(page_pt: SizePt, viewport: PixelSize) -> f32 {
    if page_pt.w <= 0.0 || page_pt.h <= 0.0 {
        return 1.0;
    }
    let content = PixelSize {
        w: (page_pt.w.round() as u32).max(1),
        h: (page_pt.h.round() as u32).max(1),
    };
    let rect = paladocs_render::fit(content, viewport);
    if rect.w == 0 {
        return 1.0;
    }
    rect.w as f32 / page_pt.w
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{Pixmap, PremultipliedColorU8};

    /// 既知の半透明プリマルチプライド色がストレートアルファの具体値へ変換される。
    ///
    /// premultiplied (100, 50, 25, 128) を tiny-skia の demultiply 規則
    /// `round(c * 255 / a)`（floor of `c/(a/255) + 0.5`）で展開すると:
    ///   r = (100 / (128/255) + 0.5) as u8 = 199
    ///   g = ( 50 / (128/255) + 0.5) as u8 = 100
    ///   b = ( 25 / (128/255) + 0.5) as u8 =  50
    ///   a = 128
    #[test]
    fn unpremultiply_known_semitransparent() {
        let mut pixmap = Pixmap::new(1, 1).unwrap();
        pixmap.pixels_mut()[0] = PremultipliedColorU8::from_rgba(100, 50, 25, 128).unwrap();
        let rgba = pixmap_to_rgba(&pixmap).unwrap();
        assert_eq!(rgba.size(), PixelSize { w: 1, h: 1 });
        assert_eq!(rgba.pixel(0, 0), Some([199, 100, 50, 128]));
    }

    /// 完全透明はストレート形式でも (0,0,0,0)。
    #[test]
    fn unpremultiply_transparent() {
        let pixmap = Pixmap::new(2, 2).unwrap();
        let rgba = pixmap_to_rgba(&pixmap).unwrap();
        assert_eq!(rgba.size().byte_len(), 16);
        assert!(rgba.as_bytes().iter().all(|&b| b == 0));
    }

    /// 不透明色は値が保たれる（demultiply は a==255 で恒等）。
    #[test]
    fn unpremultiply_opaque_preserved() {
        let mut pixmap = Pixmap::new(1, 1).unwrap();
        pixmap.pixels_mut()[0] = PremultipliedColorU8::from_rgba(10, 20, 30, 255).unwrap();
        let rgba = pixmap_to_rgba(&pixmap).unwrap();
        assert_eq!(rgba.pixel(0, 0), Some([10, 20, 30, 255]));
    }

    /// row-major・左上原点の並び: (0,0) と (1,0) を区別する。
    #[test]
    fn row_major_layout() {
        let mut pixmap = Pixmap::new(2, 1).unwrap();
        pixmap.pixels_mut()[0] = PremultipliedColorU8::from_rgba(11, 11, 11, 255).unwrap();
        pixmap.pixels_mut()[1] = PremultipliedColorU8::from_rgba(22, 22, 22, 255).unwrap();
        let rgba = pixmap_to_rgba(&pixmap).unwrap();
        assert_eq!(rgba.pixel(0, 0), Some([11, 11, 11, 255]));
        assert_eq!(rgba.pixel(1, 0), Some([22, 22, 22, 255]));
    }

    #[test]
    fn ppp_for_fit_16_9_same_aspect() {
        // page 100x56.25 pt を 1600x900 px に収める → ppp = 1600/100 = 16
        let ppp = ppp_for_fit(SizePt { w: 100.0, h: 56.25 }, PixelSize { w: 1600, h: 900 });
        assert!((ppp - 16.0).abs() < 1e-3, "ppp = {ppp}");
    }

    #[test]
    fn ppp_for_fit_zero_page() {
        let ppp = ppp_for_fit(SizePt { w: 0.0, h: 0.0 }, PixelSize { w: 100, h: 100 });
        assert_eq!(ppp, 1.0);
    }
}
