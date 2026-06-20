//! `paladocs-core` — Typst スライドデッキの論理表現。
//!
//! このクレートは I/O フリー・解像度非依存・pixels を持たない論理 IR のみ
//! を扱う。Typst のコンパイル・端末描画・GPU・PDF 出力はいずれも本クレートの
//! 範囲外であり、`paladocs-typst` / `paladocs-render` / `paladocs-term` /
//! `paladocs-cli` などの上位クレートが担う。
//!
//! 中心となる [`Deck`] は外部（将来の `paladocs-typst`）が組み立て、
//! [`Deck::validate`] が以下の不変条件を検査する。
//!
//! - **I1**: 各 `slides[i].index == SlideIdx(i as u32)`
//! - **I2**: 各 `slides[i].steps` は非空
//! - **I3**: 全 [`Step`] を発表順に並べたとき、`n` 番目の Step の
//!   `frame == FrameId(n as u32)` であり、隙間も重複もない
//!
//! 空デッキ（`slides` が空）は許容され、ナビゲーション API はすべて
//! `None` を返す。
//!
//! すべてのナビゲーション API はパニックしない。範囲外の引数に対しては
//! [`Option::None`] を返す。

use std::fmt;

/// コンパイル済みデッキ。論理構造のみを保持する。
///
/// 描画解像度・ラスタ・端末プロトコルは含まれない。
#[derive(Debug, Clone)]
pub struct Deck {
    /// デッキ全体のメタデータ。
    pub meta: DeckMeta,
    /// 論理スライド列。`slides[i].index == SlideIdx(i as u32)` を満たす
    /// （不変条件 I1。[`Deck::validate`] で検査）。
    pub slides: Vec<Slide>,
}

/// デッキ全体のメタデータ。
#[derive(Debug, Clone)]
pub struct DeckMeta {
    /// デッキタイトル（任意）。
    pub title: Option<String>,
    /// Typst のページサイズ（pt）。キャンバスのアスペクトを規定する。
    /// 物理ピクセル解像度は描画時にレンダラが決める（`core` は関与しない）。
    pub page_pt: SizePt,
}

/// pt 単位の 2D サイズ。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizePt {
    /// 幅（pt）。
    pub w: f32,
    /// 高さ（pt）。
    pub h: f32,
}

/// 論理スライド。1 つ以上の overlay step（Touying `#pause`）に展開される。
#[derive(Debug, Clone)]
pub struct Slide {
    /// 論理スライド番号。配列上の位置と一致する（不変条件 I1）。
    pub index: SlideIdx,
    /// overlay step 列。常に非空（不変条件 I2）。`steps[0]` は何も
    /// reveal していない初期状態。
    pub steps: Vec<Step>,
    /// 発表者ノート（スライド単位）。
    pub notes: Option<String>,
}

/// スライド内の 1 overlay 状態。各 Step はちょうど 1 ページに対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// Typst 出力中の通しページ番号（発表順）。
    ///
    /// 位置と冗長だが、レンダラが直接参照できるよう保持する。整合性は
    /// [`Deck::validate`] が保証する（不変条件 I3）。
    pub frame: FrameId,
}

/// 0 始まりの論理スライド番号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlideIdx(pub u32);

/// 0 始まりのページ番号（Typst ドキュメント内 / 発表順）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameId(pub u32);

/// [`Deck::validate`] が返すエラー。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeckError {
    /// `slide.index` が配列位置と一致しない（不変条件 I1 違反）。
    SlideIndexMismatch {
        /// 違反が見つかった `slides` 上の位置。
        at: usize,
        /// 実際に格納されていた `slide.index`。
        found: SlideIdx,
    },
    /// `steps` が空のスライドがある（不変条件 I2 違反）。
    EmptySteps {
        /// 空 `steps` を持つスライドの `index`。
        slide: SlideIdx,
    },
    /// frame が「発表順の通し番号 0,1,2,…」と一致しない（不変条件 I3 違反）。
    FrameSequence {
        /// 期待した frame（発表順の次の通し番号）。
        expected: FrameId,
        /// 実際の frame。
        found: FrameId,
    },
}

