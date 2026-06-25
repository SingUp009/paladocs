//! セル空間プリミティブ（MDPT 方式の Typst 非依存部分）。
//!
//! `render` の純粋ピクセル層に、**セル（端末文字セル）空間**の型と操作を追加する。
//! ここで確定するのは Typst に依存しない純粋部分のみ:
//!
//! 1. セル空間プリミティブ [`CellWidth`] / [`Cell`] / [`CellGrid`]。
//! 2. raster→cell 量子化（半ブロック、approach A）[`quantize_half_block`] /
//!    [`quantize_half_block_into`]。
//! 3. cell-diff [`changed_runs`]（[`changed_region`](crate::changed_region) のセル版）。
//!
//! 構造化テキスト射影・CJK advance 消費は `paladocs-typst`、エスケープ列生成は
//! `paladocs-term` の責務であり、本モジュールのスコープ外。

use super::{Frame, Rect};
use std::ops::{BitAnd, BitOr, BitOrAssign};

/// セルのテキスト属性ビットフラグ（bold/dim/italic/underline/reverse）。
///
/// 依存ゼロ原則のため `bitflags` 等の外部 crate は使わず、自前の `u8` newtype で
/// 表現する。各ビットは下の定数で定義し、合成は [`BitOr`]/[`BitOrAssign`]、交差判定は
/// [`CellAttrs::contains`] で行う。
///
/// 既定は [`CellAttrs::NONE`]（属性なし）。画像由来セル（量子化出力）は常に `NONE`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellAttrs(u8);

impl CellAttrs {
    /// 属性なし（既定）。
    pub const NONE: Self = Self(0);
    /// 太字。
    pub const BOLD: Self = Self(1 << 0);
    /// 減光（faint）。
    pub const DIM: Self = Self(1 << 1);
    /// イタリック。
    pub const ITALIC: Self = Self(1 << 2);
    /// 下線。
    pub const UNDERLINE: Self = Self(1 << 3);
    /// 前景／背景反転。
    pub const REVERSE: Self = Self(1 << 4);

    /// `f` の全ビットを含むか。
    pub fn contains(self, f: Self) -> bool {
        self.0 & f.0 == f.0
    }

    /// `f` のビットを立てる。
    pub fn insert(&mut self, f: Self) {
        self.0 |= f.0;
    }

    /// `f` のビットを落とす。
    pub fn remove(&mut self, f: Self) {
        self.0 &= !f.0;
    }
}

impl BitOr for CellAttrs {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitAnd for CellAttrs {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl BitOrAssign for CellAttrs {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// セルの前景／背景に使う **RGBA 色値**（`[r, g, b, a]`、sRGB バイト空間）。
///
/// 画像バッファ型 [`Rgba`](crate::Rgba) とは別物で、こちらは「1 ピクセル分の色」を
/// 表す。アルファは「端末への色指定の種別」を表す:
/// - `a == 255`: 不透明な truecolor（`r, g, b` をそのまま指定）。量子化（mosaic）由来の
///   セルは常にこれ。
/// - `a == 0`: **端末既定色**（[`DEFAULT`]）。`term` は truecolor を発行せず端末の既定
///   前景／背景（SGR `39`/`49`）を使う。TUI 投影の地色・本文色がこれ。
///
/// 中間アルファは使わない（合成は量子化前に完了している前提）。
pub type Color = [u8; 4];

/// 端末既定色を表す番兵（アルファ 0）。
///
/// このセル色を持つ前景／背景は、`term` 側で truecolor ではなく端末の**既定色**
/// （SGR `39` 前景 / `49` 背景）として発行される。背景に使えば地色が端末テーマに
/// 追従して透過し、前景に使えば文字色が端末テーマに追従する（TUI 投影で使用）。
pub const DEFAULT: Color = [0, 0, 0, 0];

/// セルが占有する端末カラム数の種別。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellWidth {
    /// 1 カラム。量子化 (approach A) は常にこれのみ生成する。
    Narrow,
    /// 2 カラム占有（CJK 等）。グリフは左カラムに置く。
    ///
    /// `paladocs-typst` のテキスト射影でのみ生成される。型契約として右隣は必ず
    /// [`CellWidth::Continuation`] となる（不変条件 4）。
    Wide,
    /// [`CellWidth::Wide`] の右隣に置く描画しない番兵。`term` はこのセルをスキップする。
    ///
    /// 型契約として左隣は必ず [`CellWidth::Wide`] となる（不変条件 4）。
    Continuation,
}

/// 端末文字セル 1 個。
///
/// 色は不透明 truecolor（`a == 255`）または端末既定色（[`DEFAULT`]、`a == 0`）の
/// いずれか。中間アルファは持たない（合成は量子化前に完了している前提）。量子化
/// （mosaic）由来のセルは常に不透明、TUI 投影由来のセルは前景／背景に [`DEFAULT`] を
/// 用いて端末テーマへ追従する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// 表示文字。
    // v1 は `char` 単一。合字・結合文字クラスタ対応のため将来 `SmallVec<char>` 化の
    // 余地あり。今はやらない。
    pub ch: char,
    /// 前景色（不透明、`fg[3] == 255`）。
    pub fg: Color,
    /// 背景色（不透明、`bg[3] == 255`）。
    pub bg: Color,
    /// カラム占有種別。
    pub width: CellWidth,
    /// テキスト属性（bold/underline 等）。既定は [`CellAttrs::NONE`]。画像由来セル
    /// （量子化出力）は常に `NONE`。
    pub attrs: CellAttrs,
}

impl Cell {
    /// 空白セル `{ ch: ' ', fg, bg, width: Narrow, attrs: NONE }`。
    pub fn blank(fg: Color, bg: Color) -> Self {
        Self {
            ch: ' ',
            fg,
            bg,
            width: CellWidth::Narrow,
            attrs: CellAttrs::NONE,
        }
    }

