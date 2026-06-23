//! B: ページ→CellGrid 投影（MDPT 方式 = approach B）。
//!
//! 各 Step（= 物理ページ）を「ラスタ base ＋ テキスト上書き」でセル化する:
//!
//! - **B-1 base ラスタ**: `typst_render::render` の premultiplied Pixmap を正準
//!   ストレートアルファ [`Frame`] へ変換し、[`quantize_half_block`] で base
//!   [`CellGrid`] にする。
//! - **B-2 テキスト上書き**: `page.frame` のテキスト走を再帰的にたどり、各グリフを
//!   鮮明なセルとして base に上書きする。背景は常に `page_bg`（base サンプル禁止＝
//!   グリフインク汚染回避、ブリーフ判断2）。
//! - **B-3 CJK グリッドスナップ**: 行ごとにセルカーソルを持ち、比例配置→monospace の
//!   累積丸めドリフトを断つ（ブリーフ判断3）。
//!
//! ANSI 出力（term）・アスペクト調整（cli）は本クレートのスコープ外。

use std::collections::HashMap;

use paladocs_core::FrameId;
use paladocs_render::{Cell, CellAttrs, CellGrid, CellWidth, Color, Frame, quantize_half_block};
use typst::layout::{Abs, Frame as TypstFrame, FrameItem, Point, Transform};
use typst::text::TextItem;
use typst::visualize::Paint;
use typst_render::RenderOptions;
use unicode_width::UnicodeWidthChar;

use crate::CompiledDeck;
use crate::convert;
use crate::diag::EngineError;

/// `render_step` の描画パラメータ。
///
/// `(cols, rows)` は呼び出し側が端末サイズ＋スライドアスペクト（`fit`/letterbox）から
/// 与える。本クレートはアスペクト調整しない。`pixel_per_pt` は base ラスタ（図形・
/// 画像）が十分鮮明になる値を選ぶ。
pub struct RenderOpts {
    /// 出力グリッドのカラム数。
    pub cols: u16,
    /// 出力グリッドの行数。
    pub rows: u16,
    /// base ラスタの解像度（pixels-per-pt）。
    pub pixel_per_pt: f32,
}

/// `compiled` の物理ページ `page_idx` を「ラスタ base ＋ テキスト上書き」で
/// `(cols, rows)` の [`CellGrid`] に投影する。
///
/// 不変条件:
/// - 出力 dims == `(opts.cols, opts.rows)`、全 Cell 不透明（render 不変条件3 継承）。
/// - [`CellWidth::Wide`] の右隣は必ず [`CellWidth::Continuation`]（render 不変条件4）。
///
/// `page_idx` が範囲外なら [`EngineError::Render`]。
pub fn render_step(
    compiled: &CompiledDeck,
    page_idx: usize,
    opts: &RenderOpts,
) -> Result<CellGrid, EngineError> {
    let pages = compiled.doc.pages();
    let page = pages.get(page_idx).ok_or_else(|| {
        EngineError::Render(format!(
            "page {page_idx} out of range (document has {} page(s))",
            pages.len()
        ))
    })?;

    // ページの背景色（不透明 Color）。`None`（透過）/`Auto` は v1 では不透明白、
    // 単色塗りはストレート sRGB バイトへ。Gradient/Tiling も v1 は白で代用する。
    let page_bg = resolve_page_bg(page.fill_or_white());

    // --- B-1 base ラスタ ---
    let render_opts = RenderOptions {
        pixel_per_pt: (opts.pixel_per_pt as f64).into(),
        render_bleed: false,
    };
    let pixmap = typst_render::render(page, &render_opts);
    let image = convert::pixmap_to_rgba(&pixmap)?;
    let frame = Frame {
        id: FrameId(page_idx as u32),
        image,
    };
    let mut grid = quantize_half_block(&frame, opts.cols, opts.rows, page_bg);

    // --- B-2 テキスト上書き ---
    let frame_w_pt = page.frame.width().to_pt();
    let frame_h_pt = page.frame.height().to_pt();
    let mut runs = Vec::new();
    collect_runs(
        &page.frame,
        Transform::identity(),
        compiled.body_size_pt,
        &mut runs,
    );
    overlay_runs(&mut grid, &runs, frame_w_pt, frame_h_pt, page_bg);

    Ok(grid)
}

