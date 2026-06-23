//! 公開 API。[`compile_deck`] が `World` を 1 度コンパイルして [`CompiledDeck`] を
//! 返し、そこから [`Deck`]・[`Frame`]・PDF をすべて派生させる。
//!
//! 構造抽出は Touying の pdfpc メタを必須とし、無音フォールバックしない
//! （ブリーフ §A）。pdfpc 不在は [`EngineError::PdfpcMissing`]。

use std::collections::HashMap;

use ecow::EcoVec;
use paladocs_core::{Deck, DeckMeta, FrameId, SizePt};
use paladocs_render::{Frame, PixelSize};
use typst::World;
use typst::diag::SourceDiagnostic;
use typst::layout::{Frame as TypstFrame, FrameItem};
use typst::model::Document;
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;
use typst_render::RenderOptions;

use crate::convert;
use crate::diag::{self, EngineError};

/// コンパイル済みデッキ。論理 IR（[`Deck`]）と Typst ページ実体
/// （[`PagedDocument`]）、コンパイル時の警告を 1 つにまとめて保持する。
///
/// 同じ `doc` から [`render_frame`](CompiledDeck::render_frame)・
/// [`render_fit`](CompiledDeck::render_fit)・[`to_pdf`](CompiledDeck::to_pdf)・
/// [`render_step`](crate::render_step) をすべて派生させる。
#[derive(Debug)]
pub struct CompiledDeck {
    /// 構築済みの論理デッキ（[`Deck::validate`] 済み）。
    pub deck: Deck,
    /// Typst のページ実体。各 [`FrameId`] は `doc.pages()` のインデックスに対応。
    pub doc: PagedDocument,
    /// コンパイル時に生成された警告（捨てずに保持する）。
    pub warnings: EcoVec<SourceDiagnostic>,
    /// 本文サイズ（pt）。全テキストの文字数加重ヒストグラムの最頻サイズ。
    /// [`render_step`](crate::render_step) がテキストスタイルティア（見出し→属性）の
    /// 基準に使う。テキストが無ければ `0.0`（ティア無効）。
    pub body_size_pt: f64,
}

/// `world` をコンパイルして [`CompiledDeck`] を構築する。
///
/// 失敗時は [`EngineError`]:
/// - コンパイルエラー → [`EngineError::Compile`]（行/列つき）。
/// - pdfpc メタ不在 → [`EngineError::PdfpcMissing`]。
/// - pdfpc スキーマ不整合 → [`EngineError::PdfpcSchema`]。
///
/// 警告は [`CompiledDeck::warnings`] に保持される（捨てない）。
pub fn compile_deck(world: &dyn World) -> Result<CompiledDeck, EngineError> {
    let typst::diag::Warned { output, warnings } = typst::compile::<PagedDocument>(world);
    let doc = output.map_err(|d| diag::compile_error(world, &d))?;
    let meta = deck_meta(&doc);
    let deck = crate::pdfpc::pdfpc_deck(&doc, meta)?;
    let body_size_pt = compute_body_size(&doc);
    Ok(CompiledDeck {
        deck,
        doc,
        warnings,
        body_size_pt,
    })
}

/// 全ページの本文サイズ（pt）を文字数加重ヒストグラムの最頻値として算出する。
fn compute_body_size(doc: &PagedDocument) -> f64 {
    let mut samples: Vec<(f64, usize)> = Vec::new();
    for page in doc.pages() {
        collect_text_sizes(&page.frame, &mut samples);
    }
    body_size_from(samples.into_iter())
}

/// `frame` のテキスト走を再帰的にたどり、`(size_pt, glyph 数)` を集める。
fn collect_text_sizes(frame: &TypstFrame, out: &mut Vec<(f64, usize)>) {
    for (_pos, item) in frame.items() {
        match item {
            FrameItem::Group(group) => collect_text_sizes(&group.frame, out),
            FrameItem::Text(text) => out.push((text.size.to_pt(), text.glyphs.len())),
            _ => {}
        }
    }
}