    /// 端末既定の前景／背景（[`DEFAULT`]）の空白セル。TUI 投影の地セルに使う。
    pub fn transparent() -> Self {
        Self::blank(DEFAULT, DEFAULT)
    }

    /// 不透明（`fg[3] == 255 && bg[3] == 255`）か。
    fn is_opaque(&self) -> bool {
        self.fg[3] == 255 && self.bg[3] == 255
    }
}

/// row-major のセル格子。
///
/// 不変条件 1: `cells.len() == cols * rows`。フィールドは非公開で、コンストラクタと
/// [`CellGrid::set`]（範囲外無視）がこの不変条件を保証する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellGrid {
    cols: u16,
    rows: u16,
    // 不変条件: cells.len() == cols as usize * rows as usize
    cells: Vec<Cell>,
}

impl CellGrid {
    /// 全セルを背景 `bg` の空白セルで埋めた `cols × rows` の格子。
    pub fn new_blank(cols: u16, rows: u16, bg: Color) -> Self {
        let len = cols as usize * rows as usize;
        Self {
            cols,
            rows,
            cells: vec![Cell::blank(bg, bg); len],
        }
    }

    /// `(cols, rows)`。
    pub fn dims(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// `(col, row)` のセル参照。
    ///
    /// 範囲内であることが前提（呼び出し側が `dims()` で保証する）。範囲外は
    /// `debug_assert!` で検出し、release では `(0, 0)` にクランプしてパニックを避ける。
    pub fn get(&self, col: u16, row: u16) -> &Cell {
        debug_assert!(
            col < self.cols && row < self.rows,
            "CellGrid::get out of range: ({col}, {row}) dims ({}, {})",
            self.cols,
            self.rows
        );
        let (c, r) = if col < self.cols && row < self.rows {
            (col, row)
        } else {
            (0, 0)
        };
        &self.cells[r as usize * self.cols as usize + c as usize]
    }

    /// `(col, row)` にセルを書く。範囲外は無視する（不変条件 1 を保つ）。
    pub fn set(&mut self, col: u16, row: u16, cell: Cell) {
        if col >= self.cols || row >= self.rows {
            return;
        }
        self.cells[row as usize * self.cols as usize + col as usize] = cell;
    }

    /// 全セルを `cell` で埋める。
    pub fn fill(&mut self, cell: Cell) {
        for c in &mut self.cells {
            *c = cell.clone();
        }
    }

    /// 上から順に各行のセルスライスを返す。
    pub fn rows(&self) -> impl Iterator<Item = &[Cell]> {
        self.cells.chunks(self.cols as usize)
    }
}

/// `src`（straight-alpha）を不透明 `bg` の上に source-over 合成して不透明 RGB を返す。
///
/// `bg` が不透明前提なので結果も不透明。`composite` と同じ混色規則。
fn over_opaque(src: Color, bg: Color) -> [u8; 3] {
    let sa = src[3];
    if sa == 255 {
        return [src[0], src[1], src[2]];
    }
    if sa == 0 {
        return [bg[0], bg[1], bg[2]];
    }
    let inv = 255 - sa;
    // bg は不透明 (a=255) 前提: da_contrib = mul255(255, inv) = inv、out_a = 255。
    let da_contrib = inv;
    let mut out = [0u8; 3];
    for c in 0..3 {
        let num = src[c] as u32 * sa as u32 + bg[c] as u32 * da_contrib as u32;
        out[c] = (num / 255) as u8;
    }
    out
}

/// frame を半ブロック量子化して `cols × rows` の [`CellGrid`] を新規生成する。
///
/// approach A: 各セル = 横 1 カラム・縦 2 サブピクセルとし、`U+2580 ▀`（UPPER HALF
/// BLOCK）で **fg = 上サブピクセル / bg = 下サブピクセル**を表現する。サンプル格子は
/// `cols × (rows*2)` で、各サブセルは frame の対応領域の **box 平均**（nearest 不可）。
/// frame のアルファは `bg` へ合成してから量子化するため、出力セルは常に不透明。
///
/// **アスペクト比調整はしない。** 呼び出し側が事前に [`fit`](crate::fit) 等で frame を
/// セル比（おおむね 1 セル = 1:2）へ整えてから渡すこと（責務分離）。
pub fn quantize_half_block(frame: &Frame, cols: u16, rows: u16, bg: Color) -> CellGrid {
    let mut grid = CellGrid::new_blank(cols, rows, bg);
    let dst = Rect {
        x: 0,
        y: 0,
        w: cols as u32,
        h: rows as u32,
    };
    quantize_half_block_into(&mut grid, dst, frame, bg);
    grid
}

/// frame を半ブロック量子化し、`grid` の **`dst`（セル空間矩形）領域だけ**へ焼き込む。
///
/// (B) 用: 既にテキストセルが置かれた `grid` の Shape/Image 領域だけをラスタで上書き
/// する。`dst` の外、および `grid` 範囲外へはみ出すセルは触らない。挙動の詳細は
/// [`quantize_half_block`] を参照。
pub fn quantize_half_block_into(grid: &mut CellGrid, dst: Rect, frame: &Frame, bg: Color) {
    let cols_s = dst.w; // サンプル格子の横 = dst 幅（カラム数）
    let rows_s = dst.h * 2; // サンプル格子の縦 = dst 高さ * 2（上下サブセル）
    if cols_s == 0 || rows_s == 0 {
        return;
    }

    let size = frame.image.size();
    let img_w = size.w;
    let img_h = size.h;

    // サブセル (sc, sr) の box 平均（不透明 RGB）。
    let sample = |sc: u32, sr: u32| -> Color {
        let (x0, x1) = box_range(sc, cols_s, img_w);
        let (y0, y1) = box_range(sr, rows_s, img_h);
        let mut acc = [0u64; 3];
        let mut n = 0u64;
        for y in y0..y1 {
            for x in x0..x1 {
                // frame は範囲内のみ走査するので pixel は必ず Some。
                if let Some(px) = frame.image.pixel(x, y) {
                    let rgb = over_opaque(px, bg);
                    acc[0] += rgb[0] as u64;
                    acc[1] += rgb[1] as u64;
                    acc[2] += rgb[2] as u64;
                    n += 1;
                }
            }
        }
        if n == 0 {
            // frame がゼロサイズ等で被覆ピクセル無し → bg をそのまま採用。
            return bg;
        }
        [
            (acc[0] / n) as u8,
            (acc[1] / n) as u8,
            (acc[2] / n) as u8,
            255,
        ]
    };

    for j in 0..dst.h {
        let row = dst.y + j;
        for i in 0..dst.w {
            let col = dst.x + i;
            let fg3 = sample(i, j * 2);
            let bg3 = sample(i, j * 2 + 1);
            let cell = Cell {
                ch: '\u{2580}', // ▀ UPPER HALF BLOCK
                fg: [fg3[0], fg3[1], fg3[2], 255],
                bg: [bg3[0], bg3[1], bg3[2], 255],
                width: CellWidth::Narrow,
                attrs: CellAttrs::NONE, // 画像セルに属性は無い
            };
            debug_assert!(cell.is_opaque(), "quantize must produce opaque cells");
            // col/row は u32 だが grid は u16。範囲外は set 側でクリップ。
            if col <= u16::MAX as u32 && row <= u16::MAX as u32 {
                grid.set(col as u16, row as u16, cell);
            }
        }
    }
}

/// サンプル軸インデックス `idx`（0..`samples`）が被覆する画素範囲 `[lo, hi)`。
///
/// box 境界 = `floor(idx * dim / samples) .. floor((idx+1) * dim / samples)`。
/// アップサンプル等で範囲が空になる場合は中心画素 1 個へフォールバックし、必ず
/// `hi > lo`（≥1 画素）を保証する（`dim == 0` を除く）。
fn box_range(idx: u32, samples: u32, dim: u32) -> (u32, u32) {
    if dim == 0 {
        return (0, 0);
    }
    let lo = (idx as u64 * dim as u64 / samples as u64) as u32;
    let hi = ((idx as u64 + 1) * dim as u64 / samples as u64) as u32;
    if hi > lo {
        (lo, hi)
    } else {
        // 空 → 中心画素。lo は dim 未満が保証される（idx < samples）。
        let c = lo.min(dim - 1);
        (c, c + 1)
    }
}

/// セル span（複数セルへ拡大表示する 1 まとまりのテキスト）。
///
/// 端末（Knightty）が `cols × rows` のセル矩形へ `text`（1 つ以上の書記素）を拡大して
/// 描く指示。論理グリッドのセル寸法は等幅のまま、字形だけが矩形へスケールされる。
/// `render` は純粋なデータ型として保持するだけで、ワイヤ生成（OSC 7777）は `term`、
/// 見出し→span の判定は `typst` の責務。
///
/// 不変条件（生成側が保証）: `cols >= 1`・`rows >= 1`、矩形はグリッド内に収まる、
/// `text` は制御文字を含まない非空 UTF-8。`attrs` は端末が継承する強調属性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellSpan {
    /// 矩形左上のカラム（0 始まり）。
    pub col: u16,
    /// 矩形左上の行（0 始まり）。
    pub row: u16,
    /// 矩形のカラム数（`>= 1`）。
    pub cols: u16,
    /// 矩形の行数（`>= 1`）。
    pub rows: u16,
    /// 拡大表示するテキスト（制御文字なし・非空）。
    pub text: String,
    /// 端末が継承する強調属性（bold 等）。
    pub attrs: CellAttrs,
}

