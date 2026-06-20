//! [`Deck`] 構築。
//!
//! 二段構え:
//! - **フォールバック**（[`fallback_deck`]）: 1 ページ = 1 スライド = 1 step。
//!   pdfpc メタが取れないデッキ（Touying 非使用など）で常に動く。
//! - **pdfpc グルーピング**（M2, [`pdfpc`] モジュール）: Touying の pdfpc メタ
//!   から overlay / ノートを復元する。取得失敗時はフォールバックへ落ちる。
//!
//! いずれも構築後に必ず [`Deck::validate`] を通す。

use paladocs_core::{Deck, DeckMeta, FrameId, Slide, SlideIdx, Step};

use crate::EngineError;

/// 構築済み Deck を検証する。違反は [`EngineError::Render`] に変換する
/// （フォールバックでは自明に成立し、防御的検査）。
pub(crate) fn validated(deck: Deck) -> Result<Deck, EngineError> {
    deck.validate()
        .map_err(|e| EngineError::Render(format!("deck invariant violated: {e}")))?;
    Ok(deck)
}

/// 1 ページ = 1 スライド = 1 step のフォールバック Deck を作る。
///
/// 不変条件 I1〜I3 を自明に満たす。`page_count == 0` なら空デッキ。
pub(crate) fn fallback_deck(page_count: usize, meta: DeckMeta) -> Result<Deck, EngineError> {
    let slides = (0..page_count)
        .map(|i| Slide {
            index: SlideIdx(i as u32),
            steps: vec![Step {
                frame: FrameId(i as u32),
            }],
            notes: None,
        })
        .collect();
    validated(Deck { meta, slides })
}

#[cfg(test)]
mod tests {
    use super::*;
    use paladocs_core::SizePt;

    fn meta() -> DeckMeta {
        DeckMeta {
            title: None,
            page_pt: SizePt { w: 595.0, h: 842.0 },
        }
    }

    #[test]
    fn fallback_one_step_per_page() {
        let deck = fallback_deck(3, meta()).unwrap();
        assert_eq!(deck.slides.len(), 3);
        assert_eq!(deck.frame_count(), 3);
        for (i, slide) in deck.slides.iter().enumerate() {
            assert_eq!(slide.index, SlideIdx(i as u32));
            assert_eq!(slide.steps.len(), 1);
            assert_eq!(slide.steps[0].frame, FrameId(i as u32));
        }
    }

    #[test]
    fn fallback_empty() {
        let deck = fallback_deck(0, meta()).unwrap();
        assert!(deck.slides.is_empty());
        assert_eq!(deck.frame_count(), 0);
    }
}
