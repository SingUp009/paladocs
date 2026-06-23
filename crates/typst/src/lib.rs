//! `paladocs-typst` — Typst コンパイルエンジン。
//!
//! 本クレートは **重い外部依存（Typst コンパイラ群・tiny-skia・comemo）を隔離**
//! する層であり、依存方向は `typst → render → core`。Typst ソースをコンパイルして
//! [`paladocs_core::Deck`] を構築し、各ページを [`paladocs_render::Frame`]（正準
//! ストレートアルファ）として描画し、PDF を出力し、reload（再コンパイル）と
//! 診断マッピングを提供する。
//!
//! 中心となる型は [`CompiledDeck`]。[`compile_deck`] が `World` を 1 度コンパイル
//! して返し、同一コンパイル結果（`PagedDocument`）から Deck・Frame・PDF・
//! [`render_step`]（ページ→セル投影）をすべて派生させる。
//!
//! # 正準形式
//!
//! Typst（tiny-skia `Pixmap`）の出力は**プリマルチプライド**アルファである。
//! 本クレートは [`CompiledDeck::render_frame`] 等で必ず**アンプリマルチプライ**して
//! [`paladocs_render`] の正準形式（RGBA8・ストレートアルファ）へ変換してから返す。
//!
//! # Deck の不変条件
//!
//! 構築した [`Deck`](paladocs_core::Deck) は返す前に必ず
//! [`Deck::validate`](paladocs_core::Deck::validate) を通す。違反は
//! [`EngineError`] に変換される。

mod compiled;
mod convert;
mod deck;
mod diag;
mod pdfpc;
mod project;
mod world;

pub use compiled::{CompiledDeck, compile_deck};
pub use diag::{Diagnostic, EngineError, Severity};
pub use project::{RenderOpts, render_step};
pub use world::PaladocsWorld;
