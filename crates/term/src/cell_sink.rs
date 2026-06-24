//! ANSI セルバックエンド（[`CellGrid`] → エスケープ列）。
//!
//! 画像プロトコル経路（[`Presenter`](crate::Presenter)/[`KittyBackend`](crate::KittyBackend)）
//! とは独立した**別出口**。`render` のセル層（[`CellGrid`]/[`Cell`]/[`CellAttrs`]/
//! [`Color`]/[`CellWidth`]/[`CellRun`]/[`changed_runs`]）を truecolor の ANSI テキストへ
//! 落とす。capability 検出・256 色フォールバック・出口モード選択は `cli`（step 5）の責務で
//! スコープ外。本モジュールは truecolor を出す。
//!
//! # SGR ステート機械（差分最小化）
//!
//! ラン内を左→右に走査し、各セルで望む状態 `(fg, bg, attrs)` を現在状態と比較して**差分の
//! み**発行する:
//!
//! - **追加のみ**（attrs が現在のスーパーセット、fg/bg 変化のみ）→ 変化した SGR コードだけ。
//! - **属性の除去がある** → `\x1b[0m`（全リセット）後に fg+bg+attrs をフル再構築。`22` が
//!   bold/dim を両方落とす等、個別 off コードが曖昧なため除去時はリセットが安全。
//!
//! フレーム先頭・末尾で `\x1b[0m` を発行し状態をフレーム間にリークさせない（不変条件 1）。

use paladocs_render::{Cell, CellAttrs, CellGrid, CellRun, CellWidth, Color, changed_runs};
use std::io::{self, Write};

/// `(BOLD, DIM, ITALIC, UNDERLINE, REVERSE)` → SGR コードの対応表。
const ATTR_CODES: [(CellAttrs, u8); 5] = [
    (CellAttrs::BOLD, 1),
    (CellAttrs::DIM, 2),
    (CellAttrs::ITALIC, 3),
    (CellAttrs::UNDERLINE, 4),
    (CellAttrs::REVERSE, 7),
];

/// [`CellGrid`] を ANSI エスケープ列として `W` へ書くステートフルな sink。
///
/// `cur` は直近に確定した SGR 状態。`None` は「リセット直後（端末既定）」を表し、次セルは
/// フル SGR で構築する。alt-screen の出入りは [`enter`](Self::enter)/[`leave`](Self::leave)
/// が対で行い、呼び出し側が panic 経路含め [`leave`](Self::leave) を保証する（不変条件 5）。
pub struct CellSink<W: Write> {
    w: W,
    cur: Option<(Color, Color, CellAttrs)>,
}

impl<W: Write> CellSink<W> {
    /// `writer` を包んだ未初期化（リセット状態）の sink。
    pub fn new(w: W) -> Self {
        Self { w, cur: None }
    }

    /// 内側の writer への参照（検査・テスト用）。
    pub fn writer(&self) -> &W {
        &self.w
    }

    /// 代替画面へ入りカーソルを隠す（`\x1b[?1049h\x1b[?25l`）。
    pub fn enter(&mut self) -> io::Result<()> {
        self.w.write_all(b"\x1b[?1049h\x1b[?25l")
    }

    /// カーソルを戻し代替画面から復帰（`\x1b[?25h\x1b[?1049l\x1b[0m`）。
    pub fn leave(&mut self) -> io::Result<()> {
        self.cur = None;
        self.w.write_all(b"\x1b[?25h\x1b[?1049l\x1b[0m")
    }

    /// `grid` 全体を `origin`（1-indexed、既定 `(1, 1)`）基準で全描画する。
    ///
    /// 各行を全幅 1 ラン（`col: 0, len: cols`）として発行する。先頭・末尾で `\x1b[0m`。
    pub fn draw_full(&mut self, grid: &CellGrid, origin: (u16, u16)) -> io::Result<()> {
        let (cols, rows) = grid.dims();
        if cols == 0 || rows == 0 {
            return Ok(());
        }
        self.reset()?;
        for row in 0..rows {
            let run = CellRun {
                row,
                col: 0,
                len: cols,
            };
            self.emit_run(grid, run, origin)?;
        }
        self.reset()
    }

