//! 軽量な引数解析（手書き）。サブコマンド `present` / `preview` / `build` を
//! [`Command`] へ写す純関数で、端末にもファイルにも触れない。

use std::path::PathBuf;

/// 出口レンダラの選択（`--mode`）。
///
/// 解決は起動時 1 回だけ行う（[`crate::mode::resolve_mode`]）。`Auto` は画像対応端末
/// （Knightty 等）前提で image を選ぶ。cell-mode（ANSI セル＝MDPT 表示）を出すには
/// `--mode cell` で明示強制する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// 既定。画像プロトコル経路を選ぶ。
    #[default]
    Auto,
    /// 画像プロトコル経路を強制する。
    Image,
    /// cell-mode（ANSI セル）を強制する。
    Cell,
}

impl Mode {
    /// `auto` / `image` / `cell`（小文字）を [`Mode`] へ。未知値は `None`。
    fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "image" => Some(Self::Image),
            "cell" => Some(Self::Cell),
            _ => None,
        }
    }
}

/// 解析済みサブコマンド。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// キーボード対話プレゼン。
    Present {
        /// entrypoint の `.typ`。
        root: PathBuf,
        /// 出口レンダラ選択。
        mode: Mode,
        /// cell モードで見出しを Knightty OSC 7777 で拡大するか（`--no-cell-spans` で false）。
        cell_spans: bool,
    },
    /// プレビュー。キー入力で操作し、`--control` 指定時は Neovim 契約の制御 socket
    /// からもコマンドを受ける（socket は任意）。
    Preview {
        /// entrypoint の `.typ`。
        root: PathBuf,
        /// 制御 socket のパス（省略可。無ければキー入力のみ）。
        control: Option<PathBuf>,
        /// 出口レンダラ選択。
        mode: Mode,
        /// cell モードで見出しを Knightty OSC 7777 で拡大するか（`--no-cell-spans` で false）。
        cell_spans: bool,
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
    paladocs present <ROOT.typ> [--mode auto|image|cell] [--no-cell-spans]
    paladocs preview <ROOT.typ> [--control <SOCKET>] [--mode auto|image|cell] [--no-cell-spans]
    paladocs build   <ROOT.typ> -o <OUT.pdf>";

/// プログラム名を除いた引数列 `args` を [`Command`] へ解析する。
///
/// 不正・不足はエラーメッセージ（`String`）。`--control`/`-o` は別引数でもよい。
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter();
    let sub = it.next().ok_or_else(|| "missing subcommand".to_string())?;
    match sub.as_str() {
        "present" => {
            let mut root: Option<PathBuf> = None;
            let mut mode: Option<Mode> = None;
            let mut cell_spans = true;
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "--mode" => set_mode_once(&mut mode, it.next())?,
                    "--no-cell-spans" => cell_spans = false,
                    other if other.starts_with('-') => {
                        return Err(format!("unknown flag: {other}"));
                    }
                    _ => set_once(&mut root, arg, "ROOT.typ")?,
                }
            }
            let root = root.ok_or_else(|| "present: missing ROOT.typ".to_string())?;
            Ok(Command::Present {
                root,
                mode: mode.unwrap_or_default(),
                cell_spans,
            })
        }
        "preview" => {
            let mut root: Option<PathBuf> = None;
            let mut control: Option<PathBuf> = None;
            let mut mode: Option<Mode> = None;
            let mut cell_spans = true;
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "--control" => {
                        control = Some(positional(it.next(), "SOCKET")?);
                    }
                    "--mode" => set_mode_once(&mut mode, it.next())?,
                    "--no-cell-spans" => cell_spans = false,
                    other if other.starts_with('-') => {
                        return Err(format!("unknown flag: {other}"));
                    }
                    _ => set_once(&mut root, arg, "ROOT.typ")?,
                }
            }
            let root = root.ok_or_else(|| "preview: missing ROOT.typ".to_string())?;
            Ok(Command::Preview {
                root,
                control,
                mode: mode.unwrap_or_default(),
                cell_spans,
            })
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