impl fmt::Display for DeckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SlideIndexMismatch { at, found } => write!(
                f,
                "slide at position {at} has index {} (expected {at})",
                found.0
            ),
            Self::EmptySteps { slide } => {
                write!(f, "slide {} has no steps", slide.0)
            }
            Self::FrameSequence { expected, found } => write!(
                f,
                "frame sequence broken: expected FrameId({}), found FrameId({})",
                expected.0, found.0
            ),
        }
    }
}

impl std::error::Error for DeckError {}

impl Deck {
    /// 不変条件 I1〜I3 を検査する。
    ///
    /// 空デッキ（`slides.is_empty()`）は `Ok(())` を返す。
    /// 違反が複数ある場合は、I1 → I2 → I3 の順に最初に見つかったものを返す。
    pub fn validate(&self) -> Result<(), DeckError> {
        let mut next_frame: u32 = 0;
        for (i, slide) in self.slides.iter().enumerate() {
            if slide.index.0 as usize != i {
                return Err(DeckError::SlideIndexMismatch {
                    at: i,
                    found: slide.index,
                });
            }
            if slide.steps.is_empty() {
                return Err(DeckError::EmptySteps { slide: slide.index });
            }
            for step in &slide.steps {
                if step.frame.0 != next_frame {
                    return Err(DeckError::FrameSequence {
                        expected: FrameId(next_frame),
                        found: step.frame,
                    });
                }
                next_frame += 1;
            }
        }
        Ok(())
    }

    /// 描画対象フレーム総数（= 全 step 数 = Typst ページ数）。
    pub fn frame_count(&self) -> usize {
        self.slides.iter().map(|s| s.steps.len()).sum()
    }

    /// 指定 frame が属するスライドを返す。範囲外なら `None`。
    pub fn slide_of(&self, frame: FrameId) -> Option<SlideIdx> {
        let target = frame.0 as usize;
        let mut acc: usize = 0;
        for slide in &self.slides {
            let next = acc + slide.steps.len();
            if target < next {
                return Some(slide.index);
            }
            acc = next;
        }
        None
    }

    /// 指定スライドの先頭フレーム（`steps[0].frame`）。範囲外なら `None`。
    pub fn slide_first_frame(&self, slide: SlideIdx) -> Option<FrameId> {
        let s = self.slides.get(slide.0 as usize)?;
        s.steps.first().map(|st| st.frame)
    }

    /// 「次へ」: 1 フレーム進む（同一スライドの次 overlay、末尾なら次
    /// スライド先頭）。最終フレームまたは範囲外なら `None`。
    pub fn advance(&self, frame: FrameId) -> Option<FrameId> {
        let total = self.frame_count();
        let cur = frame.0 as usize;
        if cur + 1 < total {
            Some(FrameId(frame.0 + 1))
        } else {
            None
        }
    }

    /// 「戻る」: 1 フレーム戻る。先頭フレーム（または範囲外）なら `None`。
    pub fn retreat(&self, frame: FrameId) -> Option<FrameId> {
        if frame.0 == 0 {
            return None;
        }
        let total = self.frame_count();
        if (frame.0 as usize) >= total {
            return None;
        }
        Some(FrameId(frame.0 - 1))
    }

    /// 残り overlay を飛ばして次スライドの先頭へ。最終スライド内または
    /// 範囲外の `frame` なら `None`。
    pub fn next_slide(&self, frame: FrameId) -> Option<FrameId> {
        let cur = self.slide_of(frame)?;
        let next = SlideIdx(cur.0.checked_add(1)?);
        self.slide_first_frame(next)
    }

