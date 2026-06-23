//! 端末復元シーケンスと panic 安全な復元ガード。
//!
//! プレゼン中はターミナルを所有する（raw mode・alt-screen・画像配置・カーソル非表示）。
//! 正常終了・通常 drop・**panic unwind のいずれでも**、端末を壊れたまま残さないために
//! 復元を確実に走らせる。復元の中身は次の順で行う:
//!
//! 1. 全画像をハード削除（Kitty `a=d,d=A`。配置と画像データを解放）。
//! 2. カーソル表示（`CSI ?25h`）。
//! 3. alt-screen 退出（`CSI ?1049l`）。
//! 4. raw mode 解除（[`crossterm::terminal::disable_raw_mode`]、実端末のみ）。
//!
//! [`restore_sequence`] は 1〜3 を sink へ書く純関数で単体テストできる。raw 解除は
//! プロセス全体の状態なので [`RawGuard`] が drop 時に行う。

use std::io::{self, Write};

/// 全画像ハード削除 → カーソル表示 → alt-screen 退出のバイト列を `sink` へ書く。
///
/// raw mode の解除は含まない（プロセス状態のため [`RawGuard`] が担当）。panic hook /
/// Drop のどちらからでも呼べるよう、副作用は sink への書き込みのみ。
pub fn restore_sequence(sink: &mut dyn Write) -> io::Result<()> {
    // 1. 全画像ハード削除（q=1 で OK 応答を抑制）。
    sink.write_all(b"\x1b_Ga=d,d=A,q=1\x1b\\")?;
    // 2. カーソル表示。
    sink.write_all(b"\x1b[?25h")?;
    // 3. alt-screen 退出。
    sink.write_all(b"\x1b[?1049l")?;
    sink.flush()
}

/// alt-screen 入場とカーソル非表示のバイト列を `sink` へ書く（[`restore_sequence`] の対）。
pub fn enter_sequence(sink: &mut dyn Write) -> io::Result<()> {
    // alt-screen 入場。
    sink.write_all(b"\x1b[?1049h")?;
    // カーソル非表示。
    sink.write_all(b"\x1b[?25l")?;
    sink.flush()
}

/// drop（panic unwind を含む）で復元シーケンスを `sink` へ書き、raw mode を解除する。
///
/// `manage_raw` が真のとき、drop で [`crossterm::terminal::disable_raw_mode`] を呼ぶ。
/// テストでは偽にして純粋なバイト列出力だけを検証する。実運用では `sink` に
/// [`std::io::Stdout`] のハンドルを渡す（global stdout なので Presenter 側の別
/// ハンドルと同じ fd を指す）。
pub struct RawGuard<W: Write> {
    sink: W,
    manage_raw: bool,
}

impl<W: Write> RawGuard<W> {
    /// ガードを作る。`manage_raw` が真なら drop 時に raw mode を解除する。
    pub fn new(sink: W, manage_raw: bool) -> Self {
        Self { sink, manage_raw }
    }
}

impl<W: Write> Drop for RawGuard<W> {
    fn drop(&mut self) {
        let _ = restore_sequence(&mut self.sink);
        if self.manage_raw {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// drop 後にバイト列を検査するための共有 sink。
    #[derive(Clone, Default)]
    struct Shared(Rc<RefCell<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn restore_sequence_emits_delete_cursor_altscreen_in_order() {
        let mut out = Vec::new();
        restore_sequence(&mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        let del = s.find("\x1b_Ga=d,d=A").expect("delete-all images");
        let cursor = s.find("\x1b[?25h").expect("show cursor");
        let alt = s.find("\x1b[?1049l").expect("leave alt-screen");
        assert!(del < cursor && cursor < alt, "wrong order: {s:?}");
    }

    #[test]
    fn raw_guard_drop_emits_restore_sequence() {
        let shared = Shared::default();
        {
            let _guard = RawGuard::new(shared.clone(), false);
            // まだ何も書かれていない。
            assert!(shared.0.borrow().is_empty());
        } // ここで drop → 復元シーケンス。
        let s = String::from_utf8(shared.0.borrow().clone()).unwrap();
        assert!(s.contains("\x1b_Ga=d,d=A"));
        assert!(s.contains("\x1b[?25h"));
        assert!(s.contains("\x1b[?1049l"));
    }
}
