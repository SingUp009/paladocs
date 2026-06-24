//! cell-mode のサイジング（grid）と full grid 合成（純粋）。
//!
//! 意味的 TUI 投影では inner グリッドを**本文サイズ基準**で決める（[`text_grid`]）:
//! `pt_per_col ≈ body/2`・`pt_per_row ≈ body` とし、欧文 1 文字 ≒ 1 セル・CJK ≒ 2 セル
//! に詰めて可読にする。セルの視覚アスペクトを 1:2 とみなすとスライドアスペクトも保たれる。
//! 本文が無いデッキはアスペクト基準の [`letterbox`]（`cols : rows*2`）へフォールバック。
//! いずれも `render_step` に渡す inner 寸法 `(icols, irows)` と中央寄せオフセット
//! `(off_col, off_row)` を返す。
//!
//! 合成は cli 側の責務（render に新 API を足さない）。inner を `CellGrid::set` で
//! 余白入りの full グリッドへ焼き込む（[`compose_full`]、余白は端末既定で透過）。

use paladocs_render::{CellGrid, Color};

/// letterbox 余白の背景（端末既定色＝透過、[`paladocs_render::DEFAULT`]）。
///
/// 意味的 TUI 投影では地色を焼かず端末テーマに追従させるため、レターボックス帯も
/// 端末既定色（`term` が SGR `49` を発行）にする。これでスライド本体・余白とも端末
/// 背景が透ける。
pub const DEFAULT_BG: Color = paladocs_render::DEFAULT;

/// letterbox の結果。`(icols, irows)` は inner グリッド寸法、`(off_col, off_row)` は
/// full グリッド内の中央寄せオフセット（セル）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Letterbox {
    /// inner グリッドのカラム数。
    pub icols: u16,
    /// inner グリッドの行数。
    pub irows: u16,
    /// inner グリッドの左端カラムオフセット。
    pub off_col: u16,
    /// inner グリッドの上端行オフセット。
    pub off_row: u16,
}

/// 端末 `(cols, rows)` とスライド `(w_pt, h_pt)` から inner 寸法＋オフセットを求める。
///
/// セル画素アスペクト `cols : rows*2` と比較し:
/// - `W/H > cols/(rows*2)`（スライドが相対的に横長）→ 幅律速:
///   `icols = cols`, `irows = round(cols*H/(2W))`。
/// - そうでなければ高さ律速: `irows = rows`, `icols = round(2*rows*W/H)`。
///
/// `icols`/`irows` は `1..=(cols|rows)` にクランプ（丸めオーバー・アンダー対策）。
/// `cols`/`rows` が 0、または `w_pt`/`h_pt` が非正のときは letterbox せず
/// `icols=cols, irows=rows, off=0` を返す。
pub fn letterbox(cols: u16, rows: u16, w_pt: f64, h_pt: f64) -> Letterbox {
    if cols == 0 || rows == 0 || w_pt <= 0.0 || h_pt <= 0.0 {
        return Letterbox {
            icols: cols,
            irows: rows,
            off_col: 0,
            off_row: 0,
        };
    }
    let cols_f = cols as f64;
    let rows_f = rows as f64;
    // W/H > cols/(rows*2) ⇔ W*2*rows > H*cols（除算回避）。
    let (icols, irows) = if w_pt * 2.0 * rows_f > h_pt * cols_f {
        // 横長 → 幅律速。
        (cols, clamp_dim(cols_f * h_pt / (2.0 * w_pt), rows))
    } else {
        // 高さ律速。
        (clamp_dim(2.0 * rows_f * w_pt / h_pt, cols), rows)
    };
    Letterbox {
        icols,
        irows,
        off_col: (cols - icols) / 2,
        off_row: (rows - irows) / 2,
    }
}