/// `--mode <value>` を 1 度だけ解析する。値欠落・未知値・重複はエラー。
fn set_mode_once(slot: &mut Option<Mode>, value: Option<&String>) -> Result<(), String> {
    if slot.is_some() {
        return Err("--mode specified more than once".to_string());
    }
    let value = value.ok_or_else(|| "--mode requires a value (auto|image|cell)".to_string())?;
    let mode = Mode::parse(value).ok_or_else(|| format!("unknown --mode value: {value}"))?;
    *slot = Some(mode);
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
                root: PathBuf::from("deck.typ"),
                mode: Mode::Auto,
                cell_spans: true,
            })
        );
    }

    #[test]
    fn parse_preview_with_control() {
        assert_eq!(
            parse_args(&argv(&["preview", "deck.typ", "--control", "/tmp/p.sock"])),
            Ok(Command::Preview {
                root: PathBuf::from("deck.typ"),
                control: Some(PathBuf::from("/tmp/p.sock")),
                mode: Mode::Auto,
                cell_spans: true,
            })
        );
    }

    #[test]
    fn parse_preview_control_before_root() {
        assert_eq!(
            parse_args(&argv(&["preview", "--control", "s.sock", "deck.typ"])),
            Ok(Command::Preview {
                root: PathBuf::from("deck.typ"),
                control: Some(PathBuf::from("s.sock")),
                mode: Mode::Auto,
                cell_spans: true,
            })
        );
    }

    #[test]
    fn parse_present_mode_values() {
        for (arg, want) in [
            ("auto", Mode::Auto),
            ("image", Mode::Image),
            ("cell", Mode::Cell),
        ] {
            assert_eq!(
                parse_args(&argv(&["present", "deck.typ", "--mode", arg])),
                Ok(Command::Present {
                    root: PathBuf::from("deck.typ"),
                    mode: want,
                    cell_spans: true,
                })
            );
        }
    }

    #[test]
    fn parse_present_mode_before_root() {
        assert_eq!(
            parse_args(&argv(&["present", "--mode", "cell", "deck.typ"])),
            Ok(Command::Present {
                root: PathBuf::from("deck.typ"),
                mode: Mode::Cell,
                cell_spans: true,
            })
        );
    }

    #[test]
    fn parse_preview_with_mode() {
        assert_eq!(
            parse_args(&argv(&[
                "preview",
                "deck.typ",
                "--control",
                "s.sock",
                "--mode",
                "cell",
            ])),
            Ok(Command::Preview {
                root: PathBuf::from("deck.typ"),
                control: Some(PathBuf::from("s.sock")),
                mode: Mode::Cell,
                cell_spans: true,
            })
        );
    }

    #[test]
    fn present_mode_default_is_auto() {
        let Ok(Command::Present { mode, .. }) = parse_args(&argv(&["present", "deck.typ"])) else {
            panic!("expected Present");
        };
        assert_eq!(mode, Mode::Auto);
    }

    #[test]
    fn cell_spans_default_true_and_disabled_by_flag() {
        // 既定は true。
        let Ok(Command::Present { cell_spans, .. }) = parse_args(&argv(&["present", "deck.typ"]))
        else {
            panic!("expected Present");
        };
        assert!(cell_spans);
        // --no-cell-spans で false（present・preview とも）。
        let Ok(Command::Present { cell_spans, .. }) =
            parse_args(&argv(&["present", "deck.typ", "--no-cell-spans"]))
        else {
            panic!("expected Present");
        };
        assert!(!cell_spans);
        let Ok(Command::Preview { cell_spans, .. }) =
            parse_args(&argv(&["preview", "deck.typ", "--no-cell-spans"]))
        else {
            panic!("expected Preview");
        };
        assert!(!cell_spans);
    }

    #[test]
    fn unknown_mode_value_errors() {
        assert!(parse_args(&argv(&["present", "deck.typ", "--mode", "ascii"])).is_err());
    }

    #[test]
    fn mode_missing_value_errors() {
        assert!(parse_args(&argv(&["present", "deck.typ", "--mode"])).is_err());
    }

    #[test]
    fn mode_specified_twice_errors() {
        assert!(
            parse_args(&argv(&[
                "present", "deck.typ", "--mode", "cell", "--mode", "image",
            ]))
            .is_err()
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
    fn parse_preview_without_control_is_keyboard_only() {
        // socket は任意: `preview <file>` はキー入力のみのプレビュー。
        assert_eq!(
            parse_args(&argv(&["preview", "deck.typ"])),
            Ok(Command::Preview {
                root: PathBuf::from("deck.typ"),
                control: None,
                mode: Mode::Auto,
                cell_spans: true,
            })
        );
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
