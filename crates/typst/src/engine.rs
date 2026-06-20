//! 公開 API。[`Engine`] が `World` とコンパイル済み `PagedDocument` を保持し、
//! そこから [`Deck`]・[`Frame`]・PDF を派生させる。

use std::path::Path;

use paladocs_core::{Deck, DeckMeta, FrameId, SizePt};
use paladocs_render::{Frame, PixelSize};
use typst::model::Document;
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;
use typst_render::RenderOptions;

use crate::convert;
use crate::deck;
use crate::diag::{self, EngineError};
use crate::world::PaladocsWorld;

/// Typst プレゼンテーションエンジン。
///
/// [`compile`](Engine::compile) で `root.typ` を 1 度コンパイルし、同じ
/// `PagedDocument` から Deck・Frame・PDF をすべて派生させる。
/// [`reload`](Engine::reload) で変更を取り込んで再コンパイルする。
pub struct Engine {
    world: PaladocsWorld,
    doc: PagedDocument,
    deck: Deck,
}

impl Engine {
    /// `root`（entrypoint の `.typ`）をコンパイルしてエンジンを作る。
    ///
    /// 失敗時は [`EngineError`]（コンパイルエラーなら行/列つきの
    /// [`EngineError::Compile`]）。
    pub fn compile(root: &Path) -> Result<Engine, EngineError> {
        let world = PaladocsWorld::new(root)?;
        let doc = compile_doc(&world)?;
        let deck = build_deck(&doc)?;
        Ok(Engine { world, doc, deck })
    }

    /// 構築済みデッキ（論理 IR）。
    pub fn deck(&self) -> &Deck {
        &self.deck
    }

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
    pub fn to_pdf(&self) -> Result<Vec<u8>, EngineError> {
        typst_pdf::pdf(&self.doc, &PdfOptions::default())
            .map_err(|d| diag::compile_error(&self.world, &d))
    }

    /// 変更されたソースを再読込し、再コンパイルして Deck を作り直す。
    ///
    /// `comemo` の増分メモ化により、変わっていないファイルの再評価は省かれる。
    pub fn reload(&mut self) -> Result<(), EngineError> {
        self.world.reset_files();
        self.doc = compile_doc(&self.world)?;
        self.deck = build_deck(&self.doc)?;
        Ok(())
    }
}

/// `World` をコンパイルして `PagedDocument` を得る。warnings は捨てる
/// （致命的エラーのみ [`EngineError::Compile`] にする）。
fn compile_doc(world: &PaladocsWorld) -> Result<PagedDocument, EngineError> {
    let typst::diag::Warned {
        output,
        warnings: _,
    } = typst::compile::<PagedDocument>(world);
    output.map_err(|d| diag::compile_error(world, &d))
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

/// Deck を構築する。
///
/// まず Touying の pdfpc メタデータ（[`crate::pdfpc`]）で overlay/ノートを復元
/// しようとし、取れなければフォールバック（1 ページ = 1 スライド = 1 step）へ
/// 落ちる。いずれも内部で [`Deck::validate`] を通す。
fn build_deck(doc: &PagedDocument) -> Result<Deck, EngineError> {
    let meta = deck_meta(doc);
    match crate::pdfpc::pdfpc_deck(doc, meta.clone()) {
        Some(result) => result,
        None => deck::fallback_deck(doc.pages().len(), meta),
    }
}
