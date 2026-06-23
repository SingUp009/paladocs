//! `paladocs-render` — 解像度を持つ pixel / 幾何の描画プリミティブ。
//!
//! 依存方向は `term → render → core` および `typst → render → core`。本クレートは
//! **pixels を持つが I/O は持たない**純粋層であり、将来の `paladocs-typst`
//! （Frame を生成する側）と `paladocs-term`（Frame/Layer を消費する側）の
//! **共通の契約**となる。Typst コンパイル・端末プロトコル・ファイル/標準入出力・
//! 画像のスケーリングはいずれも本クレートの範囲外である。
//!
//! # 正準ピクセル形式（プロジェクト共通）
//!
//! 本クレートが定義し、全クレートが従う正準形式は次のとおり:
//!
//! - **RGBA8**: 1 チャンネル 8bit、チャンネル順は `R, G, B, A`。
//! - **ストレート（非プリマルチプライド）アルファ**。
//! - 行優先（row-major）、**左上原点**、隙間なし詰め（stride = `width * 4`）。
//! - 色は **sRGB バイト空間**のまま扱う。合成も sRGB バイトで行い、線形空間へは
//!   変換しない。
//!   - 根拠: Typst / 端末プロトコルの出力に揃え、PDF とのズレや意図しない色変化を
//!     避ける。将来フリンジ（合成端の色にじみ）が問題化したら線形合成への移行を
//!     再検討する。
//!
//! `tiny-skia`（将来 `typst` 側が使う）の `Pixmap` は**プリマルチプライド**なので、
//! `typst` 側で**アンプリマルチプライ**して本形式へ変換する責務がある（本クレートは
//! ストレート前提）。
//!
//! # 純粋性
//!
//! [`changed_region`] / [`crop`] / [`composite`] / [`fit`] はすべて純粋関数であり、
//! パニックしない。範囲外の入力に対しては `None` を返すか、範囲外画素をクリップする。

use paladocs_core::{FrameId, SizePt};
use std::fmt;

mod cell;
pub use cell::*;

/// pixel 単位の 2D サイズ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelSize {
    /// 幅（pixel）。
    pub w: u32,
    /// 高さ（pixel）。
    pub h: u32,
}

impl PixelSize {
    /// 画素数（`w * h`）。
    pub fn area(self) -> usize {
        self.w as usize * self.h as usize
    }

    /// 正準形式（RGBA8）でのバイト数（`area * 4`）。
    pub fn byte_len(self) -> usize {
        self.area() * 4
    }
}

/// 正準形式の所有 RGBA バッファ（クレートレベル doc の「正準ピクセル形式」を参照）。
///
/// 不変条件: `as_bytes().len() == size().byte_len()`。内部バッファは非公開で、
/// [`Rgba::new`] が長さを検査することでこの不変条件を保証する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    size: PixelSize,
    // 不変条件: data.len() == size.byte_len()
    data: Vec<u8>,
}

impl Rgba {
    /// `data` を所有 RGBA バッファにする。
    ///
    /// `data.len()` が `size.byte_len()` と一致しなければ
    /// [`RgbaError::LengthMismatch`] を返す。
    pub fn new(size: PixelSize, data: Vec<u8>) -> Result<Self, RgbaError> {
        let expected = size.byte_len();
        if data.len() != expected {
            return Err(RgbaError::LengthMismatch {
                expected,
                found: data.len(),
            });
        }
        Ok(Self { size, data })
    }

    /// 全画素を `(0, 0, 0, 0)`（完全透明）で確保する。
    pub fn transparent(size: PixelSize) -> Self {
        Self {
            size,
            data: vec![0u8; size.byte_len()],
        }
    }

    /// このバッファのサイズ。
    pub fn size(&self) -> PixelSize {
        self.size
    }

    /// 正準形式の生バイト列（`R, G, B, A` の row-major）。
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// `(x, y)` の 4 バイト `[r, g, b, a]`。範囲外は `None`。
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.size.w || y >= self.size.h {
            return None;
        }
        let i = (y as usize * self.size.w as usize + x as usize) * 4;
        Some([
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ])
    }
}

/// pixel 空間の矩形（フレーム座標、左上原点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// 左端 x（pixel）。
    pub x: u32,
    /// 上端 y（pixel）。
    pub y: u32,
    /// 幅（pixel）。
    pub w: u32,
    /// 高さ（pixel）。
    pub h: u32,
}