/// 罫線（box-drawing）コーナースタイル。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxStyle {
    /// 直角コーナー `┌┐└┘`。
    #[default]
    Square,
    /// 丸コーナー `╭╮╰╯`。
    Rounded,
}

impl BoxStyle {
    /// `(top_left, top_right, bottom_left, bottom_right)` のコーナー文字。
    fn corners(self) -> (char, char, char, char) {
        match self {
            BoxStyle::Square => ('┌', '┐', '└', '┘'),
            BoxStyle::Rounded => ('╭', '╮', '╰', '╯'),
        }
    }
}

/// 横罫線文字 `─`。
const H_LINE: char = '─';
/// 縦罫線文字 `│`。
const V_LINE: char = '│';

/// 罫線セル 1 個を置く（背景は [`DEFAULT`]、幅 [`CellWidth::Narrow`]）。
///
/// `u32` 座標は `u16` 範囲内のみ [`CellGrid::set`] へ渡す（範囲外は無視）。
fn set_line_cell(grid: &mut CellGrid, col: u32, row: u32, ch: char, fg: Color, attrs: CellAttrs) {
    if col > u16::MAX as u32 || row > u16::MAX as u32 {
        return;
    }
    grid.set(
        col as u16,
        row as u16,
        Cell {
            ch,
            fg,
            bg: DEFAULT,
            width: CellWidth::Narrow,
            attrs,
        },
    );
}