/// H1（本文の 1.5 倍以上）ティアの相対閾値。
const H1_RATIO: f64 = 1.5;
/// H2（本文の 1.2 倍以上）ティアの相対閾値。
const H2_RATIO: f64 = 1.2;

/// テキストサイズ（pt）と本文サイズ（pt）から見出しティアの [`CellAttrs`] を決める。
///
/// `>= body*1.5` → H1、`>= body*1.2` → H2、それ未満 → 本文（`NONE`）。当面 H1/H2 は
/// いずれも `BOLD`（`UNDERLINE` は将来の調整点として `H1_RATIO` 等とともに残す）。
/// **色には一切触れない**（fg は呼び出し側が `TextItem::fill` から決める）。`body <= 0`
/// （テキスト無し）は全 `NONE`。
///
/// これはサイズベースの見た目近似であり Typst の意味的見出しではない。将来 introspection
/// による意味判定へ置換可能。
// H1/H2 は当面いずれも BOLD で同値だが、ティア境界（H1_RATIO/H2_RATIO）と
// 分岐は将来の差別化（H1 に UNDERLINE 追加等）の成長点として明示的に残す。
#[allow(clippy::if_same_then_else)]
fn tier_attrs(size_pt: f64, body_pt: f64) -> CellAttrs {
    if body_pt <= 0.0 || !size_pt.is_finite() {
        return CellAttrs::NONE;
    }
    if size_pt >= body_pt * H1_RATIO {
        CellAttrs::BOLD // H1
    } else if size_pt >= body_pt * H2_RATIO {
        CellAttrs::BOLD // H2
    } else {
        CellAttrs::NONE
    }
}

/// 解決済み `Page.fill` を不透明セル背景 [`Color`] にする。
fn resolve_page_bg(fill: Option<Paint>) -> Color {
    match fill {
        Some(Paint::Solid(color)) => {
            let (r, g, b, _) = color.to_rgb().into_format::<u8, u8>().into_components();
            [r, g, b, 255]
        }
        // 透過（None）・Gradient・Tiling は v1 では不透明白で代用する。
        _ => [255, 255, 255, 255],
    }
}

/// 1 つのテキスト走（= 単一 [`TextItem`]）。run 内のグリフは累積 advance で連続し、
/// 位置ギャップは空白グリフ（[`RunGlyph::is_space`]）としてのみ現れる。
struct PlacedRun {
    /// run 開始（先頭グリフ）の x（ページ pt）。配置の起点列にのみ使う。
    x_pt: f64,
    /// baseline の y（ページ pt）。行の決定に使う。
    y_pt: f64,
    /// サイズティアから決めた属性（[`CellAttrs`]）。run 内全セル共通。
    attrs: CellAttrs,
    /// 出現順のグリフ列。
    glyphs: Vec<RunGlyph>,
}

/// run 内の 1 グリフ。位置は持たず、advance（pt）とセル幅で前進する。
struct RunGlyph {
    /// このグリフの x_advance（ページ pt）。空白の列換算・font スケール基準に使う。
    adv_pt: f64,
    /// 表示文字（先頭グラフェムの先頭 char）。
    ch: char,
    /// セル占有幅（[`CellWidth::Narrow`] か [`CellWidth::Wide`] のみ）。
    width: CellWidth,
    /// 前景色。`None` は単色塗り以外（Gradient/Tiling）で base サンプル代用の合図。
    fg: Option<Color>,
    /// 空白文字か（語間・タブ）。run 内では advance の列換算ぶん blank で前進する。
    is_space: bool,
}