/// `(size_pt, weight)` 列から本文サイズを返す。サイズを 0.1pt バケットへ丸めて
/// 文字数加重ヒストグラム化し、最頻バケットの代表 pt を返す。見出しは少数なので
/// 本文が支配的になる。有効サンプルが無ければ `0.0`。
fn body_size_from(samples: impl Iterator<Item = (f64, usize)>) -> f64 {
    let mut hist: HashMap<i64, u64> = HashMap::new();
    for (size_pt, weight) in samples {
        if !size_pt.is_finite() || size_pt <= 0.0 || weight == 0 {
            continue;
        }
        let bucket = (size_pt * 10.0).round() as i64;
        *hist.entry(bucket).or_insert(0) += weight as u64;
    }
    hist.into_iter()
        .max_by_key(|&(_, w)| w)
        .map(|(bucket, _)| bucket as f64 / 10.0)
        .unwrap_or(0.0)
}

impl CompiledDeck {
    /// `ppp`（pixels-per-pt）指定でフレームを描画する。
    ///
    /// 返る [`Frame`] は正準形式（RGBA8・ストレートアルファ）。範囲外フレームは
    /// [`EngineError::Render`]。
    pub fn render_frame(&self, frame: FrameId, ppp: f32) -> Result<Frame, EngineError> {
        let pages = self.doc.pages();
        let page = pages.get(frame.0 as usize).ok_or_else(|| {
            EngineError::Render(format!(
                "frame {} out of range (document has {} page(s))",
                frame.0,
                pages.len()
            ))
        })?;
        let opts = RenderOptions {
            pixel_per_pt: (ppp as f64).into(),
            render_bleed: false,
        };
        let pixmap = typst_render::render(page, &opts);
        let image = convert::pixmap_to_rgba(&pixmap)?;
        Ok(Frame { id: frame, image })
    }

    /// フレームを pixel ビューポートにアスペクト保持で収める。
    ///
    /// 内部で `paladocs_render::fit` を使い `ppp = fit.w / page_pt.w` を算出する。
    /// `Pixmap` の実寸は fit 矩形と ±1px ずれうるが、返る [`Frame`] は `Pixmap`
    /// 実寸を保持する。
    pub fn render_fit(&self, frame: FrameId, viewport: PixelSize) -> Result<Frame, EngineError> {
        let ppp = convert::ppp_for_fit(self.deck.meta.page_pt, viewport);
        self.render_frame(frame, ppp)
    }

    /// 同じ `PagedDocument` から PDF バイト列を出力する。
    ///
    /// PDF エクスポートの失敗診断をソース位置へ解決するため、コンパイルに使った
    /// `world` を渡す（[`compile_deck`] と同じものを渡すこと）。
    pub fn to_pdf(&self, world: &dyn World) -> Result<Vec<u8>, EngineError> {
        typst_pdf::pdf(&self.doc, &PdfOptions::default())
            .map_err(|d| diag::compile_error(world, &d))
    }
}

/// ドキュメントから [`DeckMeta`] を作る。
///
/// `page_pt` は先頭ページのフレームサイズ（pt）。スライドは一様サイズ前提で、
/// 非一様でも先頭を採用する。タイトルはドキュメント情報から、無ければ `None`。
fn deck_meta(doc: &PagedDocument) -> DeckMeta {
    let page_pt = doc
        .pages()
        .first()
        .map(|p| SizePt {
            w: p.frame.width().to_pt() as f32,
            h: p.frame.height().to_pt() as f32,
        })
        .unwrap_or(SizePt { w: 0.0, h: 0.0 });
    let title = doc.info().title.as_ref().map(|t| t.to_string());
    DeckMeta { title, page_pt }
}

#[cfg(test)]
mod tests {
    use super::body_size_from;

    #[test]
    fn body_size_picks_weighted_mode() {
        // 本文 11pt が文字数で支配的、見出し 22pt は少数 → body = 11.0。
        let samples = [(11.0, 200), (22.0, 5), (16.5, 3)];
        assert_eq!(body_size_from(samples.into_iter()), 11.0);
    }

    #[test]
    fn body_size_buckets_near_sizes() {
        // 11.02 と 10.98 は同じ 0.1pt バケット(110)へ → 合算で最頻。
        let samples = [(11.02, 50), (10.98, 60), (22.0, 40)];
        assert_eq!(body_size_from(samples.into_iter()), 11.0);
    }

    #[test]
    fn body_size_empty_is_zero() {
        let samples: [(f64, usize); 0] = [];
        assert_eq!(body_size_from(samples.into_iter()), 0.0);
        // 不正サンプルも除外される。
        let bad = [(f64::NAN, 10), (-1.0, 10), (12.0, 0)];
        assert_eq!(body_size_from(bad.into_iter()), 0.0);
    }
}
