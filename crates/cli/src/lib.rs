//! `paladocs-cli` — プレゼンタ CLI。`typst`/`render`/`term`/`core` を結線した
//! 動くプレゼンタ。
//!
//! 本クレートは全ライブラリクレートを結線する唯一の層であり、ここで初めて実際の
//! **端末・スレッド・socket・プロセス境界**に触れる。下位クレートの責務（IR・描画・
//! Typst・Kitty ワイヤ）は**再実装せず**、orchestration に徹する。
//!
//! # 純粋な決定ロジック × 不純なシェル
//!
//! テスト容易性のため、決定ロジックを純粋関数へ切り出し、端末・engine・スレッド・
//! socket を不純シェルへ隔離する:
//!
//! - 純粋（単体テスト対象）: [`nav::step`]（ナビ状態機械）、[`protocol`]（socket JSON /
//!   `CSI 16 t` パーサ）、[`cli::parse_args`]（引数解析）、[`diag`]（診断 JSON）、
//!   [`restore::restore_sequence`]（端末復元シーケンス）、[`app::map_key`]（キー写像）。
//! - 不純（orchestration）: [`app`]（端末所有・入力多重化・メインループ）、
//!   [`terminal`]（viewport 計測）、[`pdf`]（`build`）。
//!
//! # サブコマンド
//!
//! - `present <ROOT.typ>` — キーボード対話プレゼン。
//! - `preview <ROOT.typ> --control <SOCKET>` — Neovim 契約（socket + キー）。
//! - `build <ROOT.typ> -o <OUT.pdf>` — PDF 書き出し（対話なし）。

pub mod app;
pub mod cli;
pub mod diag;
mod error;
pub mod nav;
pub mod pdf;
pub mod protocol;
pub mod restore;
pub mod terminal;

pub use cli::Command;
pub use error::CliError;

/// プログラム名を除いた引数列を解析し、該当サブコマンドを実行する。
pub fn run_cli(args: &[String]) -> Result<(), CliError> {
    let command = cli::parse_args(args).map_err(CliError::Usage)?;
    match command {
        Command::Present { root } => app::run_present(&root),
        Command::Preview { root, control } => app::run_preview(&root, &control),
        Command::Build { root, out } => pdf::run_build(&root, &out),
    }
}