/// `frame` のテキスト走を `ts`（current-frame→page の累積変換）を畳みながら再帰収集する。
///
/// グループに入るたび `ts' = ts ∘ translate(pos) ∘ group.transform` を合成する
/// （`typst_render` の走査と同順）。run 開始のページ座標は
/// `(pos, pos.y).transform(ts)`。`body_pt` はサイズティア判定の基準。
fn collect_runs(frame: &TypstFrame, ts: Transform, body_pt: f64, out: &mut Vec<PlacedRun>) {
    for (pos, item) in frame.items() {
        match item {
            FrameItem::Group(group) => {
                let child_ts = ts
                    .pre_concat(Transform::translate(pos.x, pos.y))
                    .pre_concat(group.transform);
                collect_runs(&group.frame, child_ts, body_pt, out);
            }
            FrameItem::Text(text) => collect_text(text, *pos, ts, body_pt, out),
            _ => {}
        }
    }
}

/// 1 つのテキスト走を 1 つの [`PlacedRun`] として収集する。
fn collect_text(
    text: &TextItem,
    pos: Point,
    ts: Transform,
    body_pt: f64,
    out: &mut Vec<PlacedRun>,
) {
    // 単色塗りなら fg を straight sRGB バイトへ。Gradient/Tiling は None（base サンプル）。
    let fg = match &text.fill {
        Paint::Solid(color) => {
            let (r, g, b, _) = color.to_rgb().into_format::<u8, u8>().into_components();
            Some([r, g, b, 255])
        }
        Paint::Gradient(_) | Paint::Tiling(_) => None,
    };
    let attrs = tier_attrs(text.size.to_pt(), body_pt);

    let mut advance = Abs::zero();
    let mut glyphs: Vec<RunGlyph> = Vec::new();
    let mut start: Option<(f64, f64)> = None; // run 開始の (x_pt, y_pt)

    for glyph in &text.glyphs {
        let x_offset = glyph.x_offset.at(text.size);
        // current-frame 座標 → page 座標へ写像（run 開始位置の確定にのみ使う）。
        let local = Point::new(pos.x + advance + x_offset, pos.y);
        let abs = local.transform(ts);
        let adv_pt = glyph.x_advance.at(text.size).to_pt();
        advance += glyph.x_advance.at(text.size);

        // 文字復元: glyph の text 範囲の先頭 char（v1。合字/結合は先頭採用）。
        let Some(ch) = text.text.get(glyph.range()).and_then(|s| s.chars().next()) else {
            continue;
        };
        let is_space = ch.is_whitespace();
        let width = if is_space {
            CellWidth::Narrow
        } else {
            // 幅 0（結合・ゼロ幅）や制御文字は配置しない。
            match cell_width(ch) {
                Some(w) => w,
                None => continue,
            }
        };
        if start.is_none() {
            start = Some((abs.x.to_pt(), abs.y.to_pt()));
        }
        glyphs.push(RunGlyph {
            adv_pt,
            ch,
            width,
            fg,
            is_space,
        });
    }

    if let Some((x_pt, y_pt)) = start {
        out.push(PlacedRun {
            x_pt,
            y_pt,
            attrs,
            glyphs,
        });
    }
}

/// East Asian Width によるセル占有幅。幅 2 → [`CellWidth::Wide`]、幅 1 →
/// [`CellWidth::Narrow`]、幅 0/制御 → `None`（配置しない）。
fn cell_width(ch: char) -> Option<CellWidth> {
    match UnicodeWidthChar::width(ch) {
        Some(2) => Some(CellWidth::Wide),
        Some(1) => Some(CellWidth::Narrow),
        _ => None,
    }
}

