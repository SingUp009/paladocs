//! コンパイル診断の行区切り JSON 出力。
//!
//! 形は `{severity,file,line,col,message}` で、`paladocs_typst::Diagnostic`（#3）と
//! Neovim 契約に一致させる。`severity` は小文字文字列 `"error"` / `"warning"`。
//! 1 診断 = 1 行で sink（通常 stderr）へ書く。

use std::io::{self, Write};

use paladocs_typst::{Diagnostic, EngineError, Severity};

/// [`EngineError`] を診断 JSON として `sink`（通常 stderr）へ書き出す。
///
/// [`EngineError::Compile`] は各 [`Diagnostic`] を 1 行ずつ JSON 化する。その他の
/// バリアント（Render/Io/Package）は `severity:"error"`・位置不明（空 file・行列 0）
/// の 1 診断として書く。
pub fn report_engine_error(sink: &mut dyn Write, err: &EngineError) -> io::Result<()> {
    match err {
        EngineError::Compile(diags) => write_diagnostics(sink, diags),
        EngineError::Render(msg) => write_message(sink, "render", msg),
        EngineError::Io(msg) => write_message(sink, "io", msg),
        EngineError::Package(msg) => write_message(sink, "package", msg),
    }
}

/// [`Diagnostic`] 群を 1 件 1 行の JSON で書く。
pub fn write_diagnostics(sink: &mut dyn Write, diags: &[Diagnostic]) -> io::Result<()> {
    for d in diags {
        writeln!(sink, "{}", diagnostic_json(d))?;
    }
    Ok(())
}

/// 1 件の [`Diagnostic`] を JSON 文字列にする。
fn diagnostic_json(d: &Diagnostic) -> String {
    serde_json::json!({
        "severity": severity_str(d.severity),
        "file": d.file,
        "line": d.line,
        "col": d.col,
        "message": d.message,
    })
    .to_string()
}

/// 位置を持たないエラー（Render/Io/Package）を 1 行の診断 JSON にする。
fn write_message(sink: &mut dyn Write, kind: &str, msg: &str) -> io::Result<()> {
    let line = serde_json::json!({
        "severity": "error",
        "file": "",
        "line": 0,
        "col": 0,
        "message": format!("{kind}: {msg}"),
    })
    .to_string();
    writeln!(sink, "{line}")
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(severity: Severity, file: &str, line: u32, col: u32, msg: &str) -> Diagnostic {
        Diagnostic {
            severity,
            file: file.to_string(),
            line,
            col,
            message: msg.to_string(),
        }
    }

    #[test]
    fn writes_one_json_line_per_diagnostic() {
        let diags = vec![
            diag(Severity::Error, "main.typ", 3, 5, "unexpected token"),
            diag(Severity::Warning, "lib.typ", 1, 1, "unused"),
        ];
        let mut out = Vec::new();
        write_diagnostics(&mut out, &diags).unwrap();
        let s = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 2);

        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["severity"], "error");
        assert_eq!(v["file"], "main.typ");
        assert_eq!(v["line"], 3);
        assert_eq!(v["col"], 5);
        assert_eq!(v["message"], "unexpected token");

        let v2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v2["severity"], "warning");
    }

    #[test]
    fn engine_compile_error_emits_each_diagnostic() {
        let err = EngineError::Compile(vec![diag(Severity::Error, "a.typ", 2, 4, "boom")]);
        let mut out = Vec::new();
        report_engine_error(&mut out, &err).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s.lines().count(), 1);
        assert!(s.contains("\"file\":\"a.typ\""));
    }

    #[test]
    fn engine_io_error_emits_positionless_diagnostic() {
        let err = EngineError::Io("root.typ not found".to_string());
        let mut out = Vec::new();
        report_engine_error(&mut out, &err).unwrap();
        let s = String::from_utf8(out).unwrap();
        let v: serde_json::Value = serde_json::from_str(s.lines().next().unwrap()).unwrap();
        assert_eq!(v["severity"], "error");
        assert_eq!(v["line"], 0);
        assert!(v["message"].as_str().unwrap().contains("io:"));
    }
}
