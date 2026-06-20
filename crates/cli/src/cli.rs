//! 軽量な引数解析（手書き）。サブコマンド `present` / `preview` / `build` を
//! [`Command`] へ写す純関数で、端末にもファイルにも触れない。

use std::path::PathBuf;

/// 解析済みサブコマンド。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// キーボード対話プレゼン。
    Present {
        /// entrypoint の `.typ`。
        root: PathBuf,
    },
    /// Neovim 契約。制御 socket からコマンド受信（キーも併用）。
    Preview {
        /// entrypoint の `.typ`。
        root: PathBuf,
        /// 制御 socket のパス。
        control: PathBuf,
    },
    /// PDF 書き出し（対話なし）。
    Build {
        /// entrypoint の `.typ`。
        root: PathBuf,
        /// 出力 PDF パス。
        out: PathBuf,
    },
}

/// 使い方文字列。
pub const USAGE: &str = "\
paladocs — Typst presenter

USAGE:
    paladocs present <ROOT.typ>
    paladocs preview <ROOT.typ> --control <SOCKET>
    paladocs build   <ROOT.typ> -o <OUT.pdf>";

/// プログラム名を除いた引数列 `args` を [`Command`] へ解析する。
///
/// 不正・不足はエラーメッセージ（`String`）。`--control`/`-o` は別引数でもよい。
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter();
    let sub = it.next().ok_or_else(|| "missing subcommand".to_string())?;
    match sub.as_str() {
        "present" => {
            let root = positional(it.next(), "ROOT.typ")?;
            if let Some(extra) = it.next() {
                return Err(format!("unexpected extra argument: {extra}"));
            }
            Ok(Command::Present { root })
        }
        "preview" => {
            let mut root: Option<PathBuf> = None;
            let mut control: Option<PathBuf> = None;
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "--control" => {
                        control = Some(positional(it.next(), "SOCKET")?);
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown flag: {other}"));
                    }
                    _ => set_once(&mut root, arg, "ROOT.typ")?,
                }
            }
            let root = root.ok_or_else(|| "preview: missing ROOT.typ".to_string())?;
            let control =
                control.ok_or_else(|| "preview: missing --control <SOCKET>".to_string())?;
            Ok(Command::Preview { root, control })
        }
        "build" => {
            let mut root: Option<PathBuf> = None;
            let mut out: Option<PathBuf> = None;
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "-o" | "--output" => {
                        out = Some(positional(it.next(), "OUT.pdf")?);
                    }
                    other if other.starts_with('-') => {
                        return Err(format!("unknown flag: {other}"));
                    }
                    _ => set_once(&mut root, arg, "ROOT.typ")?,
                }
            }
            let root = root.ok_or_else(|| "build: missing ROOT.typ".to_string())?;
            let out = out.ok_or_else(|| "build: missing -o <OUT.pdf>".to_string())?;
            Ok(Command::Build { root, out })
        }
        "-h" | "--help" | "help" => Err(USAGE.to_string()),
        other => Err(format!("unknown subcommand: {other}")),
    }
}

fn positional(arg: Option<&String>, name: &str) -> Result<PathBuf, String> {
    arg.map(PathBuf::from)
        .ok_or_else(|| format!("missing argument: {name}"))
}

fn set_once(slot: &mut Option<PathBuf>, arg: &str, name: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!(
            "unexpected extra argument: {arg} (already have {name})"
        ));
    }
    *slot = Some(PathBuf::from(arg));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_present() {
        assert_eq!(
            parse_args(&argv(&["present", "deck.typ"])),
            Ok(Command::Present {
                root: PathBuf::from("deck.typ")
            })
        );
    }

    #[test]
    fn parse_preview_with_control() {
        assert_eq!(
            parse_args(&argv(&["preview", "deck.typ", "--control", "/tmp/p.sock"])),
            Ok(Command::Preview {
                root: PathBuf::from("deck.typ"),
                control: PathBuf::from("/tmp/p.sock"),
            })
        );
    }

    #[test]
    fn parse_preview_control_before_root() {
        assert_eq!(
            parse_args(&argv(&["preview", "--control", "s.sock", "deck.typ"])),
            Ok(Command::Preview {
                root: PathBuf::from("deck.typ"),
                control: PathBuf::from("s.sock"),
            })
        );
    }

    #[test]
    fn parse_build() {
        assert_eq!(
            parse_args(&argv(&["build", "deck.typ", "-o", "out.pdf"])),
            Ok(Command::Build {
                root: PathBuf::from("deck.typ"),
                out: PathBuf::from("out.pdf"),
            })
        );
    }

    #[test]
    fn preview_missing_control_errors() {
        assert!(parse_args(&argv(&["preview", "deck.typ"])).is_err());
    }

    #[test]
    fn build_missing_output_errors() {
        assert!(parse_args(&argv(&["build", "deck.typ"])).is_err());
    }

    #[test]
    fn unknown_subcommand_errors() {
        assert!(parse_args(&argv(&["frobnicate"])).is_err());
    }

    #[test]
    fn empty_args_errors() {
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn extra_positional_errors() {
        assert!(parse_args(&argv(&["present", "a.typ", "b.typ"])).is_err());
    }
}