/// 本文サイズ基準で inner グリッド寸法＋オフセットを求める（密詰め＝可読サイジング）。
///
/// アスペクト基準の [`letterbox`] は `pt_per_col = w_pt/icols` がグリフ advance より
/// 小さくなり、比例配置でテキストが間延びする。本関数は本文フォント `body_pt` から
/// `pt_per_col ≈ body/2`・`pt_per_row ≈ body` になるよう
/// `icols = round(2*w_pt/body)`・`irows = round(h_pt/body)` を選ぶ。これで欧文 1 文字
/// ≒ 1 セル、CJK（[`CellWidth::Wide`](paladocs_render::CellWidth) で 2 セル）も詰まる。
/// セルの視覚アスペクトを 1:2 とみなすと `icols : irows*2 == w : h` になり、スライドの
/// アスペクトも保たれる。各寸法は `1..=(cols|rows)` にクランプし中央寄せする。
///
/// `body_pt <= 0`（テキスト無し等）や `cols/rows == 0`・`w/h <= 0` のときはアスペクト
/// 基準の [`letterbox`] にフォールバックする。
pub fn text_grid(cols: u16, rows: u16, w_pt: f64, h_pt: f64, body_pt: f64) -> Letterbox {
    if cols == 0 || rows == 0 || w_pt <= 0.0 || h_pt <= 0.0 || body_pt <= 0.0 {
        return letterbox(cols, rows, w_pt, h_pt);
    }
    let icols = clamp_dim(2.0 * w_pt / body_pt, cols);
    let irows = clamp_dim(h_pt / body_pt, rows);
    Letterbox {
        icols,
        irows,
        off_col: (cols - icols) / 2,
        off_row: (rows - irows) / 2,
    }
}

/// `v` を丸めて `1..=max` にクランプする（`max >= 1` 前提）。非有限は `max`。
fn clamp_dim(v: f64, max: u16) -> u16 {
    if !v.is_finite() {
        return max;
    }
    let v = v.round();
    if v < 1.0 {
        1
    } else if v > max as f64 {
        max
    } else {
        v as u16
    }
}

/// inner グリッドの base ラスタ解像度（pixels-per-pt）を求める。
///
/// 量子化のサンプル格子は `icols × (irows*2)`。box 平均が各サブセルで 1 画素以上を
/// 拾えるよう、ラスタ高さがサンプル高さと 1:1 になる `2*irows/h_pt` を返す。
/// `h_pt<=0`/`irows==0` の異常系は 1.0。AA 目的の oversample は follow-up の調整点。
pub fn ppp_for(irows: u16, h_pt: f64) -> f32 {
    if h_pt <= 0.0 || irows == 0 {
        return 1.0;
    }
    ((2.0 * irows as f64) / h_pt) as f32
}

/// `inner` を `(cols, rows)` の full グリッドへ `(off_col, off_row)` から焼き込む。
///
/// 余白は `bg` の blank セル（不変条件 3）。行単位のセルコピーなので
/// [`CellWidth::Wide`](paladocs_render::CellWidth) とその右隣
/// [`CellWidth::Continuation`](paladocs_render::CellWidth) の対は保たれる（不変条件 4）。
/// full 範囲外へはみ出す inner セルは [`CellGrid::set`] が無視する。
pub fn compose_full(
    inner: &CellGrid,
    cols: u16,
    rows: u16,
    off_col: u16,
    off_row: u16,
    bg: Color,
) -> CellGrid {
    let mut full = CellGrid::new_blank(cols, rows, bg);
    let (icols, irows) = inner.dims();
    for r in 0..irows {
        for c in 0..icols {
            full.set(
                off_col.saturating_add(c),
                off_row.saturating_add(r),
                inner.get(c, r).clone(),
            );
        }
    }
    full
}

#[cfg(test)]
mod tests {
    use super::*;
    use paladocs_render::{Cell, CellAttrs, CellWidth};

    const WHITE: Color = [255, 255, 255, 255];

    #[test]
    fn wide_slide_on_square_terminal_is_width_limited() {
        // 40x40 セル端末（画素 40:80）に 16:9 横長スライド → 幅いっぱい・上下余白。
        let lb = letterbox(40, 40, 1600.0, 900.0);
        assert_eq!(lb.icols, 40); // 幅律速
        assert_eq!(lb.off_col, 0);
        // irows = round(40*900/(2*1600)) = round(11.25) = 11、off_row=(40-11)/2=14。
        assert_eq!(lb.irows, 11);
        assert_eq!(lb.off_row, 14);
    }

    #[test]
    fn tall_slide_on_square_terminal_is_height_limited() {
        // 判別ペア: 縦長スライド → 高さいっぱい・左右余白。
        // 40x40 端末（画素 40:80）に 400:1600（=0.25 < 0.5）の縦長。
        let lb = letterbox(40, 40, 400.0, 1600.0);
        assert_eq!(lb.irows, 40); // 高さ律速
        assert_eq!(lb.off_row, 0);
        // icols = round(2*40*400/1600) = round(20) = 20、off_col=(40-20)/2=10。
        assert_eq!(lb.icols, 20);
        assert_eq!(lb.off_col, 10);
    }

