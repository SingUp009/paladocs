//! `paladocs` バイナリ。引数を解析してサブコマンドを実行する薄い入口。
//!
//! 実体は [`paladocs_cli`] ライブラリにある（テストはそちらで純粋関数として行う）。

use std::process::ExitCode;

use paladocs_cli::{CliError, run_cli};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run_cli(&args) {
        Ok(()) => ExitCode::SUCCESS,
        // 診断は既に stderr へ JSON 出力済み。追加表示しない。
        Err(CliError::Reported) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
