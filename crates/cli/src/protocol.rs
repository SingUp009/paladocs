//! 純粋なワイヤパーサ: 制御 socket の行区切り JSON と、端末の `CSI 16 t`
//! （セル pixel 報告）応答。いずれも端末・socket に触れず、単体テスト可能。

use paladocs_core::FrameId;
use paladocs_term::CellSize;

use crate::nav::Action;

/// 制御 socket の 1 行（行区切り JSON）を [`Action`] へ写す。
///
/// Neovim 契約のコマンド:
/// - `{"cmd":"reload"}` → [`Action::Reload`]
/// - `{"cmd":"goto","frame":N}` → [`Action::Goto`]（`N` は非負整数）
/// - `{"cmd":"next"}` → [`Action::Advance`]
/// - `{"cmd":"prev"}` → [`Action::Retreat`]
/// - `{"cmd":"quit"}` → [`Action::Quit`]
///
/// 解析不能・未知コマンド・`frame` 欠落/不正は `None`（呼び出し側はログして無視）。
pub fn parse_command(line: &str) -> Option<Action> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    match value.get("cmd")?.as_str()? {
        "reload" => Some(Action::Reload),
        "next" => Some(Action::Advance),
        "prev" => Some(Action::Retreat),
        "quit" => Some(Action::Quit),
        "goto" => {
            let n = value.get("frame")?.as_u64()?;
            let n = u32::try_from(n).ok()?;
            Some(Action::Goto(FrameId(n)))
        }
        _ => None,
    }
}

/// `CSI 16 t`（セル pixel 寸法問い合わせ）への端末応答を解析する。
///
/// xterm 系の応答形式は **`ESC [ 6 ; <height> ; <width> t`**（高さが先、幅が後）。
/// 先頭にゴミがあっても `ESC [` から走査し、`6;<h>;<w>t` を取り出す。`height`/`width`
/// が 0 や桁あふれ、形式不一致なら `None`。
///
/// > 注: 一次情報は仕様だが、最終的な正解は Knightty 実機が返す形。Knightty は PTY の
/// > `ws_xpixel/ws_ypixel`（全テキスト領域 pixel）を設定するため、通常は
/// > [`crossterm::terminal::window_size`] で割って求まる。本パーサはその pixel が
/// > 取れない端末向けのフォールバック経路で使う。
pub fn parse_cell_size_report(bytes: &[u8]) -> Option<CellSize> {
    // `ESC [` を探す。
    let start = bytes.windows(2).position(|w| w == b"\x1b[")? + 2;
    let rest = &bytes[start..];
    // 終端 `t` まで。
    let end = rest.iter().position(|&b| b == b't')?;
    let body = std::str::from_utf8(&rest[..end]).ok()?;
    let mut parts = body.split(';');
    if parts.next()? != "6" {
        return None;
    }
    let height: u32 = parts.next()?.parse().ok()?;
    let width: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None; // 余分なフィールドがあれば不正。
    }
    if width == 0 || height == 0 {
        return None;
    }
    Some(CellSize {
        w_px: width,
        h_px: height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reload() {
        assert_eq!(parse_command(r#"{"cmd":"reload"}"#), Some(Action::Reload));
    }

    #[test]
    fn parse_next_prev_quit() {
        assert_eq!(parse_command(r#"{"cmd":"next"}"#), Some(Action::Advance));
        assert_eq!(parse_command(r#"{"cmd":"prev"}"#), Some(Action::Retreat));
        assert_eq!(parse_command(r#"{"cmd":"quit"}"#), Some(Action::Quit));
    }

    #[test]
    fn parse_goto_with_frame() {
        assert_eq!(
            parse_command(r#"{"cmd":"goto","frame":7}"#),
            Some(Action::Goto(FrameId(7)))
        );
    }

    #[test]
    fn parse_goto_missing_frame_is_none() {
        assert_eq!(parse_command(r#"{"cmd":"goto"}"#), None);
    }

    #[test]
    fn parse_goto_negative_frame_is_none() {
        assert_eq!(parse_command(r#"{"cmd":"goto","frame":-1}"#), None);
    }

    #[test]
    fn parse_unknown_cmd_is_none() {
        assert_eq!(parse_command(r#"{"cmd":"explode"}"#), None);
    }

    #[test]
    fn parse_invalid_json_is_none() {
        assert_eq!(parse_command("not json at all"), None);
        assert_eq!(parse_command(""), None);
        assert_eq!(parse_command("{}"), None);
    }

    #[test]
    fn parse_command_ignores_surrounding_whitespace() {
        assert_eq!(
            parse_command("  {\"cmd\":\"next\"}\n"),
            Some(Action::Advance)
        );
    }

    #[test]
    fn cell_size_report_width_height() {
        // ESC [ 6 ; height=20 ; width=10 t → セルは 10x20。
        assert_eq!(
            parse_cell_size_report(b"\x1b[6;20;10t"),
            Some(CellSize { w_px: 10, h_px: 20 })
        );
    }

    #[test]
    fn cell_size_report_ignores_leading_noise() {
        assert_eq!(
            parse_cell_size_report(b"junk\x1b[6;40;30t"),
            Some(CellSize { w_px: 30, h_px: 40 })
        );
    }

    #[test]
    fn cell_size_report_rejects_wrong_kind() {
        // 8 は文字数報告で、pixel 報告ではない。
        assert_eq!(parse_cell_size_report(b"\x1b[8;24;80t"), None);
    }

    #[test]
    fn cell_size_report_rejects_zero_and_malformed() {
        assert_eq!(parse_cell_size_report(b"\x1b[6;0;10t"), None);
        assert_eq!(parse_cell_size_report(b"\x1b[6;20t"), None);
        assert_eq!(parse_cell_size_report(b"\x1b[6;20;10;3t"), None);
        assert_eq!(parse_cell_size_report(b"nonsense"), None);
    }
}