    /// `old` → `new` の差分（[`changed_runs`]）だけを `origin` 基準で再発行する。
    ///
    /// 変化が無ければ**何も書かない**（出力空）。触る位置は変更ランの範囲内に限る
    /// （不変条件 3）。dims 不一致時 [`changed_runs`] は full repaint を返すため全描画になる。
    pub fn draw_diff(
        &mut self,
        old: &CellGrid,
        new: &CellGrid,
        origin: (u16, u16),
    ) -> io::Result<()> {
        let runs = changed_runs(old, new);
        if runs.is_empty() {
            return Ok(());
        }
        self.reset()?;
        for run in runs {
            self.emit_run(new, run, origin)?;
        }
        self.reset()
    }

    /// `\x1b[0m` を発行し SGR 状態をリセット（`cur = None`）。
    fn reset(&mut self) -> io::Result<()> {
        self.cur = None;
        self.w.write_all(b"\x1b[0m")
    }

    /// 1 ラン分: 先頭で CUP、ラン内を左→右に SGR 差分＋グリフ発行。
    fn emit_run(&mut self, grid: &CellGrid, run: CellRun, origin: (u16, u16)) -> io::Result<()> {
        let (cols, rows) = grid.dims();
        if run.row >= rows {
            return Ok(());
        }
        // CUP（1-indexed）: origin + ラン先頭セル。
        let cur_row = origin.0 as u32 + run.row as u32;
        let cur_col = origin.1 as u32 + run.col as u32;
        write!(self.w, "\x1b[{cur_row};{cur_col}H")?;

        let end = (run.col as u32 + run.len as u32).min(cols as u32) as u16;
        for col in run.col..end {
            let cell = grid.get(col, run.row);
            // Continuation は左 Wide が消費済み。glyph もカーソル移動も出さない。
            if cell.width == CellWidth::Continuation {
                continue;
            }
            self.sync_sgr(cell)?;
            write!(self.w, "{}", cell.ch)?;
        }
        Ok(())
    }

    /// セルの望む `(fg, bg, attrs)` へ SGR 状態を最小差分で遷移する。
    fn sync_sgr(&mut self, cell: &Cell) -> io::Result<()> {
        let want = (cell.fg, cell.bg, cell.attrs);
        match self.cur {
            // リセット直後 → フル構築。
            None => {
                self.write_full_sgr(want.0, want.1, want.2)?;
            }
            Some((cf, cb, ca)) => {
                // 属性除去（現在 attrs が望む attrs のサブセットでない）→ リセット＋再構築。
                if !want.2.contains(ca) {
                    self.w.write_all(b"\x1b[0m")?;
                    self.write_full_sgr(want.0, want.1, want.2)?;
                } else {
                    // 追加のみ: 変化した fg/bg と新規 attrs ビットだけ発行。
                    let mut params: Vec<String> = Vec::new();
                    for (bit, code) in ATTR_CODES {
                        if want.2.contains(bit) && !ca.contains(bit) {
                            params.push(code.to_string());
                        }
                    }
                    if want.0 != cf {
                        params.push(fg_param(want.0));
                    }
                    if want.1 != cb {
                        params.push(bg_param(want.1));
                    }
                    if !params.is_empty() {
                        write!(self.w, "\x1b[{}m", params.join(";"))?;
                    }
                }
            }
        }
        self.cur = Some(want);
        Ok(())
    }

    /// attrs→fg→bg を 1 つの `\x1b[…m` でフル発行する（リセット直後／除去後）。
    fn write_full_sgr(&mut self, fg: Color, bg: Color, attrs: CellAttrs) -> io::Result<()> {
        let mut params: Vec<String> = Vec::new();
        for (bit, code) in ATTR_CODES {
            if attrs.contains(bit) {
                params.push(code.to_string());
            }
        }
        params.push(fg_param(fg));
        params.push(bg_param(bg));
        write!(self.w, "\x1b[{}m", params.join(";"))
    }
}