/// `row` の `[col, col+len)` に横罫線 `─` を引く。grid 範囲外はクリップ。
pub fn draw_hline(grid: &mut CellGrid, row: u32, col: u32, len: u32, fg: Color, attrs: CellAttrs) {
    for i in 0..len {
        set_line_cell(grid, col.saturating_add(i), row, H_LINE, fg, attrs);
    }
}

/// `col` の `[row, row+len)` に縦罫線 `│` を引く。grid 範囲外はクリップ。
pub fn draw_vline(grid: &mut CellGrid, col: u32, row: u32, len: u32, fg: Color, attrs: CellAttrs) {
    for i in 0..len {
        set_line_cell(grid, col, row.saturating_add(i), V_LINE, fg, attrs);
    }
}

/// `rect`（セル空間）の**外周だけ**を罫線で `grid` に描く（塗りはしない）。
///
/// `fg` は罫線色（[`DEFAULT`] で端末既定前景）。罫線セルの背景は [`DEFAULT`]（透過）。
/// grid 範囲外はクリップしパニックしない。退化形:
/// - `w == 0 || h == 0` → 何もしない。
/// - `w == 1` → 縦 1 本（[`draw_vline`]）。`h == 1` → 横 1 本（[`draw_hline`]）。
pub fn draw_box(grid: &mut CellGrid, rect: Rect, fg: Color, attrs: CellAttrs, style: BoxStyle) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    if rect.w == 1 {
        draw_vline(grid, rect.x, rect.y, rect.h, fg, attrs);
        return;
    }
    if rect.h == 1 {
        draw_hline(grid, rect.y, rect.x, rect.w, fg, attrs);
        return;
    }
    let (tl, tr, bl, br) = style.corners();
    let x0 = rect.x;
    let x1 = rect.x.saturating_add(rect.w - 1);
    let y0 = rect.y;
    let y1 = rect.y.saturating_add(rect.h - 1);
    // 上下辺（コーナーの内側だけ横線）。
    draw_hline(grid, y0, x0 + 1, rect.w - 2, fg, attrs);
    draw_hline(grid, y1, x0 + 1, rect.w - 2, fg, attrs);
    // 左右辺（コーナーの内側だけ縦線）。
    draw_vline(grid, x0, y0 + 1, rect.h - 2, fg, attrs);
    draw_vline(grid, x1, y0 + 1, rect.h - 2, fg, attrs);
    // コーナー。
    set_line_cell(grid, x0, y0, tl, fg, attrs);
    set_line_cell(grid, x1, y0, tr, fg, attrs);
    set_line_cell(grid, x0, y1, bl, fg, attrs);
    set_line_cell(grid, x1, y1, br, fg, attrs);
}

/// セルの水平ラン（同一行の連続変更領域）。term のカーソル移動 + SGR 発行単位に対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRun {
    /// 行（0 始まり）。
    pub row: u16,
    /// 開始カラム（0 始まり）。
    pub col: u16,
    /// セル数（`>= 1`）。
    pub len: u16,
}