/// 完全に描画された 1 ページ。
#[derive(Debug, Clone)]
pub struct Frame {
    /// 発表順の通しページ番号。
    pub id: FrameId,
    /// ページ全体の正準 RGBA 画像。
    pub image: Rgba,
}

/// 合成対象の部分画像。
///
/// 前提: `image.size() == PixelSize { w: rect.w, h: rect.h }`。`z` は合成順序
/// （昇順で base に近い側＝下、降順で上）を表す。
#[derive(Debug, Clone)]
pub struct Layer {
    /// フレーム座標での配置矩形。
    pub rect: Rect,
    /// `rect` と同じサイズの正準 RGBA 画像。
    pub image: Rgba,
    /// 合成順序。小さいほど下、大きいほど上。
    pub z: i32,
}

/// [`Rgba::new`] が返すエラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbaError {
    /// `data` 長が `size.byte_len()` と一致しない。
    LengthMismatch {
        /// 期待バイト長（`size.byte_len()`）。
        expected: usize,
        /// 実際に渡されたバイト長。
        found: usize,
    },
}

impl fmt::Display for RgbaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthMismatch { expected, found } => write!(
                f,
                "rgba buffer length mismatch: expected {expected} bytes, found {found}"
            ),
        }
    }
}

impl std::error::Error for RgbaError {}

/// 2 フレームで内容が異なる画素の最小包含矩形（単一 bbox）。
///
/// 完全一致なら `None`。1 画素でも違えばその位置を覆う矩形（最小で `1×1`）を返す。
///
/// 事前条件: `a.image.size() == b.image.size()`（呼び出し側が保証する。サイズが
/// 異なる場合は「差分せず全置換」を選ぶべきで、本関数は `debug_assert!` で確認する
/// のみ）。サイズが一致しない release ビルドでは、共通範囲外を無視して走査する。
///
/// TODO: 現状は単一 bbox。タイル分割・複数矩形による部分更新の細粒度化は将来の
/// 最適化として保留する（reveal は通常 1 領域なので単一 bbox で十分）。
pub fn changed_region(a: &Frame, b: &Frame) -> Option<Rect> {
    debug_assert!(
        a.image.size() == b.image.size(),
        "changed_region precondition: frame sizes must match"
    );

    let sa = a.image.size();
    let sb = b.image.size();
    let w = sa.w.min(sb.w);
    let h = sa.h.min(sb.h);

    let ab = a.image.as_bytes();
    let bb = b.image.as_bytes();
    let stride_a = sa.w as usize * 4;
    let stride_b = sb.w as usize * 4;

    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;

    for y in 0..h {
        let row_a = y as usize * stride_a;
        let row_b = y as usize * stride_b;
        for x in 0..w {
            let ia = row_a + x as usize * 4;
            let ib = row_b + x as usize * 4;
            if ab[ia..ia + 4] != bb[ib..ib + 4] {
                found = true;
                if x < min_x {
                    min_x = x;
                }
                if x > max_x {
                    max_x = x;
                }
                if y < min_y {
                    min_y = y;
                }
                if y > max_y {
                    max_y = y;
                }
            }
        }
    }

    if !found {
        return None;
    }
    Some(Rect {
        x: min_x,
        y: min_y,
        w: max_x - min_x + 1,
        h: max_y - min_y + 1,
    })
}

/// `src` の部分矩形 `rect` を切り出して新しい [`Rgba`] を返す。
///
/// `rect` が `src` の範囲を 1 画素でもはみ出す場合（オーバーフロー含む）は `None`。
pub fn crop(src: &Rgba, rect: Rect) -> Option<Rgba> {
    let size = src.size();
    let right = rect.x.checked_add(rect.w)?;
    let bottom = rect.y.checked_add(rect.h)?;
    if right > size.w || bottom > size.h {
        return None;
    }

    let src_stride = size.w as usize * 4;
    let dst_stride = rect.w as usize * 4;
    let mut data = vec![0u8; rect.h as usize * dst_stride];
    let bytes = src.as_bytes();
    for row in 0..rect.h as usize {
        let s = (rect.y as usize + row) * src_stride + rect.x as usize * 4;
        let d = row * dst_stride;
        data[d..d + dst_stride].copy_from_slice(&bytes[s..s + dst_stride]);
    }

    Some(Rgba {
        size: PixelSize {
            w: rect.w,
            h: rect.h,
        },
        data,
    })
}

