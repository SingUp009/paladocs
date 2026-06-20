//! 統合テスト。実機リソースを要するものは `#[ignore]` で分離し、既定の
//! `cargo test` ではスキップする（CLAUDE.md のネットワーク/実機分離方針）。
//!
//! - [`control_socket_roundtrip`]: 実 `UnixListener` 上で行区切り JSON を流し、
//!   §6 の socket → [`Action`] 契約を端から端まで確認する（Unix のみ）。
//! - 実機 PTY + Knightty round-trip（Kitty 列の発行・クラッシュ無し・終了後の端末
//!   復元・#4 で残した 1px round-trip）は、GPU 端末を要するため本ファイルでは
//!   `present` バイナリを PTY 上で起動する手動/CI 専用手順として記す（下記コメント）。

/// 実 `UnixListener` 越しに socket → `Action` 契約を確認する。
///
/// `cargo test -- --ignored` で明示実行する。Windows では `#[cfg(unix)]` により
/// テスト本体が除外され、空のテストバイナリになる。
#[cfg(unix)]
#[test]
#[ignore = "exercises a real UnixListener; run explicitly with --ignored on a Unix host"]
fn control_socket_roundtrip() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::thread;

    use paladocs_cli::nav::Action;
    use paladocs_cli::protocol::parse_command;
    use paladocs_core::FrameId;

    let path = std::env::temp_dir().join(format!("paladocs-it-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();

    // サーバ側: 1 接続の各行を parse_command で Action へ写し、不正行は無視。
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut actions = Vec::new();
        for line in BufReader::new(stream).lines() {
            let line = line.unwrap();
            if let Some(action) = parse_command(&line) {
                actions.push(action);
            }
        }
        actions
    });

    // クライアント側: Neovim 契約のコマンド列（途中に不正行を混ぜる）。
    let mut client = UnixStream::connect(&path).unwrap();
    writeln!(client, "{}", r#"{"cmd":"next"}"#).unwrap();
    writeln!(client, "this is not json").unwrap();
    writeln!(client, "{}", r#"{"cmd":"goto","frame":3}"#).unwrap();
    writeln!(client, "{}", r#"{"cmd":"reload"}"#).unwrap();
    writeln!(client, "{}", r#"{"cmd":"quit"}"#).unwrap();
    drop(client);

    let actions = server.join().unwrap();
    assert_eq!(
        actions,
        vec![
            Action::Advance,
            Action::Goto(FrameId(3)),
            Action::Reload,
            Action::Quit,
        ]
    );
    let _ = std::fs::remove_file(&path);
}

// --- 実機 PTY + Knightty round-trip（手動/CI 専用）---
//
// 自動化は GPU 端末（Knightty）を要するため本リポジトリの hermetic CI からは外す。
// 手順:
//   1. Knightty を PTY モードで起動し、その中で `paladocs present <ROOT.typ>` を実行。
//   2. キー（Space/←/q）と制御 socket（`--control` 経由の JSON）を送り、
//      Kitty graphics 列（`ESC _ G ...`）が発行され、クラッシュしないことを確認。
//   3. `q` で終了後、端末が復元される（alt-screen 退出・カーソル表示・画像全削除・
//      raw 解除）ことを確認する。
//   4. #4 で残した 1px round-trip（1x1 画像の transmit→place→読み戻し）も
//      Knightty の Unix PTY CI 上でここに実装する。
