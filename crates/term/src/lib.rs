//! `paladocs-term` — Kitty graphics protocol の端末ドライバ。
//!
//! `render::Frame` / `render::Layer` を **Kitty graphics protocol** で Kitty クラス端末
//! （Knightty）へ送り、**image/placement ライフサイクルと z オーバーレイ部分更新**を
//! 管理するステートフルなドライバ。依存方向は `term → render, core`（`typst` には
//! 依存しない）。
//!
//! # 責務の境界
//!
//! 本クレートは**バイト列を抽象 sink（[`std::io::Write`]）へ書くだけ**で、端末そのもの
//! （alt-screen・raw mode・ioctl・入力ループ・制御 socket・engine 呼び出し）は持たない。
//! それらは `cli` の責務。viewport（cols/rows/セル pixel 寸法）は `cli` が測って渡す。
//!
//! # 主要 API
//!
//! - [`Backend`] / [`KittyBackend`]: 画像送信・placement・削除のワイヤ抽象。送信
//!   （`a=t,f=32`）と placement（`a=p`）を分離し、RGBA 直送を基本とする。
//! - [`Presenter`]: present / overlay（部分更新）/ retreat / clear / resize の
//!   ライフサイクル。画像/placement を確実に解放しリークさせない。
//! - [`place_geometry`]: pixel 矩形原点 → アンカーセル + セル内オフセットの純粋写像
//!   （cell マッピングは term の責務）。
//! - [`select_medium`] / [`Capability`] / [`Medium`] / [`transmit_reference`]: 送出
//!   medium（`t=d` 直送 / `t=s` 共有メモリ / `t=f` 一時ファイル）の純粋な選択と参照
//!   wire 生成。Knightty は `t=d` のみ受理する（実測）ため [`KittyBackend`] は常に直送し、
//!   参照 medium は確保機構を持つ将来の backend / cli 向けの建材として提供する。
//! - [`CellSink`]: [`CellGrid`](paladocs_render::CellGrid) → ANSI テキスト出力の別出口。
//!   画像プロトコル経路とは独立し、SGR 差分最小化・truecolor で全描画／部分更新する。
//!
//! # ワイヤ仕様と Knightty 実機での確定点
//!
//! 一次情報は Kitty graphics protocol 仕様だが、最終的な正解は Knightty 実機が受理する
//! 形。実機で確定した主な点:
//!
//! - フォーマットは `f=32`（RGBA 直送）。`s=`(幅)/`v=`(高さ) 必須・非ゼロ、`t=d`（直接）。
//! - チャンクは base64 で ≤ 4096 バイト。先頭チャンクに全制御キー（`i=` 含む）、継続
//!   チャンクは `m=` のみ、最終 `m=0`。`a=p` はチャンク不可。
//! - **1 placement のソフト削除は `d=i`（小文字）＋ `i=`＋`p=`** を使う。Knightty の
//!   `d=p` は**セル**削除で base placement も巻き込むため不採用（仕様＋実機で確定した
//!   是正点）。画像のハード削除は `d=I`（大文字）。
//! - カーソルは CSI CUP（1-based）で動かし、`a=p,C=1` で不動。前後を DECSC/DECRC で囲む。

mod backend;
mod base64;
mod cell_sink;
mod encode;
mod geometry;
mod ids;
mod medium;
mod presenter;

pub use backend::{Backend, KittyBackend};
pub use cell_sink::CellSink;
pub use encode::transmit_reference;
pub use geometry::{CellPos, CellSize, PixelOffset, Placement, Viewport, place_geometry};
pub use ids::{IdAllocator, ImageId, PlacementId};
pub use medium::{Capability, DIRECT_MAX_PAYLOAD_BYTES, Medium, select_medium};
pub use presenter::Presenter;
