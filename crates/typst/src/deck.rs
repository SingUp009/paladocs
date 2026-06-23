//! [`Deck`] 構築の共通検証。
//!
//! 構造抽出は Touying の pdfpc メタから overlay / ノートを復元する（[`crate::pdfpc`]）。
//! pdfpc が取れないデッキは無音フォールバックせず明示エラーにする（ブリーフ §A）。
//! 構築後は必ず [`Deck::validate`] を通す。

use paladocs_core::Deck;

use crate::EngineError;

/// 構築済み Deck を検証する。違反は [`EngineError::Render`] に変換する
/// （防御的検査）。
pub(crate) fn validated(deck: Deck) -> Result<Deck, EngineError> {
    deck.validate()
        .map_err(|e| EngineError::Render(format!("deck invariant violated: {e}")))?;
    Ok(deck)
}
