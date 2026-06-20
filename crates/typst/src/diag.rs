//! エンジンのエラー型と Typst 診断（[`SourceDiagnostic`]）のマッピング。
//!
//! [`Diagnostic`] の `{severity, file, line, col, message}` 形は、将来の nvim /
//! `cli` 側が stderr/socket へ整形する JSON 診断契約と一致させてある。

use std::fmt;

use typst::WorldExt;
use typst::diag::{Severity as TypstSeverity, SourceDiagnostic};
use typst::syntax::Source;
use typst::{World, syntax::DiagSpan};

/// 診断の深刻度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// コンパイルを失敗させるエラー。
    Error,
    /// 警告（コンパイルは継続する）。
    Warning,
}

/// 1 件の診断。ソース位置（1 始まりの行・列）とメッセージを持つ。
///
/// `span` を解決できない（detached な）診断では `file` が空文字、`line`/`col`
/// が `0` になる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// エラーか警告か。
    pub severity: Severity,
    /// 診断が指すファイルパス（プロジェクトルート相対）。解決不能なら空。
    pub file: String,
    /// 1 始まりの行番号。解決不能なら `0`。
    pub line: u32,
    /// 1 始まりの列番号（文字単位）。解決不能なら `0`。
    pub col: u32,
    /// 診断メッセージ。
    pub message: String,
}

/// エンジンが返す失敗。コンパイル/描画/IO/パッケージ取得の各失敗を集約する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// コンパイル失敗。`SourceDiagnostic` から変換した致命的エラー群。
    Compile(Vec<Diagnostic>),
    /// 描画失敗（範囲外フレーム・ラスタ化失敗など）。
    Render(String),
    /// ファイル I/O 失敗（root.typ が読めない等）。
    Io(String),
    /// パッケージ解決/取得失敗。
    Package(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(diags) => {
                write!(f, "compilation failed with {} diagnostic(s)", diags.len())?;
                for d in diags {
                    write!(f, "\n  {}:{}:{}: {}", d.file, d.line, d.col, d.message)?;
                }
                Ok(())
            }
            Self::Render(msg) => write!(f, "render error: {msg}"),
            Self::Io(msg) => write!(f, "io error: {msg}"),
            Self::Package(msg) => write!(f, "package error: {msg}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// Typst 致命的診断群を [`Diagnostic`] 群へ変換し、[`EngineError::Compile`] にする。
pub(crate) fn compile_error(world: &dyn World, diags: &[SourceDiagnostic]) -> EngineError {
    EngineError::Compile(map_diagnostics(world, diags))
}

/// `SourceDiagnostic` 群を行/列解決つきの [`Diagnostic`] 群へ変換する。
pub(crate) fn map_diagnostics(world: &dyn World, diags: &[SourceDiagnostic]) -> Vec<Diagnostic> {
    diags.iter().map(|d| map_one(world, d)).collect()
}

fn map_one(world: &dyn World, diag: &SourceDiagnostic) -> Diagnostic {
    let severity = match diag.severity {
        TypstSeverity::Error => Severity::Error,
        TypstSeverity::Warning => Severity::Warning,
    };
    let (file, line, col) = resolve(world, diag).unwrap_or_default();
    Diagnostic {
        severity,
        file,
        line,
        col,
        message: diag.message.to_string(),
    }
}

/// 診断の span を (file, line, col) へ解決する。line/col は 1 始まり。
fn resolve(world: &dyn World, diag: &SourceDiagnostic) -> Option<(String, u32, u32)> {
    let span: DiagSpan = diag.span;
    let id = span.id()?;
    let source: Source = world.source(id).ok()?;
    let range = world.range(span)?;
    let (line, col) = source.lines().byte_to_line_column(range.start)?;
    let file = id.vpath().get_without_slash().to_string();
    Some((file, line as u32 + 1, col as u32 + 1))
}
