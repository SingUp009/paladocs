//! `build` サブコマンド: 対話・端末制御なしで PDF を書き出す。
//!
//! `PaladocsWorld::new(root)` → `compile_deck(&world)` → `compiled.to_pdf(&world)`
//! → `-o` のパスへ書き込む。失敗は [`crate::diag`] で診断 JSON を stderr へ出して
//! から [`CliError::Reported`]。

use std::io;
use std::path::Path;

use paladocs_typst::{EngineError, PaladocsWorld, compile_deck};

use crate::diag;
use crate::error::CliError;

/// `root` をコンパイルして PDF を `out` へ書き出す。
pub fn run_build(root: &Path, out: &Path) -> Result<(), CliError> {
    let world = PaladocsWorld::new(root).map_err(report)?;
    let compiled = compile_deck(&world).map_err(report)?;
    let pdf = compiled.to_pdf(&world).map_err(report)?;
    std::fs::write(out, pdf).map_err(CliError::Io)?;
    Ok(())
}

/// engine 失敗を診断 JSON にして「出力済み」へ畳む。
fn report(err: EngineError) -> CliError {
    let _ = diag::report_engine_error(&mut io::stderr(), &err);
    CliError::Reported
}