/// 前景 SGR。不透明（`a == 255`）は `38;2;r;g;b`（truecolor）、端末既定
/// （[`paladocs_render::DEFAULT`]、`a == 0`）は `39`（端末既定前景）。
fn fg_param(c: Color) -> String {
    if c[3] == 0 {
        "39".to_string()
    } else {
        format!("38;2;{};{};{}", c[0], c[1], c[2])
    }
}

/// 背景 SGR。不透明（`a == 255`）は `48;2;r;g;b`（truecolor）、端末既定
/// （[`paladocs_render::DEFAULT`]、`a == 0`）は `49`（端末既定背景＝透過）。
fn bg_param(c: Color) -> String {
    if c[3] == 0 {
        "49".to_string()
    } else {
        format!("48;2;{};{};{}", c[0], c[1], c[2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paladocs_render::CellWidth;

    const BLACK: Color = [0, 0, 0, 255];
    const WHITE: Color = [255, 255, 255, 255];

    fn out(sink: CellSink<Vec<u8>>) -> String {
        String::from_utf8(sink.writer().clone()).unwrap()
    }

    fn narrow(ch: char, fg: Color, bg: Color, attrs: CellAttrs) -> Cell {
        Cell {
            ch,
            fg,
            bg,
            width: CellWidth::Narrow,
            attrs,
        }
    }

    #[test]
    fn lifecycle_enter_leave_alt_screen_pair() {
        let mut sink = CellSink::new(Vec::new());
        sink.enter().unwrap();
        sink.leave().unwrap();
        let s = out(sink);
        let h = s.find("\x1b[?1049h").expect("enter alt-screen");
        let l = s.find("\x1b[?1049l").expect("leave alt-screen");
        assert!(h < l, "?1049h must precede ?1049l");
        assert!(s.contains("\x1b[?25l")); // hide
        assert!(s.contains("\x1b[?25h")); // show
    }

    #[test]
    fn solid_grid_emits_color_once() {
        // 単色 grid → 先頭で fg/bg SGR 1 回、以降同一セルは再発行なし。
        let mut grid = CellGrid::new_blank(3, 2, BLACK);
        for row in 0..2 {
            for col in 0..3 {
                grid.set(col, row, narrow(' ', WHITE, BLACK, CellAttrs::NONE));
            }
        }
        let mut sink = CellSink::new(Vec::new());
        sink.draw_full(&grid, (1, 1)).unwrap();
        let s = out(sink);
        // fg(38;2) は全 6 セルで 1 回だけ。
        assert_eq!(s.matches("38;2;255;255;255").count(), 1);
        assert_eq!(s.matches("48;2;0;0;0").count(), 1);
        // 先頭・末尾でリセット。
        assert!(s.starts_with("\x1b[0m"));
        assert!(s.ends_with("\x1b[0m"));
    }

    #[test]
    fn bold_run_emits_then_resets_on_removal() {
        // [bold, bold, plain] → `1m` を含み、非 BOLD への遷移でリセット＋再構築。
        let mut grid = CellGrid::new_blank(3, 1, BLACK);
        grid.set(0, 0, narrow('A', WHITE, BLACK, CellAttrs::BOLD));
        grid.set(1, 0, narrow('B', WHITE, BLACK, CellAttrs::BOLD));
        grid.set(2, 0, narrow('C', WHITE, BLACK, CellAttrs::NONE));
        let mut sink = CellSink::new(Vec::new());
        sink.draw_full(&grid, (1, 1)).unwrap();
        let s = out(sink);
        assert!(s.contains("\x1b[1;38;2;255;255;255;48;2;0;0;0m")); // 先頭フル（bold 込み）
        assert!(s.contains('A') && s.contains('B') && s.contains('C'));
        // BOLD 除去で C の前に 0m リセット＋再構築（先頭/末尾以外にもう 1 つ 0m）。
        assert!(s.matches("\x1b[0m").count() >= 3);
    }

    #[test]
    fn all_plain_grid_has_no_bold_code() {
        // 判別: 全非 BOLD grid は `1m` を含まない。
        let mut grid = CellGrid::new_blank(2, 1, BLACK);
        grid.set(0, 0, narrow('a', WHITE, BLACK, CellAttrs::NONE));
        grid.set(1, 0, narrow('b', WHITE, BLACK, CellAttrs::NONE));
        let mut sink = CellSink::new(Vec::new());
        sink.draw_full(&grid, (1, 1)).unwrap();
        let s = out(sink);
        assert!(!s.contains("\x1b[1m"));
        assert!(!s.contains(";1;")); // 連結中にも bold コード無し
    }

    #[test]
    fn default_colors_emit_terminal_default_sgr() {
        use paladocs_render::DEFAULT;
        // TUI 投影セル: 前景 DEFAULT・背景 DEFAULT → `39`/`49`（truecolor を出さない）。
        let mut grid = CellGrid::new_blank(1, 1, DEFAULT);
        grid.set(0, 0, narrow('A', DEFAULT, DEFAULT, CellAttrs::NONE));
        let mut sink = CellSink::new(Vec::new());
        sink.draw_full(&grid, (1, 1)).unwrap();
        let s = out(sink);
        assert!(s.contains("39"), "default fg must emit 39: {s:?}");
        assert!(s.contains("49"), "default bg must emit 49: {s:?}");
        assert!(!s.contains("38;2;"), "must not emit truecolor fg: {s:?}");
        assert!(!s.contains("48;2;"), "must not emit truecolor bg: {s:?}");
    }

    #[test]
    fn diff_no_change_is_empty() {
        let grid = CellGrid::new_blank(4, 2, BLACK);
        let mut sink = CellSink::new(Vec::new());
        sink.draw_diff(&grid, &grid, (1, 1)).unwrap();
        assert!(out(sink).is_empty());
    }

    #[test]
    fn diff_single_cell_one_cup_only_that_cell() {
        let old = CellGrid::new_blank(4, 2, BLACK);
        let mut new = old.clone();
        new.set(2, 1, narrow('x', WHITE, BLACK, CellAttrs::NONE));
        let mut sink = CellSink::new(Vec::new());
        sink.draw_diff(&old, &new, (1, 1)).unwrap();
        let s = out(sink);
        // CUP は 1 回（origin(1,1)+ (col2,row1) = 2;3）。他行に触れない。
        assert_eq!(s.matches("\x1b[").count() - s.matches("\x1b[0m").count(), 2);
        // ↑ CUP 1 + SGR 1（= ESC[ 総数からリセット 2 を引いた残り）。
        assert!(s.contains("\x1b[2;3H"));
        assert!(s.contains('x'));
        // 1 行目(row0)の CUP は出ない。
        assert!(!s.contains("\x1b[1;"));
    }

    #[test]
    fn wide_emits_glyph_once_skips_continuation() {
        // 全角 1 文字 → グリフ 1 回、Continuation 列へ移動も glyph も出さない。
        let mut grid = CellGrid::new_blank(3, 1, BLACK);
        grid.set(
            0,
            0,
            Cell {
                ch: '漢',
                fg: WHITE,
                bg: BLACK,
                width: CellWidth::Wide,
                attrs: CellAttrs::NONE,
            },
        );
        grid.set(
            1,
            0,
            Cell {
                ch: ' ',
                fg: WHITE,
                bg: BLACK,
                width: CellWidth::Continuation,
                attrs: CellAttrs::NONE,
            },
        );
        grid.set(2, 0, narrow('z', WHITE, BLACK, CellAttrs::NONE));
        let mut sink = CellSink::new(Vec::new());
        sink.draw_full(&grid, (1, 1)).unwrap();
        let s = out(sink);
        assert_eq!(s.matches('漢').count(), 1);
        // Continuation の空白は出さない（行内の ' ' は 1 つも無い）。
        assert!(!s.contains(' '));
        assert!(s.contains('z'));
    }

    #[test]
    fn origin_offsets_cup() {
        // letterbox: origin(3,5) → row0col0 の CUP は 3;5。
        let mut grid = CellGrid::new_blank(1, 1, BLACK);
        grid.set(0, 0, narrow('q', WHITE, BLACK, CellAttrs::NONE));
        let mut sink = CellSink::new(Vec::new());
        sink.draw_full(&grid, (3, 5)).unwrap();
        assert!(out(sink).contains("\x1b[3;5H"));
    }
}