    #[test]
    fn rounding_under_clamps_to_one() {
        // 極端な横長 → irows が 0 に丸まるところを 1 にクランプ。
        let lb = letterbox(10, 10, 10000.0, 1.0);
        assert_eq!(lb.icols, 10);
        assert_eq!(lb.irows, 1);
        assert_eq!(lb.off_row, 4); // (10-1)/2
    }

    #[test]
    fn nonpositive_or_zero_dims_fall_back_without_letterbox() {
        let lb = letterbox(80, 24, 0.0, 100.0);
        assert_eq!(
            lb,
            Letterbox {
                icols: 80,
                irows: 24,
                off_col: 0,
                off_row: 0,
            }
        );
        let lb0 = letterbox(0, 0, 100.0, 100.0);
        assert_eq!(lb0.icols, 0);
        assert_eq!(lb0.irows, 0);
    }

    #[test]
    fn text_grid_sizes_from_body_font() {
        // 842x474 pt のスライド・本文 16pt を広い端末(200x60)へ。
        // icols = round(2*842/16) = round(105.25) = 105、irows = round(474/16) = round(29.6) = 30。
        // 判別: アスペクト基準なら幅律速で icols=200 になり落ちる。
        let lb = text_grid(200, 60, 842.0, 474.0, 16.0);
        assert_eq!(lb.icols, 105);
        assert_eq!(lb.irows, 30);
        assert_eq!(lb.off_col, (200 - 105) / 2);
        assert_eq!(lb.off_row, (60 - 30) / 2);
    }

    #[test]
    fn text_grid_clamps_to_terminal_width() {
        // 端末が本文容量より狭い → icols は端末幅にクランプ・オフセット 0。
        let lb = text_grid(50, 60, 842.0, 474.0, 16.0);
        assert_eq!(lb.icols, 50);
        assert_eq!(lb.off_col, 0);
    }

    #[test]
    fn text_grid_falls_back_to_aspect_when_no_body() {
        // body_pt <= 0（テキスト無し）→ アスペクト基準の letterbox と一致。
        let cols = 40;
        let rows = 40;
        let (w, h) = (1600.0, 900.0);
        assert_eq!(
            text_grid(cols, rows, w, h, 0.0),
            letterbox(cols, rows, w, h)
        );
    }

    #[test]
    fn ppp_matches_sample_grid_and_clamps() {
        // 2*irows/h_pt。irows=50, h=100 → 1.0。irows=200, h=100 → 4.0。
        assert_eq!(ppp_for(50, 100.0), 1.0);
        assert_eq!(ppp_for(200, 100.0), 4.0);
        // h<=0 / irows==0 は 1.0。
        assert_eq!(ppp_for(50, 0.0), 1.0);
        assert_eq!(ppp_for(0, 100.0), 1.0);
    }

    #[test]
    fn compose_places_inner_and_blanks_margins() {
        // 4x3 full に 2x1 inner を (1,1) から焼く。
        let mut inner = CellGrid::new_blank(2, 1, WHITE);
        inner.set(
            0,
            0,
            Cell {
                ch: 'a',
                fg: WHITE,
                bg: WHITE,
                width: CellWidth::Narrow,
                attrs: CellAttrs::NONE,
            },
        );
        let full = compose_full(&inner, 4, 3, 1, 1, DEFAULT_BG);
        assert_eq!(full.dims(), (4, 3));
        // 配置位置に inner が入る。
        assert_eq!(full.get(1, 1).ch, 'a');
        // 余白は既定 bg の blank。
        assert_eq!(*full.get(0, 0), Cell::blank(DEFAULT_BG, DEFAULT_BG));
        assert_eq!(*full.get(3, 2), Cell::blank(DEFAULT_BG, DEFAULT_BG));
    }

    #[test]
    fn compose_preserves_wide_continuation_pair() {
        // inner の Wide+Continuation 対が blit 後も保たれる（不変条件 4）。
        let mut inner = CellGrid::new_blank(2, 1, WHITE);
        inner.set(
            0,
            0,
            Cell {
                ch: '漢',
                fg: WHITE,
                bg: WHITE,
                width: CellWidth::Wide,
                attrs: CellAttrs::NONE,
            },
        );
        inner.set(
            1,
            0,
            Cell {
                ch: ' ',
                fg: WHITE,
                bg: WHITE,
                width: CellWidth::Continuation,
                attrs: CellAttrs::NONE,
            },
        );
        let full = compose_full(&inner, 6, 1, 2, 0, DEFAULT_BG);
        assert_eq!(full.get(2, 0).width, CellWidth::Wide);
        assert_eq!(full.get(3, 0).width, CellWidth::Continuation);
        assert_eq!(full.get(2, 0).ch, '漢');
    }
}
