//! 配置幾何: viewport / セル寸法と、pixel 原点 → Kitty 配置（アンカーセル + セル内
//! オフセット）への純粋写像。
//!
//! `cli` が ioctl で測った viewport（cols, rows, セル pixel 寸法）を term に渡す。
//! term は pixel 矩形（`render::fit` の結果や変化矩形）を **アンカーセル + セル内
//! ピクセルオフセット**へ写す。これが「cell マッピングは term の責務」の中身。

use crate::ids::{ImageId, PlacementId};
use paladocs_render::PixelSize;

/// 1 セルの pixel 寸法。**前提: `w_px > 0` かつ `h_px > 0`**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSize {
    /// セル幅（pixel）。
    pub w_px: u32,
    /// セル高さ（pixel）。
    pub h_px: u32,
}

/// 端末 viewport。`cli` が測って渡す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// 列数（セル）。
    pub cols: u32,
    /// 行数（セル）。
    pub rows: u32,
    /// 1 セルの pixel 寸法。
    pub cell: CellSize,
}

impl Viewport {
    /// pixel での viewport サイズ（`cols*w_px × rows*h_px`）。
    pub fn pixel_size(&self) -> PixelSize {
        PixelSize {
            w: self.cols.saturating_mul(self.cell.w_px),
            h: self.rows.saturating_mul(self.cell.h_px),
        }
    }
}

/// アンカーセル位置（0-based）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPos {
    /// 列（0-based）。
    pub col: u32,
    /// 行（0-based）。
    pub row: u32,
}

/// セル内 pixel オフセット。**不変条件: `x < cell.w_px` かつ `y < cell.h_px`**。
///
/// Kitty の `X=`/`Y=` は u16 で、セルサイズ未満でなければならない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelOffset {
    /// セル内 x オフセット（pixel）。
    pub x: u16,
    /// セル内 y オフセット（pixel）。
    pub y: u16,
}

/// 配置済み画像 1 件の Kitty 配置パラメータ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// 配置する画像 ID。
    pub image: ImageId,
    /// この placement の ID。
    pub id: PlacementId,
    /// アンカーセル（0-based）。
    pub cell: CellPos,
    /// セル内 pixel オフセット。
    pub offset: PixelOffset,
    /// z-index（小さいほど下、大きいほど上）。
    pub z: i32,
}

/// 絶対 pixel 原点 `(x, y)` を、アンカーセルとセル内オフセットへ写す純関数。
///
/// `col = x / w_px`, `X = x % w_px`（`Y` も同様）。`X < w_px` は構造的に保証される。
///
/// 次の場合は `None`:
/// - セル寸法が 0（ゼロ除算回避）。
/// - セル内オフセットが `u16` に収まらない（セル寸法が `> 65535` の異常系）。
pub fn place_geometry(x: u32, y: u32, cell: CellSize) -> Option<(CellPos, PixelOffset)> {
    if cell.w_px == 0 || cell.h_px == 0 {
        return None;
    }
    let col = x / cell.w_px;
    let row = y / cell.h_px;
    let off_x = u16::try_from(x % cell.w_px).ok()?;
    let off_y = u16::try_from(y % cell.h_px).ok()?;
    Some((CellPos { col, row }, PixelOffset { x: off_x, y: off_y }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use paladocs_render::fit;

    fn cell() -> CellSize {
        CellSize { w_px: 10, h_px: 20 }
    }

    #[test]
    fn origin_maps_to_cell_zero() {
        assert_eq!(
            place_geometry(0, 0, cell()),
            Some((CellPos { col: 0, row: 0 }, PixelOffset { x: 0, y: 0 }))
        );
    }

    #[test]
    fn fractional_offset_within_cell() {
        // x=25 → col 2, X 5 ; y=45 → row 2, Y 5
        assert_eq!(
            place_geometry(25, 45, cell()),
            Some((CellPos { col: 2, row: 2 }, PixelOffset { x: 5, y: 5 }))
        );
    }

    #[test]
    fn exact_cell_boundary_has_zero_offset() {
        // x=20 (== 2*10) → col 2, X 0 ; y=40 (== 2*20) → row 2, Y 0
        assert_eq!(
            place_geometry(20, 40, cell()),
            Some((CellPos { col: 2, row: 2 }, PixelOffset { x: 0, y: 0 }))
        );
    }

    #[test]
    fn offset_always_below_cell_size() {
        for x in 0..100u32 {
            let (_, off) = place_geometry(x, 0, cell()).unwrap();
            assert!((off.x as u32) < cell().w_px);
        }
    }

    #[test]
    fn zero_cell_is_none() {
        assert_eq!(place_geometry(5, 5, CellSize { w_px: 0, h_px: 20 }), None);
        assert_eq!(place_geometry(5, 5, CellSize { w_px: 10, h_px: 0 }), None);
    }

    #[test]
    fn fit_then_place_composes() {
        // viewport 800x480 px (80 cols * 10, 24 rows * 20), content 600x480
        // → fit scale 1, rect x=100, y=0 → アンカー col 10, row 0, offset 0,0
        let vp = Viewport {
            cols: 80,
            rows: 24,
            cell: cell(),
        };
        let rect = fit(PixelSize { w: 600, h: 480 }, vp.pixel_size());
        assert_eq!((rect.x, rect.y, rect.w, rect.h), (100, 0, 600, 480));
        assert_eq!(
            place_geometry(rect.x, rect.y, vp.cell),
            Some((CellPos { col: 10, row: 0 }, PixelOffset { x: 0, y: 0 }))
        );
    }
}