/// 収集済み run を CJK グリッドスナップ（判断3, addendum 改修）で `grid` に上書きする。
///
/// 行（baseline row）ごとにセルカーソルを持ち、document 順に各 run を:
/// 1. **run 開始列**のみ pt で算出（`start_col = round(run.x_pt / pt_per_col)`）。
///    その行の初回 run は `start_col` でカーソルを初期化し（行頭 base を消さない）、
///    2 回目以降の run は `[line_cursor, max(line_cursor, start_col))` を `page_bg`
///    blank で埋める（run 間の余白）。
/// 2. **run 内前進は文字内容＋セル幅駆動**（pt 非依存）:
///    - 空白グリフ → advance の列換算ぶん（`round(adv / ref_per_col)`、最低 1）を
///      `page_bg` blank で埋めて前進（語間・タブ）。
///    - 非空白 → `cursor` に Cell を置き `cursor += width`（Narrow=1 / Wide=2）。
/// 3. 行末で `line_cursor = cursor`。
///
/// `ref_per_col` は run 内の最小の `adv/width`（= その font の 1 列ぶん自然 advance）。
/// これにより大フォント run でも空白換算がスケール不変になり、`target=round` 方式の
/// 累積ドリフト（字間開き）が消える。
fn overlay_runs(
    grid: &mut CellGrid,
    runs: &[PlacedRun],
    frame_w_pt: f64,
    frame_h_pt: f64,
    page_bg: Color,
) {
    let (cols, rows) = grid.dims();
    if cols == 0 || rows == 0 || frame_w_pt <= 0.0 || frame_h_pt <= 0.0 {
        return;
    }
    let pt_per_col = frame_w_pt / cols as f64;
    let pt_per_row = frame_h_pt / rows as f64;

    // 行 → 次の空きカラム（line_cursor）。
    let mut line_cursors: HashMap<u16, u16> = HashMap::new();

    for run in runs {
        let row_f = (run.y_pt / pt_per_row).floor();
        if !row_f.is_finite() || row_f < 0.0 || row_f >= rows as f64 {
            continue;
        }
        let row = row_f as u16;
        let start_col = col_from_pt(run.x_pt, pt_per_col, cols);

        // font スケール基準: run 内の最小 adv/width。無ければ pt_per_col へフォールバック。
        let ref_per_col = run
            .glyphs
            .iter()
            .filter(|g| !g.is_space && g.adv_pt > 0.0)
            .map(|g| g.adv_pt / g.width_cells() as f64)
            .fold(f64::INFINITY, f64::min);
        let ref_per_col = if ref_per_col.is_finite() && ref_per_col > 0.0 {
            ref_per_col
        } else {
            pt_per_col
        };

        // カーソル確定。初回 run は start_col で初期化（行頭 base 保持）、2 回目以降は
        // [line_cursor, max(line_cursor, start_col)) を page_bg で埋める。
        let mut cursor = match line_cursors.get(&row) {
            Some(&lc) => {
                let c = lc.max(start_col);
                for col in lc..c.min(cols) {
                    grid.set(col, row, Cell::blank(page_bg, page_bg));
                }
                c
            }
            None => start_col,
        };

        for g in &run.glyphs {
            if cursor >= cols {
                break;
            }
            if g.is_space {
                // 空白の占有列数（advance を ref で列換算、最低 1）。
                let n = (g.adv_pt / ref_per_col).round();
                let n = if n.is_finite() && n >= 1.0 {
                    (n as u32).min(cols as u32)
                } else {
                    1
                };
                for _ in 0..n {
                    if cursor >= cols {
                        break;
                    }
                    grid.set(cursor, row, Cell::blank(page_bg, page_bg));
                    cursor += 1;
                }
                continue;
            }

            let place = cursor;
            // Gradient/Tiling は base ラスタの当該セル色を fg に代用（上書き前に読む）。
            let fg = match g.fg {
                Some(c) => c,
                None => grid.get(place, row).fg,
            };
            cursor = if g.width == CellWidth::Wide && place + 1 < cols {
                grid.set(
                    place,
                    row,
                    Cell {
                        ch: g.ch,
                        fg,
                        bg: page_bg,
                        width: CellWidth::Wide,
                        attrs: run.attrs,
                    },
                );
                grid.set(
                    place + 1,
                    row,
                    Cell {
                        ch: ' ',
                        fg,
                        bg: page_bg,
                        width: CellWidth::Continuation,
                        attrs: run.attrs,
                    },
                );
                place + 2
            } else {
                // Narrow、または Wide が右端で入りきらない場合は Narrow へ縮退して
                // 不変条件5（Wide の右隣は Continuation）を守る。
                grid.set(
                    place,
                    row,
                    Cell {
                        ch: g.ch,
                        fg,
                        bg: page_bg,
                        width: CellWidth::Narrow,
                        attrs: run.attrs,
                    },
                );
                place + 1
            };
        }
        line_cursors.insert(row, cursor);
    }
}