/// `old` → `new` で変化したセルを水平ランへ coalesce して返す（[`changed_region`] の
/// セル版）。
///
/// - dims が異なる場合は **full repaint**: `new` の全行を `col: 0, len: cols` の run
///   として返す。
/// - 同 dims では変更セルを水平ランにまとめる。[`CellWidth::Wide`] が変化したらその
///   右隣 [`CellWidth::Continuation`] も同じ run に含め、`Continuation` が変化したら
///   左隣 `Wide` を含める（不変条件 4 の対を一体で扱う）。
/// - 各 run は単一行内・grid 内に収まる（不変条件 5）。
///
/// [`changed_region`]: crate::changed_region
pub fn changed_runs(old: &CellGrid, new: &CellGrid) -> Vec<CellRun> {
    let (nc, nr) = new.dims();
    if old.dims() != new.dims() {
        // full repaint: 全行を 1 run ずつ。
        if nc == 0 {
            return Vec::new();
        }
        return (0..nr)
            .map(|row| CellRun {
                row,
                col: 0,
                len: nc,
            })
            .collect();
    }

    let mut runs = Vec::new();
    let cols = nc as usize;
    for row in 0..nr {
        let base = row as usize * cols;
        // この行の変更フラグ（Wide/Continuation のペア拡張込み）。
        let mut changed = vec![false; cols];
        for (c, flag) in changed.iter_mut().enumerate() {
            *flag = old.cells[base + c] != new.cells[base + c];
        }
        // ペア拡張: 変更された Wide は右の Continuation を、変更された Continuation は
        // 左の Wide を巻き込む（new 側の構造で判定）。`changed` を書き換えるので検出時の
        // スナップショットを基準にする。
        let detected = changed.clone();
        for (c, &was) in detected.iter().enumerate() {
            if !was {
                continue;
            }
            match new.cells[base + c].width {
                CellWidth::Wide => {
                    if c + 1 < cols {
                        changed[c + 1] = true;
                    }
                }
                CellWidth::Continuation => {
                    if c > 0 {
                        changed[c - 1] = true;
                    }
                }
                CellWidth::Narrow => {}
            }
        }
        // 連続する true を水平ランへ coalesce。
        let mut c = 0;
        while c < cols {
            if !changed[c] {
                c += 1;
                continue;
            }
            let start = c;
            while c < cols && changed[c] {
                c += 1;
            }
            runs.push(CellRun {
                row,
                col: start as u16,
                len: (c - start) as u16,
            });
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameId, PixelSize, Rgba};

    const BLACK: Color = [0, 0, 0, 255];
    const WHITE: Color = [255, 255, 255, 255];

    fn frame(size: PixelSize, data: Vec<u8>) -> Frame {
        Frame {
            id: FrameId(0),
            image: Rgba::new(size, data).unwrap(),
        }
    }

    /// 単色 frame をその色で埋めて作る。
    fn solid_frame(w: u32, h: u32, rgba: [u8; 4]) -> Frame {
        let mut data = Vec::with_capacity((w * h) as usize * 4);
        for _ in 0..(w * h) {
            data.extend_from_slice(&rgba);
        }
        frame(PixelSize { w, h }, data)
    }

    // ---- 型 ----

    #[test]
    fn new_blank_dims_and_len() {
        let g = CellGrid::new_blank(4, 3, BLACK);
        assert_eq!(g.dims(), (4, 3));
        assert_eq!(g.cells.len(), 12);
        assert!(g.cells.iter().all(|c| *c == Cell::blank(BLACK, BLACK)));
    }

    #[test]
    fn get_set_roundtrip() {
        let mut g = CellGrid::new_blank(3, 2, BLACK);
        let cell = Cell {
            ch: 'x',
            fg: WHITE,
            bg: BLACK,
            width: CellWidth::Narrow,
            attrs: CellAttrs::NONE,
        };
        g.set(2, 1, cell.clone());
        assert_eq!(*g.get(2, 1), cell);
        assert_eq!(*g.get(0, 0), Cell::blank(BLACK, BLACK));
    }

    #[test]
    fn set_out_of_range_ignored() {
        let mut g = CellGrid::new_blank(2, 2, BLACK);
        g.set(5, 5, Cell::blank(WHITE, WHITE));
        // 不変条件 1 を破らず、内容も変わらない。
        assert_eq!(g.cells.len(), 4);
        assert!(g.cells.iter().all(|c| *c == Cell::blank(BLACK, BLACK)));
    }

    #[test]
    fn fill_replaces_all() {
        let mut g = CellGrid::new_blank(2, 2, BLACK);
        let cell = Cell {
            ch: '#',
            fg: WHITE,
            bg: BLACK,
            width: CellWidth::Narrow,
            attrs: CellAttrs::NONE,
        };
        g.fill(cell.clone());
        assert!(g.cells.iter().all(|c| *c == cell));
    }

    #[test]
    fn rows_yields_row_slices() {
        let mut g = CellGrid::new_blank(2, 3, BLACK);
        g.set(0, 0, Cell::blank(WHITE, WHITE));
        let rows: Vec<&[Cell]> = g.rows().collect();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.len() == 2));
        assert_eq!(rows[0][0], Cell::blank(WHITE, WHITE));
        assert_eq!(rows[0][1], Cell::blank(BLACK, BLACK));
    }

    // ---- CellAttrs ----

    #[test]
    fn cell_attrs_bit_ops() {
        let mut a = CellAttrs::BOLD | CellAttrs::UNDERLINE;
        assert!(a.contains(CellAttrs::BOLD));
        assert!(a.contains(CellAttrs::UNDERLINE));
        assert!(a.contains(CellAttrs::BOLD | CellAttrs::UNDERLINE));
        assert!(!a.contains(CellAttrs::REVERSE));
        a.remove(CellAttrs::BOLD);
        assert!(!a.contains(CellAttrs::BOLD));
        assert!(a.contains(CellAttrs::UNDERLINE));
        a.insert(CellAttrs::DIM);
        assert!(a.contains(CellAttrs::DIM));
        assert_eq!(CellAttrs::default(), CellAttrs::NONE);
    }

    // ---- 量子化 ----

    #[test]
    fn quantize_solid_uniform_opaque() {
        // 単色 → 全セル同色・▀・fg==bg・不透明（受け入れ条件 1）。
        let f = solid_frame(8, 8, [10, 20, 30, 255]);
        let g = quantize_half_block(&f, 4, 4, BLACK);
        assert_eq!(g.dims(), (4, 4)); // 不変条件 2
        for cell in &g.cells {
            assert_eq!(cell.ch, '\u{2580}');
            assert_eq!(cell.fg, [10, 20, 30, 255]);
            assert_eq!(cell.bg, [10, 20, 30, 255]);
            assert_eq!(cell.width, CellWidth::Narrow);
            assert_eq!(cell.attrs, CellAttrs::NONE); // 画像セルは常に NONE
            assert!(cell.fg[3] == 255 && cell.bg[3] == 255); // 不変条件 3
        }
    }

    #[test]
    fn quantize_vertical_gradient_fg_ne_bg() {
        // 縦グラデーション: 上半分 黒、下半分 白 → 1 セルで fg=黒, bg=白。
        // frame 4x2, cols=4 rows=1 → サンプル格子 4x2: 上行=y0, 下行=y1。
        let data = vec![
            // y=0: 黒
            0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, //
            // y=1: 白
            255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
        ];
        let f = frame(PixelSize { w: 4, h: 2 }, data);
        let g = quantize_half_block(&f, 4, 1, BLACK);
        for cell in &g.cells {
            assert_eq!(cell.fg, [0, 0, 0, 255]); // 上サブセル = 黒
            assert_eq!(cell.bg, [255, 255, 255, 255]); // 下サブセル = 白
            assert_ne!(cell.fg, cell.bg);
        }
    }

    #[test]
    fn quantize_box_average_not_nearest() {
        // frame 2x4, cols=1 rows=1 → サンプル格子 1x2。
        // 上サブセル = y∈[0,1) の box 平均、下 = y∈[1,2)... ではない。
        // box_range(0,2,4) = [0,2), box_range(1,2,4) = [2,4)。
        // 上 = rows 0,1 の平均、下 = rows 2,3 の平均。各行 2px。
        // rows: 0=[0], 1=[100], 2=[200], 3=[40] のグレースケール。
        let g0 = 0u8;
        let g1 = 100u8;
        let g2 = 200u8;
        let g3 = 40u8;
        let mk = |v: u8| [v, v, v, 255u8];
        let mut data = Vec::new();
        for v in [g0, g1, g2, g3] {
            // 各行 2px
            data.extend_from_slice(&mk(v));
            data.extend_from_slice(&mk(v));
        }
        let f = frame(PixelSize { w: 2, h: 4 }, data);
        let g = quantize_half_block(&f, 1, 1, BLACK);
        let cell = &g.cells[0];
        // 上 = (0+100)/2 = 50, 下 = (200+40)/2 = 120。nearest なら 0/200 等になる。
        assert_eq!(cell.fg, [50, 50, 50, 255]);
        assert_eq!(cell.bg, [120, 120, 120, 255]);
    }

    #[test]
    fn quantize_alpha_composited_over_bg() {
        // 半透明 frame は bg へ合成されてから不透明化される。
        // src=(200,200,200,128), bg=(40,40,40,255):
        //   out = (200*128 + 40*127) / 255 = 30680/255 = 120
        let f = solid_frame(2, 2, [200, 200, 200, 128]);
        let g = quantize_half_block(&f, 1, 1, [40, 40, 40, 255]);
        let cell = &g.cells[0];
        assert_eq!(cell.fg, [120, 120, 120, 255]);
        assert_eq!(cell.bg, [120, 120, 120, 255]);
    }

    #[test]
    fn quantize_into_writes_only_dst() {
        // 5x5 grid を白で塗り、中央 (1,1)..(2,2) の 2x2 だけ黒 frame で焼く。
        let mut g = CellGrid::new_blank(5, 5, WHITE);
        let f = solid_frame(4, 4, BLACK);
        let dst = Rect {
            x: 1,
            y: 1,
            w: 2,
            h: 2,
        };
        quantize_half_block_into(&mut g, dst, &f, BLACK);
        for row in 0..5u16 {
            for col in 0..5u16 {
                let c = g.get(col, row);
                if (1..=2).contains(&col) && (1..=2).contains(&row) {
                    assert_eq!(c.fg, BLACK, "dst 内 ({col},{row})");
                    assert_eq!(c.ch, '\u{2580}');
                } else {
                    assert_eq!(*c, Cell::blank(WHITE, WHITE), "dst 外 ({col},{row}) は不変");
                }
            }
        }
    }

    #[test]
    fn quantize_into_clips_overhang() {
        // dst が grid をはみ出す → グリッド外は触らずパニックしない。
        let mut g = CellGrid::new_blank(2, 2, WHITE);
        let f = solid_frame(4, 4, BLACK);
        let dst = Rect {
            x: 1,
            y: 1,
            w: 3,
            h: 3,
        };
        quantize_half_block_into(&mut g, dst, &f, BLACK);
        // (1,1) のみ書ける。
        assert_eq!(g.get(1, 1).fg, BLACK);
        assert_eq!(*g.get(0, 0), Cell::blank(WHITE, WHITE));
    }

    // ---- changed_runs ----

    fn narrow(ch: char) -> Cell {
        Cell {
            ch,
            fg: WHITE,
            bg: BLACK,
            width: CellWidth::Narrow,
            attrs: CellAttrs::NONE,
        }
    }

    #[test]
    fn changed_runs_identical_empty() {
        let g = CellGrid::new_blank(4, 2, BLACK);
        assert!(changed_runs(&g, &g).is_empty());
    }

    #[test]
    fn changed_runs_single_cell() {
        let old = CellGrid::new_blank(4, 2, BLACK);
        let mut new = old.clone();
        new.set(2, 1, narrow('x'));
        assert_eq!(
            changed_runs(&old, &new),
            vec![CellRun {
                row: 1,
                col: 2,
                len: 1
            }]
        );
    }

    #[test]
    fn changed_runs_detects_attrs_only_change() {
        // attrs だけ異なる 2 grid → 当該セルが run に出る（等価判定に attrs を含む）。
        let old = CellGrid::new_blank(4, 1, BLACK);
        let mut new = old.clone();
        let mut bold = narrow(' ');
        bold.fg = BLACK; // blank と同色にして attrs 以外を一致させる
        bold.bg = BLACK;
        bold.attrs = CellAttrs::BOLD;
        new.set(1, 0, bold);
        assert_eq!(
            changed_runs(&old, &new),
            vec![CellRun {
                row: 0,
                col: 1,
                len: 1
            }]
        );
        // 判別: 完全同一 grid は空。
        assert!(changed_runs(&old, &old).is_empty());
    }

    #[test]
    fn changed_runs_coalesce_and_split() {
        let old = CellGrid::new_blank(6, 1, BLACK);
        let mut new = old.clone();
        // (0,0),(1,0) 連続 + (4,0) 単独 → 2 run。
        new.set(0, 0, narrow('a'));
        new.set(1, 0, narrow('b'));
        new.set(4, 0, narrow('c'));
        assert_eq!(
            changed_runs(&old, &new),
            vec![
                CellRun {
                    row: 0,
                    col: 0,
                    len: 2
                },
                CellRun {
                    row: 0,
                    col: 4,
                    len: 1
                },
            ]
        );
    }

    #[test]
    fn changed_runs_dims_mismatch_full_repaint() {
        let old = CellGrid::new_blank(2, 2, BLACK);
        let new = CellGrid::new_blank(3, 2, WHITE);
        assert_eq!(
            changed_runs(&old, &new),
            vec![
                CellRun {
                    row: 0,
                    col: 0,
                    len: 3
                },
                CellRun {
                    row: 1,
                    col: 0,
                    len: 3
                },
            ]
        );
    }

    #[test]
    fn changed_runs_wide_pulls_continuation() {
        // Wide が変化 → 右の Continuation も同じ run へ。
        let mut old = CellGrid::new_blank(3, 1, BLACK);
        let wide = Cell {
            ch: '漢',
            fg: WHITE,
            bg: BLACK,
            width: CellWidth::Wide,
            attrs: CellAttrs::NONE,
        };
        let cont = Cell {
            ch: ' ',
            fg: WHITE,
            bg: BLACK,
            width: CellWidth::Continuation,
            attrs: CellAttrs::NONE,
        };
        old.set(0, 0, wide.clone());
        old.set(1, 0, cont.clone());
        let mut new = old.clone();
        // Wide の文字だけ変える（Continuation は同一）。
        let mut wide2 = wide;
        wide2.ch = '字';
        new.set(0, 0, wide2);
        assert_eq!(
            changed_runs(&old, &new),
            vec![CellRun {
                row: 0,
                col: 0,
                len: 2
            }]
        );
    }

    #[test]
    fn changed_runs_continuation_pulls_wide() {
        // Continuation が変化 → 左の Wide も同じ run へ。
        let mut old = CellGrid::new_blank(3, 1, BLACK);
        let wide = Cell {
            ch: '漢',
            fg: WHITE,
            bg: BLACK,
            width: CellWidth::Wide,
            attrs: CellAttrs::NONE,
        };
        let cont = Cell {
            ch: ' ',
            fg: WHITE,
            bg: BLACK,
            width: CellWidth::Continuation,
            attrs: CellAttrs::NONE,
        };
        old.set(0, 0, wide);
        old.set(1, 0, cont.clone());
        let mut new = old.clone();
        let mut cont2 = cont;
        cont2.bg = [1, 2, 3, 255];
        new.set(1, 0, cont2);
        assert_eq!(
            changed_runs(&old, &new),
            vec![CellRun {
                row: 0,
                col: 0,
                len: 2
            }]
        );
    }

    // ---- 罫線（box-drawing） ----

    fn rect(x: u32, y: u32, w: u32, h: u32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn draw_box_outline_corners_edges_and_hollow_interior() {
        let mut g = CellGrid::new_blank(6, 4, BLACK);
        draw_box(
            &mut g,
            rect(1, 0, 4, 3),
            DEFAULT,
            CellAttrs::NONE,
            BoxStyle::Square,
        );
        // コーナー。
        assert_eq!(g.get(1, 0).ch, '┌');
        assert_eq!(g.get(4, 0).ch, '┐');
        assert_eq!(g.get(1, 2).ch, '└');
        assert_eq!(g.get(4, 2).ch, '┘');
        // 辺。
        assert_eq!(g.get(2, 0).ch, '─');
        assert_eq!(g.get(3, 0).ch, '─');
        assert_eq!(g.get(1, 1).ch, '│');
        assert_eq!(g.get(4, 1).ch, '│');
        // 内部は触らない（地セルのまま、罫線色は端末既定）。
        assert_eq!(g.get(2, 1).ch, ' ');
        // 罫線セルの背景は DEFAULT（透過）。
        assert_eq!(g.get(1, 0).bg, DEFAULT);
        // 枠外は不変。
        assert_eq!(g.get(0, 0).ch, ' ');
        assert_eq!(g.get(5, 0).ch, ' ');
    }

    #[test]
    fn draw_box_rounded_uses_arc_corners() {
        let mut g = CellGrid::new_blank(4, 3, BLACK);
        draw_box(
            &mut g,
            rect(0, 0, 4, 3),
            WHITE,
            CellAttrs::NONE,
            BoxStyle::Rounded,
        );
        assert_eq!(g.get(0, 0).ch, '╭');
        assert_eq!(g.get(3, 0).ch, '╮');
        assert_eq!(g.get(0, 2).ch, '╰');
        assert_eq!(g.get(3, 2).ch, '╯');
    }

    #[test]
    fn draw_box_degenerate_dims() {
        // w==1 → 縦線のみ（コーナー無し）。
        let mut g = CellGrid::new_blank(3, 3, BLACK);
        draw_box(
            &mut g,
            rect(1, 0, 1, 3),
            DEFAULT,
            CellAttrs::NONE,
            BoxStyle::Square,
        );
        for row in 0..3 {
            assert_eq!(g.get(1, row).ch, '│');
        }
        // h==1 → 横線のみ。
        let mut g2 = CellGrid::new_blank(3, 3, BLACK);
        draw_box(
            &mut g2,
            rect(0, 2, 3, 1),
            DEFAULT,
            CellAttrs::NONE,
            BoxStyle::Square,
        );
        for col in 0..3 {
            assert_eq!(g2.get(col, 2).ch, '─');
        }
        // w==0 → 無変更。
        let mut g3 = CellGrid::new_blank(2, 2, BLACK);
        draw_box(
            &mut g3,
            rect(0, 0, 0, 2),
            DEFAULT,
            CellAttrs::NONE,
            BoxStyle::Square,
        );
        assert_eq!(g3.get(0, 0).ch, ' ');
    }

    #[test]
    fn draw_box_clips_at_grid_edge_without_panic() {
        let mut g = CellGrid::new_blank(3, 3, BLACK);
        // grid をはみ出す矩形でもパニックせず、内側だけ描く。
        draw_box(
            &mut g,
            rect(1, 1, 10, 10),
            DEFAULT,
            CellAttrs::NONE,
            BoxStyle::Square,
        );
        assert_eq!(g.get(1, 1).ch, '┌');
        // 右下コーナーは範囲外なので出ない（クリップ）。範囲内の辺は描かれる。
        assert_eq!(g.get(2, 1).ch, '─');
        assert_eq!(g.get(1, 2).ch, '│');
    }

    #[test]
    fn lines_draw_runs() {
        let mut g = CellGrid::new_blank(5, 3, BLACK);
        draw_hline(&mut g, 0, 1, 3, DEFAULT, CellAttrs::NONE);
        for col in 1..4 {
            assert_eq!(g.get(col, 0).ch, '─');
        }
        assert_eq!(g.get(0, 0).ch, ' ');
        draw_vline(&mut g, 4, 0, 3, DEFAULT, CellAttrs::NONE);
        for row in 0..3 {
            assert_eq!(g.get(4, row).ch, '│');
        }
    }
}
