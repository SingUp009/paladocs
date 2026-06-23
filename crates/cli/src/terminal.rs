//! 端末計測（不純シェル）: cols/rows とセル pixel 寸法から [`Viewport`] を作る。
//!
//! 主経路は [`crossterm::terminal::window_size`]。これは Unix では `TIOCGWINSZ` の
//! `ws_xpixel/ws_ypixel`（全テキスト領域 pixel）を読む。Knightty は PTY にこの pixel
//! 寸法を設定するため、`cols`/`rows` で割ればセル寸法が求まる。pixel が 0 の端末では
//! 控えめなデフォルトへフォールバックする（`CSI 16 t` 応答パーサは
//! [`crate::protocol::parse_cell_size_report`] に用意済みで、将来のライブ問い合わせ
//! 経路で使う）。

use std::io;

use paladocs_term::{CellSize, Viewport};

/// セル pixel 寸法が端末から取れないときの控えめなデフォルト。
pub const DEFAULT_CELL: CellSize = CellSize { w_px: 10, h_px: 20 };

/// 端末から現在の [`Viewport`]（cols/rows + セル pixel 寸法）を計測する。
pub fn measure_viewport() -> io::Result<Viewport> {
    let ws = crossterm::terminal::window_size()?;
    Ok(viewport_from_window(
        ws.columns, ws.rows, ws.width, ws.height,
    ))
}

/// `window_size` の生値から [`Viewport`] を組む純関数（テスト可能）。
///
/// `cols`/`rows` は最低 1 にクランプ。pixel 寸法が非ゼロならセル寸法は整数除算で求め、
/// 1 未満は 1 にクランプ。pixel が 0 のときは [`DEFAULT_CELL`]。
fn viewport_from_window(cols: u16, rows: u16, width_px: u16, height_px: u16) -> Viewport {
    let cols = (cols.max(1)) as u32;
    let rows = (rows.max(1)) as u32;
    let cell = if width_px > 0 && height_px > 0 {
        CellSize {
            w_px: (width_px as u32 / cols).max(1),
            h_px: (height_px as u32 / rows).max(1),
        }
    } else {
        DEFAULT_CELL
    };
    Viewport { cols, rows, cell }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_cell_size_by_division() {
        let vp = viewport_from_window(80, 24, 800, 480);
        assert_eq!(vp.cols, 80);
        assert_eq!(vp.rows, 24);
        assert_eq!(vp.cell, CellSize { w_px: 10, h_px: 20 });
    }

    #[test]
    fn falls_back_to_default_when_pixels_zero() {
        let vp = viewport_from_window(80, 24, 0, 0);
        assert_eq!(vp.cell, DEFAULT_CELL);
    }

    #[test]
    fn clamps_zero_cols_rows_to_one() {
        let vp = viewport_from_window(0, 0, 0, 0);
        assert_eq!(vp.cols, 1);
        assert_eq!(vp.rows, 1);
    }

    #[test]
    fn clamps_subcell_pixels_to_one() {
        // pixel < cols/rows → 0 になるところを 1 にクランプ。
        let vp = viewport_from_window(100, 100, 50, 50);
        assert_eq!(vp.cell, CellSize { w_px: 1, h_px: 1 });
    }
}
