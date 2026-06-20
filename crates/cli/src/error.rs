//! `cli` の失敗型。診断は [`crate::diag`] が JSON で出力するため、コンパイル系の
//! 失敗は [`CliError::Reported`] として「出力済み」を表し、`main` での二重出力を避ける。

use std::fmt;
use std::io;

/// CLI 実行の失敗。
#[derive(Debug)]
pub enum CliError {
    /// 既に診断 JSON を stderr へ出力済みの失敗（`main` は追加出力しない）。
    Reported,
    /// I/O 失敗（端末制御・ファイル書き込み等）。
    Io(io::Error),
    /// 引数・使い方の誤り。`main` がメッセージを表示する。
    Usage(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reported => Ok(()),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Usage(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for CliError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