    /// 前スライドの先頭へ。先頭スライド内または範囲外の `frame` なら `None`。
    pub fn prev_slide(&self, frame: FrameId) -> Option<FrameId> {
        let cur = self.slide_of(frame)?;
        if cur.0 == 0 {
            return None;
        }
        self.slide_first_frame(SlideIdx(cur.0 - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `steps_per_slide` に従い、発表順 `FrameId(0..)` を採番してデッキを作る。
    fn deck(steps_per_slide: &[u32]) -> Deck {
        let mut slides = Vec::with_capacity(steps_per_slide.len());
        let mut frame: u32 = 0;
        for (i, &n) in steps_per_slide.iter().enumerate() {
            let mut steps = Vec::with_capacity(n as usize);
            for _ in 0..n {
                steps.push(Step {
                    frame: FrameId(frame),
                });
                frame += 1;
            }
            slides.push(Slide {
                index: SlideIdx(i as u32),
                steps,
                notes: None,
            });
        }
        Deck {
            meta: DeckMeta {
                title: None,
                page_pt: SizePt { w: 595.0, h: 842.0 },
            },
            slides,
        }
    }

    #[test]
    fn frame_count_no_overlay() {
        assert_eq!(deck(&[1, 1, 1]).frame_count(), 3);
    }

    #[test]
    fn frame_count_with_overlay() {
        assert_eq!(deck(&[2, 3]).frame_count(), 5);
    }

    #[test]
    fn slide_of_first_slide() {
        let d = deck(&[2, 3]);
        assert_eq!(d.slide_of(FrameId(0)), Some(SlideIdx(0)));
        assert_eq!(d.slide_of(FrameId(1)), Some(SlideIdx(0)));
    }

    #[test]
    fn slide_of_second_slide() {
        let d = deck(&[2, 3]);
        assert_eq!(d.slide_of(FrameId(2)), Some(SlideIdx(1)));
        assert_eq!(d.slide_of(FrameId(3)), Some(SlideIdx(1)));
        assert_eq!(d.slide_of(FrameId(4)), Some(SlideIdx(1)));
    }

    #[test]
    fn slide_of_out_of_range() {
        let d = deck(&[2, 3]);
        assert_eq!(d.slide_of(FrameId(5)), None);
        assert_eq!(d.slide_of(FrameId(100)), None);
    }

    #[test]
    fn advance_across_slide_boundary() {
        let d = deck(&[2, 3]);
        assert_eq!(d.advance(FrameId(1)), Some(FrameId(2)));
    }

    #[test]
    fn advance_at_last_frame() {
        let d = deck(&[2, 3]);
        assert_eq!(d.advance(FrameId(4)), None);
    }

    #[test]
    fn retreat_at_first_frame() {
        let d = deck(&[2, 3]);
        assert_eq!(d.retreat(FrameId(0)), None);
    }

    #[test]
    fn next_slide_from_first() {
        let d = deck(&[2, 3]);
        assert_eq!(d.next_slide(FrameId(0)), Some(FrameId(2)));
    }

    #[test]
    fn next_slide_within_last_slide() {
        let d = deck(&[2, 3]);
        assert_eq!(d.next_slide(FrameId(3)), None);
    }

    #[test]
    fn prev_slide_from_last_slide() {
        let d = deck(&[2, 3]);
        assert_eq!(d.prev_slide(FrameId(4)), Some(FrameId(0)));
    }

    #[test]
    fn slide_first_frame_second_slide() {
        let d = deck(&[2, 3]);
        assert_eq!(d.slide_first_frame(SlideIdx(1)), Some(FrameId(2)));
    }

    #[test]
    fn validate_ok() {
        assert_eq!(deck(&[2, 3]).validate(), Ok(()));
    }

    #[test]
    fn validate_empty_steps() {
        let mut d = deck(&[2, 3]);
        d.slides[1].steps.clear();
        assert_eq!(
            d.validate(),
            Err(DeckError::EmptySteps { slide: SlideIdx(1) })
        );
    }

    #[test]
    fn validate_slide_index_mismatch() {
        let mut d = deck(&[2, 3]);
        d.slides[1].index = SlideIdx(5);
        assert_eq!(
            d.validate(),
            Err(DeckError::SlideIndexMismatch {
                at: 1,
                found: SlideIdx(5),
            })
        );
    }

    #[test]
    fn validate_frame_gap() {
        let mut d = deck(&[2, 3]);
        d.slides[1].steps[1].frame = FrameId(7);
        assert_eq!(
            d.validate(),
            Err(DeckError::FrameSequence {
                expected: FrameId(3),
                found: FrameId(7),
            })
        );
    }

    #[test]
    fn empty_deck() {
        let d = deck(&[]);
        assert_eq!(d.frame_count(), 0);
        assert_eq!(d.slide_of(FrameId(0)), None);
        assert_eq!(d.advance(FrameId(0)), None);
        assert_eq!(d.retreat(FrameId(0)), None);
        assert_eq!(d.next_slide(FrameId(0)), None);
        assert_eq!(d.prev_slide(FrameId(0)), None);
        assert_eq!(d.slide_first_frame(SlideIdx(0)), None);
        assert_eq!(d.validate(), Ok(()));
    }
}
