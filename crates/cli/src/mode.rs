//! 出口モード解決と truecolor capability 検出（純粋中心）。
//!
//! [`crate::cli::Mode`]（`--mode` 要求）を起動時 1 回だけ [`OutputMode`]（実際に使う
//! 出口）へ解決する。`Auto` は画像対応端末（Knightty 等）前提で常に image を選ぶ。
//! cell-mode（ANSI セル＝MDPT 表示）は `--mode cell` で明示強制する。
//!
//! cell-mode v1 は truecolor 前提。`COLORTERM` が truecolor を示さない場合は**警告のみ**
//! 出し、出力は truecolor のままにする（256 色ダウンコンバートは documented follow-up）。
//! 検出（env 読み取り）と判定（純関数）を分け、後者を単体テストする。

use crate::cli::Mode;

/// 解決後の出口レンダラ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Kitty graphics protocol 経路（[`crate::app`] の画像 Runner）。
    Image,
    /// ANSI セル経路（cell-mode、CellRunner）。
    Cell,
}

/// `--mode` 要求を出口へ解決する（純粋・起動時 1 回）。
///
/// `Auto` は画像対応端末前提で image を選ぶ（確定方針）。`Image`/`Cell` は明示強制。
pub fn resolve_mode(requested: Mode) -> OutputMode {
    match requested {
        Mode::Auto | Mode::Image => OutputMode::Image,
        Mode::Cell => OutputMode::Cell,
    }
}

/// `COLORTERM` の値が truecolor（24bit 直接色）対応を示すか（純粋）。
///
/// `truecolor` / `24bit` を含めば真。未設定（`None`）・その他の値は偽。
pub fn truecolor_from_env(colorterm: Option<&str>) -> bool {
    match colorterm {
        Some(v) => v.contains("truecolor") || v.contains("24bit"),
        None => false,
    }
}

/// 端末環境から truecolor 対応を検出する（不純: `COLORTERM` を読む）。
pub fn detect_truecolor() -> bool {
    truecolor_from_env(std::env::var("COLORTERM").ok().as_deref())
}

/// truecolor 非対応の警告を出すべきか（純粋）。
///
/// cell 出口かつ truecolor 非対応のときだけ真。image 出口は色を端末へ委ねるため対象外。
pub fn should_warn_truecolor(output: OutputMode, truecolor: bool) -> bool {
    output == OutputMode::Cell && !truecolor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_resolves_to_image() {
        assert_eq!(resolve_mode(Mode::Auto), OutputMode::Image);
    }

    #[test]
    fn image_forced_is_image() {
        assert_eq!(resolve_mode(Mode::Image), OutputMode::Image);
    }

    #[test]
    fn cell_forced_is_cell() {
        // 判別ペア: 同じ入力源でも cell 強制は cell、auto/image は image。
        assert_eq!(resolve_mode(Mode::Cell), OutputMode::Cell);
        assert_ne!(resolve_mode(Mode::Cell), resolve_mode(Mode::Auto));
    }

    #[test]
    fn truecolor_env_recognizes_known_values() {
        assert!(truecolor_from_env(Some("truecolor")));
        assert!(truecolor_from_env(Some("24bit")));
        // 大小・前後の付随値があっても部分一致で検出。
        assert!(truecolor_from_env(Some("truecolor:1")));
    }

    #[test]
    fn truecolor_env_rejects_others_and_unset() {
        assert!(!truecolor_from_env(None));
        assert!(!truecolor_from_env(Some("")));
        assert!(!truecolor_from_env(Some("256")));
    }

    #[test]
    fn warns_only_for_cell_without_truecolor() {
        assert!(should_warn_truecolor(OutputMode::Cell, false));
        // 判別: truecolor 有り、または image 出口では警告しない。
        assert!(!should_warn_truecolor(OutputMode::Cell, true));
        assert!(!should_warn_truecolor(OutputMode::Image, false));
        assert!(!should_warn_truecolor(OutputMode::Image, true));
    }
}