/// run 開始 x（ページ pt）→ 起点列。負・非有限は 0、超過は `cols` でクランプ。
fn col_from_pt(x_pt: f64, pt_per_col: f64, cols: u16) -> u16 {
    let f = (x_pt / pt_per_col).round();
    if f.is_finite() && f > 0.0 {
        (f as i64).min(cols as i64) as u16
    } else {
        0
    }
}

impl RunGlyph {
    /// セル幅の列数（Narrow=1 / Wide=2）。Continuation は run 内に現れない。
    fn width_cells(&self) -> u16 {
        match self.width {
            CellWidth::Wide => 2,
            _ => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Color = [10, 20, 30, 255];
    const FG: Color = [200, 200, 200, 255];

    /// run 内グリフ 1 個を作る。`is_space` は文字から自動判定。
    fn rg(adv_pt: f64, ch: char, width: CellWidth, fg: Option<Color>) -> RunGlyph {
        RunGlyph {
            adv_pt,
            ch,
            width,
            fg,
            is_space: ch.is_whitespace(),
        }
    }

    /// `x_pt` 起点・`attrs` の単一 run（y_pt=0）を作る。
    fn run(x_pt: f64, attrs: CellAttrs, glyphs: Vec<RunGlyph>) -> PlacedRun {
        PlacedRun {
            x_pt,
            y_pt: 0.0,
            attrs,
            glyphs,
        }
    }

    /// pt_per_col = pt_per_row = 1.0 になる単位グリッド（x_pt がそのまま列）。
    fn unit_grid(cols: u16, rows: u16) -> CellGrid {
        CellGrid::new_blank(cols, rows, BG)
    }

    /// 全角3文字 → 各 Wide＋Continuation で 6 カラム占有・列ドリフト無し。
    #[test]
    fn full_width_three_chars_occupy_six_columns() {
        let mut grid = unit_grid(8, 1);
        // 各全角 adv=2（width2 → ref_per_col=1）。
        let r = run(
            0.0,
            CellAttrs::NONE,
            vec![
                rg(2.0, '漢', CellWidth::Wide, Some(FG)),
                rg(2.0, '字', CellWidth::Wide, Some(FG)),
                rg(2.0, '体', CellWidth::Wide, Some(FG)),
            ],
        );
        overlay_runs(&mut grid, &[r], 8.0, 1.0, BG);
        let widths: Vec<CellWidth> = (0..6).map(|c| grid.get(c, 0).width).collect();
        assert_eq!(
            widths,
            vec![
                CellWidth::Wide,
                CellWidth::Continuation,
                CellWidth::Wide,
                CellWidth::Continuation,
                CellWidth::Wide,
                CellWidth::Continuation,
            ]
        );
        assert_eq!(grid.get(0, 0).ch, '漢');
        assert_eq!(grid.get(2, 0).ch, '字');
        assert_eq!(grid.get(4, 0).ch, '体');
        // 占有外（6,7）は base のまま。
        assert_eq!(*grid.get(6, 0), Cell::blank(BG, BG));
    }

    /// 判別ペア: 半角3文字は 3 カラム（全角扱いする実装は落ちる）。
    #[test]
    fn half_width_three_chars_occupy_three_columns() {
        let mut grid = unit_grid(8, 1);
        let r = run(
            0.0,
            CellAttrs::NONE,
            vec![
                rg(1.0, 'a', CellWidth::Narrow, Some(FG)),
                rg(1.0, 'b', CellWidth::Narrow, Some(FG)),
                rg(1.0, 'c', CellWidth::Narrow, Some(FG)),
            ],
        );
        overlay_runs(&mut grid, &[r], 8.0, 1.0, BG);
        for c in 0..3 {
            assert_eq!(grid.get(c, 0).width, CellWidth::Narrow);
        }
        assert_eq!(grid.get(0, 0).ch, 'a');
        assert_eq!(grid.get(2, 0).ch, 'c');
        assert_eq!(*grid.get(3, 0), Cell::blank(BG, BG));
    }

    /// 行頭の base は消さず（カーソル遅延初期化）、run 内の空白だけ page_bg で埋める。
    #[test]
    fn leading_kept_and_inner_space_filled_with_page_bg() {
        let mut grid = unit_grid(8, 1);
        // base に先頭 2 列を別内容で置いておく（行頭が保たれることの確認用）。
        let marker = Cell {
            ch: '#',
            fg: FG,
            bg: BG,
            width: CellWidth::Narrow,
            attrs: CellAttrs::NONE,
        };
        grid.set(0, 0, marker.clone());
        grid.set(1, 0, marker.clone());
        // run は列3 起点で 'a' 空白 'b'（空白 adv=1 → 1 セル blank）。
        let r = run(
            3.0,
            CellAttrs::NONE,
            vec![
                rg(1.0, 'a', CellWidth::Narrow, Some(FG)),
                rg(1.0, ' ', CellWidth::Narrow, Some(FG)),
                rg(1.0, 'b', CellWidth::Narrow, Some(FG)),
            ],
        );
        overlay_runs(&mut grid, &[r], 8.0, 1.0, BG);
        // 行頭（列0,1）は base のまま（カーソルは列3で初期化されるため消えない）。
        assert_eq!(*grid.get(0, 0), marker);
        assert_eq!(*grid.get(1, 0), marker);
        // 列2 は base のまま（カーソル初期化前）。
        assert_eq!(*grid.get(2, 0), Cell::blank(BG, BG));
        assert_eq!(grid.get(3, 0).ch, 'a');
        // 列4 は run 内空白 → page_bg blank。
        assert_eq!(*grid.get(4, 0), Cell::blank(BG, BG));
        assert_eq!(grid.get(5, 0).ch, 'b');
    }

    /// テキストセルの bg は常に page_bg（base サンプルしない＝判断2）。
    #[test]
    fn text_cell_bg_is_page_bg() {
        let mut grid = unit_grid(4, 1);
        let r = run(
            0.0,
            CellAttrs::NONE,
            vec![rg(1.0, 'a', CellWidth::Narrow, Some(FG))],
        );
        overlay_runs(&mut grid, &[r], 4.0, 1.0, BG);
        assert_eq!(grid.get(0, 0).bg, BG);
        assert_eq!(grid.get(0, 0).fg, FG);
    }

    /// fg None（Gradient/Tiling）は上書き前の base セル fg をサンプルして代用する。
    #[test]
    fn gradient_fg_samples_base_cell() {
        let mut grid = unit_grid(4, 1);
        let base = Cell {
            ch: '\u{2580}',
            fg: [1, 2, 3, 255],
            bg: [4, 5, 6, 255],
            width: CellWidth::Narrow,
            attrs: CellAttrs::NONE,
        };
        grid.set(0, 0, base);
        let r = run(
            0.0,
            CellAttrs::NONE,
            vec![rg(1.0, 'x', CellWidth::Narrow, None)],
        );
        overlay_runs(&mut grid, &[r], 4.0, 1.0, BG);
        // fg は base の fg を採用、bg は page_bg。
        assert_eq!(grid.get(0, 0).fg, [1, 2, 3, 255]);
        assert_eq!(grid.get(0, 0).bg, BG);
        assert_eq!(grid.get(0, 0).ch, 'x');
    }

    /// 空 run 列なら上書きゼロ（base のまま）。
    #[test]
    fn no_runs_no_overwrite() {
        let mut grid = unit_grid(4, 2);
        let before = grid.clone();
        overlay_runs(&mut grid, &[], 4.0, 2.0, BG);
        assert_eq!(grid, before);
    }

    /// Wide が右端（最終列）なら Narrow へ縮退し、Continuation を場外に作らない。
    #[test]
    fn wide_at_last_column_degrades_to_narrow() {
        let mut grid = unit_grid(2, 1);
        // 列1（最終列）起点に Wide を狙わせる。
        let r = run(
            1.0,
            CellAttrs::NONE,
            vec![rg(2.0, '漢', CellWidth::Wide, Some(FG))],
        );
        overlay_runs(&mut grid, &[r], 2.0, 1.0, BG);
        assert_eq!(grid.get(1, 0).width, CellWidth::Narrow);
        assert_eq!(grid.get(1, 0).ch, '漢');
    }

    /// ティア判定の閾値（addendum 受け入れ）。
    #[test]
    fn tier_attrs_thresholds() {
        let body = 10.0;
        assert_eq!(tier_attrs(15.0, body), CellAttrs::BOLD); // H1 (>=1.5x)
        assert_eq!(tier_attrs(12.0, body), CellAttrs::BOLD); // H2 (>=1.2x)
        assert_eq!(tier_attrs(11.0, body), CellAttrs::NONE); // 本文
        // body <= 0（テキスト無し）は全 NONE。
        assert_eq!(tier_attrs(99.0, 0.0), CellAttrs::NONE);
    }

    /// 大フォント run（advance≈2×pt_per_col）でも字間 blank ゼロ・連続配置（ドリフト無し）。
    /// 判別: 旧 `target=round(x_pt/pt_per_col)` 方式なら 0,2,4,6 に開いて落ちる。
    #[test]
    fn large_font_run_no_drift() {
        let mut grid = unit_grid(8, 1);
        let r = run(
            0.0,
            CellAttrs::BOLD,
            vec![
                rg(2.0, 'H', CellWidth::Narrow, Some(FG)),
                rg(2.0, 'e', CellWidth::Narrow, Some(FG)),
                rg(2.0, 'a', CellWidth::Narrow, Some(FG)),
                rg(2.0, 'd', CellWidth::Narrow, Some(FG)),
            ],
        );
        overlay_runs(&mut grid, &[r], 8.0, 1.0, BG);
        assert_eq!(grid.get(0, 0).ch, 'H');
        assert_eq!(grid.get(1, 0).ch, 'e'); // 旧方式なら blank
        assert_eq!(grid.get(2, 0).ch, 'a');
        assert_eq!(grid.get(3, 0).ch, 'd');
        // H1 run は全セル BOLD。判別: 本文 run（NONE）なら BOLD 無し。
        for c in 0..4 {
            assert!(grid.get(c, 0).attrs.contains(CellAttrs::BOLD));
        }
    }

    /// 判別: 本文 run（attrs NONE）はセルに BOLD を付けない。
    #[test]
    fn body_run_has_no_bold() {
        let mut grid = unit_grid(4, 1);
        let r = run(
            0.0,
            CellAttrs::NONE,
            vec![rg(1.0, 'x', CellWidth::Narrow, Some(FG))],
        );
        overlay_runs(&mut grid, &[r], 4.0, 1.0, BG);
        assert!(!grid.get(0, 0).attrs.contains(CellAttrs::BOLD));
    }

    /// 色非介入: テーマ着色（fg 指定）の見出し run → fg はそのまま、attrs に BOLD のみ加算。
    #[test]
    fn tier_does_not_touch_color() {
        let mut grid = unit_grid(4, 1);
        let themed = [123, 45, 67, 255];
        let r = run(
            0.0,
            CellAttrs::BOLD,
            vec![rg(1.0, 'T', CellWidth::Narrow, Some(themed))],
        );
        overlay_runs(&mut grid, &[r], 4.0, 1.0, BG);
        assert_eq!(grid.get(0, 0).fg, themed); // 色は維持
        assert!(grid.get(0, 0).attrs.contains(CellAttrs::BOLD));
    }

    /// cell_width: ラテン=Narrow、CJK=Wide、結合/ゼロ幅=None。
    #[test]
    fn cell_width_classification() {
        assert_eq!(cell_width('a'), Some(CellWidth::Narrow));
        assert_eq!(cell_width('漢'), Some(CellWidth::Wide));
        assert_eq!(cell_width('　'), Some(CellWidth::Wide)); // 全角スペース
        assert_eq!(cell_width('\u{0301}'), None); // 結合アクセント（幅0）
    }

    /// resolve_page_bg: 単色塗りは straight sRGB（不透明）、透過/None は白。
    #[test]
    fn page_bg_solid_and_transparent() {
        use typst::visualize::Color as TypstColor;
        let solid = resolve_page_bg(Some(Paint::Solid(TypstColor::from_u8(20, 40, 60, 255))));
        assert_eq!(solid, [20, 40, 60, 255]);
        let transparent = resolve_page_bg(None);
        assert_eq!(transparent, [255, 255, 255, 255]);
    }
}