/// 丸め付き 8bit 乗算: `round(a * b / 255)`。
fn mul255(a: u8, b: u8) -> u8 {
    ((a as u16 * b as u16 + 127) / 255) as u8
}

/// `base` の上に `layers` を z 昇順で source-over 合成し、新しい [`Frame`] を返す。
///
/// 各 layer は `rect` の位置に重ねる。`base` の外へはみ出す画素はクリップされ、
/// パニックしない。合成はストレートアルファの source-over（sRGB バイト空間）で行う:
///
/// - `out_a = src_a + mul255(dst_a, 255 - src_a)`
/// - `out_c = (src_c * src_a + dst_c * mul255(dst_a, 255 - src_a)) / out_a`
///   （`out_a == 0` なら 0）
///
/// 返り値の `id` は `base.id` を引き継ぐ。
pub fn composite(base: &Frame, layers: &[Layer]) -> Frame {
    let size = base.image.size();
    let mut data = base.image.as_bytes().to_vec();
    let stride = size.w as usize * 4;

    let mut order: Vec<&Layer> = layers.iter().collect();
    order.sort_by_key(|l| l.z); // 安定ソート: 同 z は入力順を保つ

    for layer in order {
        let lsize = layer.image.size();
        let lstride = lsize.w as usize * 4;
        let lbytes = layer.image.as_bytes();

        for ly in 0..lsize.h {
            let fy = layer.rect.y as u64 + ly as u64;
            if fy >= size.h as u64 {
                continue;
            }
            for lx in 0..lsize.w {
                let fx = layer.rect.x as u64 + lx as u64;
                if fx >= size.w as u64 {
                    continue;
                }

                let si = ly as usize * lstride + lx as usize * 4;
                let di = fy as usize * stride + fx as usize * 4;

                let sa = lbytes[si + 3];
                if sa == 0 {
                    continue; // 完全透明は dst を変えない
                }
                let inv = 255 - sa;
                let da = data[di + 3];
                let da_contrib = mul255(da, inv);
                let out_a = sa as u16 + da_contrib as u16; // <= 255

                if out_a == 0 {
                    data[di] = 0;
                    data[di + 1] = 0;
                    data[di + 2] = 0;
                    data[di + 3] = 0;
                    continue;
                }

                for c in 0..3 {
                    let sc = lbytes[si + c] as u32;
                    let dc = data[di + c] as u32;
                    let num = sc * sa as u32 + dc * da_contrib as u32;
                    data[di + c] = (num / out_a as u32) as u8;
                }
                data[di + 3] = out_a as u8;
            }
        }
    }

    Frame {
        id: base.id,
        image: Rgba { size, data },
    }
}

/// `content` をアスペクト保持で `viewport` 内に最大充填し、中央寄せした矩形。
///
/// 返り値は viewport 座標（左上原点）の [`Rect`]。スケールは
/// `min(vw / cw, vh / ch)`。結果の `w`, `h` は四捨五入し、viewport を超えないよう
/// クランプする。中央寄せのオフセットは整数除算（floor）で求めるため、余白が奇数
/// pixel のときは端数を**左／上**に寄せる（右／下に多く残る）。
///
/// `content.w == 0` または `content.h == 0` のときはゼロ除算を避けて
/// `Rect { x: 0, y: 0, w: 0, h: 0 }` を返す。
///
/// この矩形は「`typst` が再ラスタすべき目標解像度」と「`term` が画像を置く位置」の
/// 両方に使われる（本クレートは拡縮を行わない）。
pub fn fit(content: PixelSize, viewport: PixelSize) -> Rect {
    if content.w == 0 || content.h == 0 {
        return Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };
    }

    let cw = content.w as f64;
    let ch = content.h as f64;
    let vw = viewport.w as f64;
    let vh = viewport.h as f64;

    let scale = (vw / cw).min(vh / ch);

    let mut w = (cw * scale).round() as u32;
    let mut h = (ch * scale).round() as u32;
    w = w.min(viewport.w);
    h = h.min(viewport.h);

    let x = (viewport.w - w) / 2;
    let y = (viewport.h - h) / 2;
    Rect { x, y, w, h }
}

