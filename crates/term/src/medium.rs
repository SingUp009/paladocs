//! 送出 medium（transmission medium）の選択。
//!
//! Kitty graphics protocol は画素を **直送**（`t=d`, base64）するほか、**共有メモリ**
//! （`t=s`）や **一時ファイル**（`t=f`）への参照で渡せる。後者は wire 上の base64 コピーを
//! 省けるが、参照オブジェクトの確保は term の責務外（term は I/O を持たず、バイト列を
//! sink へ書くだけ）であり、かつ対象端末が受理する必要がある。
//!
//! 本モジュールは **capability（端末が受理する medium）+ ペイロードサイズ → medium** の
//! 純粋な選択だけを担う。実際の参照確保と参照文字列の供給は上位（将来の対応 backend /
//! cli）の責務。生成される wire は [`crate::transmit_reference`]。
//!
//! 計測（Knightty `crates/proto/src/lib.rs`: `if transmission != b'd' { return
//! Err(UnsupportedFeature) }`）では `t=d` 以外は拒否されるため、Knightty 向けの
//! capability は [`Capability::DirectOnly`]。[`crate::KittyBackend`] は常に直送する。

/// 端末が受理する transfer medium の能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// 直送（`t=d`）のみ。Knightty 実測のデフォルト。
    DirectOnly,
    /// 共有メモリ参照（`t=s`）を受理する。
    SharedMem,
    /// 一時ファイル参照（`t=f`）を受理する。
    TempFile,
}

/// 実際に用いる transmission medium。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    /// `t=d`: base64 直送（常に可搬）。
    Direct,
    /// `t=s`: 共有メモリ参照。
    SharedMem,
    /// `t=f`: 一時ファイル参照。
    TempFile,
}

impl Medium {
    /// Kitty `t=` キーの値（`'d'` / `'s'` / `'f'`）。
    pub(crate) fn wire_key(self) -> char {
        match self {
            Medium::Direct => 'd',
            Medium::SharedMem => 's',
            Medium::TempFile => 'f',
        }
    }
}

/// この閾値**以下**のペイロードは、capability に関わらず直送する。
///
/// 小さい画像では参照オブジェクト確保のオーバーヘッドが base64 直送コストを上回る
/// ため、直送のほうが速い。固定 pt 閾値ではなくバイト数（解像度非依存）で判断する。
pub const DIRECT_MAX_PAYLOAD_BYTES: usize = 64 * 1024;

/// `capability` と `payload_len`（正準 RGBA バイト数）から medium を選ぶ純粋関数。
///
/// - [`Capability::DirectOnly`] は常に [`Medium::Direct`]（capability=false 相当）。
/// - それ以外でも `payload_len <= ` [`DIRECT_MAX_PAYLOAD_BYTES`] なら [`Medium::Direct`]。
/// - 大きいペイロードのみ、対応する参照 medium（[`Medium::SharedMem`] /
///   [`Medium::TempFile`]）を選ぶ。
///
/// 端末を一切触らない純関数。送出参照の確保は本関数の範囲外。
pub fn select_medium(capability: Capability, payload_len: usize) -> Medium {
    match capability {
        Capability::DirectOnly => Medium::Direct,
        _ if payload_len <= DIRECT_MAX_PAYLOAD_BYTES => Medium::Direct,
        Capability::SharedMem => Medium::SharedMem,
        Capability::TempFile => Medium::TempFile,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIG: usize = DIRECT_MAX_PAYLOAD_BYTES + 1;
    const SMALL: usize = DIRECT_MAX_PAYLOAD_BYTES;

    #[test]
    fn direct_only_capability_is_always_direct() {
        // capability=false 相当: サイズに関わらず必ず Direct（t=d）。
        assert_eq!(select_medium(Capability::DirectOnly, SMALL), Medium::Direct);
        assert_eq!(select_medium(Capability::DirectOnly, BIG), Medium::Direct);
    }

    #[test]
    fn capable_terminal_uses_reference_for_large_payloads() {
        // capability=true ＋ 大ペイロード → 参照 medium。
        // 「常に t=d を返す」実装はここで落ちる。
        assert_eq!(select_medium(Capability::SharedMem, BIG), Medium::SharedMem);
        assert_eq!(select_medium(Capability::TempFile, BIG), Medium::TempFile);
    }

    #[test]
    fn small_payloads_stay_direct_even_when_capable() {
        assert_eq!(select_medium(Capability::SharedMem, SMALL), Medium::Direct);
        assert_eq!(select_medium(Capability::TempFile, SMALL), Medium::Direct);
    }

    #[test]
    fn wire_keys_are_kitty_spec() {
        assert_eq!(Medium::Direct.wire_key(), 'd');
        assert_eq!(Medium::SharedMem.wire_key(), 's');
        assert_eq!(Medium::TempFile.wire_key(), 'f');
    }
}