/// ページ（pt）を `viewport`（pixel）へアスペクト保持で収めるときの
/// **pixels-per-point** スケールを返す。
///
/// [`fit`] でビューポート内の充填矩形を求め、`scale = fit.w / page_pt.w` とする。
/// この値で `typst` 側がページを再ラスタすれば、出力寸法はビューポートに追従する
/// （固定 DPI ではない）。本クレートは拡縮を行わないため、スケールの算出のみ担う。
///
/// 退避値 `1.0` を返す場合:
/// - `page_pt.w <= 0` または `page_pt.h <= 0`（不正なページサイズ）。
/// - `fit` の結果が幅 0（ビューポートが空など）。
pub fn scale_for(page_pt: SizePt, viewport: PixelSize) -> f32 {
    if page_pt.w <= 0.0 || page_pt.h <= 0.0 {
        return 1.0;
    }
    let content = PixelSize {
        w: (page_pt.w.round() as u32).max(1),
        h: (page_pt.h.round() as u32).max(1),
    };
    let rect = fit(content, viewport);
    if rect.w == 0 {
        return 1.0;
    }
    rect.w as f32 / page_pt.w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(size: PixelSize, data: Vec<u8>) -> Frame {
        Frame {
            id: FrameId(0),
            image: Rgba::new(size, data).unwrap(),
        }
    }

    #[test]
    fn rgba_new_length_mismatch() {
        let size = PixelSize { w: 2, h: 2 };
        let err = Rgba::new(size, vec![0u8; 15]).unwrap_err();
        assert_eq!(
            err,
            RgbaError::LengthMismatch {
                expected: 16,
                found: 15,
            }
        );
    }

    #[test]
    fn rgba_transparent_is_all_zero() {
        let img = Rgba::transparent(PixelSize { w: 2, h: 2 });
        assert_eq!(img.as_bytes().len(), 16);
        assert!(img.as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn rgba_pixel_out_of_range() {
        let img = Rgba::transparent(PixelSize { w: 2, h: 2 });
        assert_eq!(img.pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(img.pixel(2, 0), None);
        assert_eq!(img.pixel(0, 2), None);
    }

    #[test]
    fn changed_region_identical() {
        let size = PixelSize { w: 3, h: 3 };
        let data = vec![1u8; size.byte_len()];
        let a = frame(size, data.clone());
        let b = frame(size, data);
        assert_eq!(changed_region(&a, &b), None);
    }

    #[test]
    fn changed_region_single_center_pixel() {
        let size = PixelSize { w: 3, h: 3 };
        let data = vec![0u8; size.byte_len()];
        let a = frame(size, data.clone());
        let mut data_b = data;
        // 中央 (1,1) の 1 画素のみ変更
        let (x, y) = (1usize, 1usize);
        let i = (y * 3 + x) * 4;
        data_b[i] = 255;
        let b = frame(size, data_b);
        assert_eq!(
            changed_region(&a, &b),
            Some(Rect {
                x: 1,
                y: 1,
                w: 1,
                h: 1
            })
        );
    }

    #[test]
    fn changed_region_rectangular_block() {
        let size = PixelSize { w: 4, h: 4 };
        let data = vec![0u8; size.byte_len()];
        let a = frame(size, data.clone());
        let mut data_b = data;
        // (1,1)..=(2,2) の 2x2 ブロックを変更
        for y in 1..=2u32 {
            for x in 1..=2u32 {
                let i = (y as usize * 4 + x as usize) * 4;
                data_b[i + 1] = 200;
            }
        }
        let b = frame(size, data_b);
        assert_eq!(
            changed_region(&a, &b),
            Some(Rect {
                x: 1,
                y: 1,
                w: 2,
                h: 2
            })
        );
    }

    #[test]
    fn changed_region_four_corners() {
        let size = PixelSize { w: 4, h: 4 };
        let data = vec![0u8; size.byte_len()];
        let a = frame(size, data.clone());
        let mut data_b = data;
        for (x, y) in [(0u32, 0u32), (3, 0), (0, 3), (3, 3)] {
            let i = (y as usize * 4 + x as usize) * 4;
            data_b[i + 2] = 123;
        }
        let b = frame(size, data_b);
        assert_eq!(
            changed_region(&a, &b),
            Some(Rect {
                x: 0,
                y: 0,
                w: 4,
                h: 4
            })
        );
    }

    #[test]
    fn crop_inner_rect() {
        // 2x2 の各画素を区別できる値で埋める
        let size = PixelSize { w: 2, h: 2 };
        let data = vec![
            10, 11, 12, 13, // (0,0)
            20, 21, 22, 23, // (1,0)
            30, 31, 32, 33, // (0,1)
            40, 41, 42, 43, // (1,1)
        ];
        let img = Rgba::new(size, data).unwrap();
        let out = crop(
            &img,
            Rect {
                x: 1,
                y: 0,
                w: 1,
                h: 2,
            },
        )
        .unwrap();
        assert_eq!(out.size(), PixelSize { w: 1, h: 2 });
        assert_eq!(out.as_bytes(), &[20, 21, 22, 23, 40, 41, 42, 43]);
    }

    #[test]
    fn crop_out_of_range() {
        let img = Rgba::transparent(PixelSize { w: 2, h: 2 });
        assert_eq!(
            crop(
                &img,
                Rect {
                    x: 1,
                    y: 1,
                    w: 2,
                    h: 2
                }
            ),
            None
        );
    }

    /// 不透明 (a=255) layer は base 画素を置換する。
    #[test]
    fn composite_opaque_layer_replaces() {
        let size = PixelSize { w: 2, h: 2 };
        let base = frame(size, vec![10u8; size.byte_len()]);
        let layer = Layer {
            rect: Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
            },
            image: Rgba::new(
                size,
                vec![
                    50, 60, 70, 255, 50, 60, 70, 255, 50, 60, 70, 255, 50, 60, 70, 255,
                ],
            )
            .unwrap(),
            z: 0,
        };
        let out = composite(&base, &[layer]);
        assert_eq!(out.image.pixel(0, 0), Some([50, 60, 70, 255]));
        assert_eq!(out.image.pixel(1, 1), Some([50, 60, 70, 255]));
    }

    /// α=128 の layer を不透明 base に合成: mul255 規則どおりの固定値。
    ///
    /// dst=(40,40,40,255), src=(200,200,200,128):
    ///   da_contrib = mul255(255, 127) = 127
    ///   out_a = 128 + 127 = 255
    ///   out_c = (200*128 + 40*127) / 255 = 30680 / 255 = 120
    #[test]
    fn composite_alpha_128_over_opaque() {
        let size = PixelSize { w: 1, h: 1 };
        let base = frame(size, vec![40, 40, 40, 255]);
        let layer = Layer {
            rect: Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            image: Rgba::new(size, vec![200, 200, 200, 128]).unwrap(),
            z: 0,
        };
        let out = composite(&base, &[layer]);
        assert_eq!(out.image.pixel(0, 0), Some([120, 120, 120, 255]));
    }

    /// rect オフセット配置: 指定位置のみ反映され、外側は不変。
    #[test]
    fn composite_rect_offset() {
        let size = PixelSize { w: 3, h: 3 };
        let base = frame(size, vec![0u8; size.byte_len()]);
        let layer = Layer {
            rect: Rect {
                x: 1,
                y: 2,
                w: 1,
                h: 1,
            },
            image: Rgba::new(PixelSize { w: 1, h: 1 }, vec![9, 8, 7, 255]).unwrap(),
            z: 0,
        };
        let out = composite(&base, &[layer]);
        assert_eq!(out.image.pixel(1, 2), Some([9, 8, 7, 255]));
        // 外側は元のまま
        assert_eq!(out.image.pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(out.image.pixel(2, 2), Some([0, 0, 0, 0]));
    }

    /// z 昇順: 高 z が上に来る。
    #[test]
    fn composite_z_order() {
        let size = PixelSize { w: 1, h: 1 };
        let base = frame(size, vec![0, 0, 0, 255]);
        let rect = Rect {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        };
        let low = Layer {
            rect,
            image: Rgba::new(size, vec![10, 10, 10, 255]).unwrap(),
            z: 0,
        };
        let high = Layer {
            rect,
            image: Rgba::new(size, vec![200, 200, 200, 255]).unwrap(),
            z: 5,
        };
        // 入力順を逆にしても z で並び替わり、高 z が上。
        let out = composite(&base, &[high.clone(), low.clone()]);
        assert_eq!(out.image.pixel(0, 0), Some([200, 200, 200, 255]));
        let out2 = composite(&base, &[low, high]);
        assert_eq!(out2.image.pixel(0, 0), Some([200, 200, 200, 255]));
    }

    /// はみ出し layer はクリップされ panic しない。
    #[test]
    fn composite_overhang_clipped() {
        let size = PixelSize { w: 2, h: 2 };
        let base = frame(size, vec![0u8; size.byte_len()]);
        // base の (1,1) を起点に 2x2 を置く → (1,1) のみ反映、残りはクリップ
        let layer = Layer {
            rect: Rect {
                x: 1,
                y: 1,
                w: 2,
                h: 2,
            },
            image: Rgba::new(PixelSize { w: 2, h: 2 }, vec![255u8; 16]).unwrap(),
            z: 0,
        };
        let out = composite(&base, &[layer]);
        assert_eq!(out.image.pixel(1, 1), Some([255, 255, 255, 255]));
        assert_eq!(out.image.pixel(0, 0), Some([0, 0, 0, 0]));
    }

    #[test]
    fn fit_same_aspect() {
        // 16:9 → 16:9: viewport 全面、offset 0
        let r = fit(
            PixelSize { w: 1920, h: 1080 },
            PixelSize { w: 1280, h: 720 },
        );
        assert_eq!(
            r,
            Rect {
                x: 0,
                y: 0,
                w: 1280,
                h: 720
            }
        );
    }

    #[test]
    fn fit_letterbox_top_bottom() {
        // 16:9 content → 4:3 viewport: 横いっぱい、上下に余白
        let r = fit(PixelSize { w: 1600, h: 900 }, PixelSize { w: 800, h: 600 });
        // scale = min(800/1600, 600/900) = min(0.5, 0.666) = 0.5
        // w = 800, h = 450 → 縦余白 150 を二分、上下 75
        assert_eq!(
            r,
            Rect {
                x: 0,
                y: 75,
                w: 800,
                h: 450
            }
        );
    }

    #[test]
    fn fit_pillarbox_left_right() {
        // 4:3 content → 16:9 viewport: 縦いっぱい、左右に余白
        let r = fit(PixelSize { w: 800, h: 600 }, PixelSize { w: 1600, h: 900 });
        // scale = min(1600/800, 900/600) = min(2.0, 1.5) = 1.5
        // w = 1200, h = 900 → 横余白 400 を二分、左右 200
        assert_eq!(
            r,
            Rect {
                x: 200,
                y: 0,
                w: 1200,
                h: 900
            }
        );
    }

    #[test]
    fn fit_zero_content() {
        let r = fit(PixelSize { w: 0, h: 0 }, PixelSize { w: 100, h: 100 });
        assert_eq!(
            r,
            Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0
            }
        );
    }

    #[test]
    fn scale_for_follows_viewport() {
        // page 100x56.25 pt（16:9）を 1600x900 px に収める → scale = 1600/100 = 16。
        let page = SizePt { w: 100.0, h: 56.25 };
        let s = scale_for(page, PixelSize { w: 1600, h: 900 });
        assert!((s - 16.0).abs() < 1e-3, "scale = {s}");
    }

    #[test]
    fn scale_for_doubles_with_viewport_dimension() {
        // 同一ページでビューポートの線形寸法が 2 倍（面積 4 倍）になれば scale も 2 倍。
        // identity / 固定 dpi 実装ではここで落ちる。
        let page = SizePt { w: 100.0, h: 56.25 };
        let small = scale_for(page, PixelSize { w: 800, h: 450 });
        let large = scale_for(page, PixelSize { w: 1600, h: 900 });
        assert!(
            (large - 2.0 * small).abs() < 1e-3,
            "small={small} large={large}"
        );
    }

    #[test]
    fn scale_for_invalid_page_is_unit() {
        assert_eq!(
            scale_for(SizePt { w: 0.0, h: 10.0 }, PixelSize { w: 100, h: 100 }),
            1.0
        );
        assert_eq!(
            scale_for(SizePt { w: 10.0, h: 0.0 }, PixelSize { w: 100, h: 100 }),
            1.0
        );
    }

    #[test]
    fn scale_for_zero_viewport_is_unit() {
        let page = SizePt { w: 100.0, h: 56.25 };
        assert_eq!(scale_for(page, PixelSize { w: 0, h: 0 }), 1.0);
    }
}
