//! 純粋なナビ決定ロジック（端末・engine・スレッド・socket を一切知らない）。
//!
//! [`step`] はこのプロジェクトの「純粋な決定ロジック × 不純なシェル」方針の中核で、
//! [`Action`] と現在状態 [`PresentState`] から**次状態と抽象描画命令 [`RenderOp`]**
//! だけを決める。実際の描画（engine 再ラスタ・term への送出）は不純シェル
//! （[`crate::app`]）が [`RenderOp`] を解釈して行う。
//!
//! # スライド境界の方針
//!
//! スライド境界をまたぐ移動は常に「clear + base 全提示」（[`RenderOp::PresentBase`]）。
//! 部分更新 diff（[`RenderOp::ApplyOverlay`]）は**スライド内 forward のみ**に限定する。
//! overlay は深さ（[`PresentState::overlay_depth`]）で管理し、retreat で 1 段戻す。

use paladocs_core::{Deck, FrameId};
use paladocs_term::Viewport;

/// 入力源（キー / socket / resize）を正規化した抽象操作。
///
/// キー・制御 socket・端末リサイズはいずれもこの 1 つのストリームへ写像され、
/// メインループは [`Action`] だけを見て [`step`] を回す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 1 フレーム進む（同一スライドの次 overlay、末尾なら次スライド先頭）。
    Advance,
    /// 1 フレーム戻る（overlay が残れば 1 段戻し、先頭なら前スライド最終へ）。
    Retreat,
    /// 残り overlay を飛ばして次スライドの先頭へ。
    NextSlide,
    /// 前スライドの先頭へ。
    PrevSlide,
    /// 指定フレームへ直接移動する。
    Goto(FrameId),
    /// ソースを再コンパイルして Deck を作り直す。
    Reload,
    /// 端末リサイズ。新しい viewport を伴う。
    Resize(Viewport),
    /// プレゼンを終了する。
    Quit,
}

/// 端末も engine も知らない、現在の提示状態。
///
/// `cur` は現在表示中フレーム、`overlay_depth` は現在フレームが属するスライド内で
/// 何ステップ目か（0 = スライド先頭、reveal 済み overlay 数に一致）。`overlay_depth`
/// は overlay placement の z 値と retreat の段数管理に使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentState {
    /// 現在表示中のフレーム。
    pub cur: FrameId,
    /// 現在フレームのスライド内オフセット（0 始まり）。
    pub overlay_depth: u32,
}

impl PresentState {
    /// 先頭フレーム（`FrameId(0)`・overlay 深さ 0）で初期化する。
    pub fn start() -> Self {
        Self {
            cur: FrameId(0),
            overlay_depth: 0,
        }
    }
}

/// [`step`] が返す抽象描画命令。不純シェルが engine + term で実行する。
///
/// fake シンクで列を記録すれば、純粋にナビ意味論を検証できる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderOp {
    /// `clear` してから `frame` を base として全提示する（スライド境界跨ぎ）。
    PresentBase(FrameId),
    /// スライド内 forward の部分更新。`from`→`to` の変化矩形のみを `z` で重ねる。
    ApplyOverlay {
        /// 直前フレーム（diff の基準）。
        from: FrameId,
        /// 新フレーム。
        to: FrameId,
        /// overlay の z-index（= 新しい overlay 深さ）。
        z: i32,
    },
    /// 最上位 overlay をソフト削除して 1 段戻す。
    RetreatOverlay,
    /// 現フレームを（resize 等で）新解像度に再ラスタし、clear + 再提示する。
    Rerender,
    /// 何もしない（範囲端・無効操作）。
    Noop,
}

/// `frame` が属するスライド内でのオフセット（0 始まり）。範囲外なら 0。
fn offset_within_slide(deck: &Deck, frame: FrameId) -> u32 {
    match deck.slide_of(frame).and_then(|s| deck.slide_first_frame(s)) {
        Some(first) => frame.0.saturating_sub(first.0),
        None => 0,
    }
}

/// 純粋: 端末にも engine にも触れず、次状態 (`state`) と描画命令だけを決める。
///
/// テストの主対象。各 [`Action`] の意味論は次のとおり:
///
/// - **Advance**: 同一スライド内なら [`RenderOp::ApplyOverlay`]（`z` は新しい深さ）、
///   別スライドへ進むなら [`RenderOp::PresentBase`]。最終フレームでは [`RenderOp::Noop`]。
/// - **Retreat**: overlay が残れば [`RenderOp::RetreatOverlay`]、スライド先頭からは
///   前スライドの**最終フレーム**を [`RenderOp::PresentBase`]。先頭フレームでは
///   [`RenderOp::Noop`]。
/// - **NextSlide / PrevSlide**: 隣接スライド先頭へ [`RenderOp::PresentBase`]、無ければ
///   [`RenderOp::Noop`]。
/// - **Goto(n)**: 範囲内なら [`RenderOp::PresentBase`]、範囲外なら [`RenderOp::Noop`]。
/// - **Reload**: `cur` を新 `frame_count` にクランプして [`RenderOp::PresentBase`]
///   （空デッキなら [`RenderOp::Noop`]）。再コンパイル自体は不純シェルが行い、
///   ここには既に**新しい `deck`** が渡る前提。
/// - **Resize**: [`RenderOp::Rerender`]。viewport の更新は不純シェルが行う。
/// - **Quit**: 描画命令なし（空 `Vec`）。ループ終了は不純シェルが判断する。
pub fn step(state: &mut PresentState, deck: &Deck, action: Action) -> Vec<RenderOp> {
    match action {
        Action::Advance => match deck.advance(state.cur) {
            None => vec![RenderOp::Noop],
            Some(next) => {
                let same_slide = deck.slide_of(state.cur) == deck.slide_of(next);
                if same_slide {
                    let from = state.cur;
                    state.overlay_depth += 1;
                    let z = state.overlay_depth as i32;
                    state.cur = next;
                    vec![RenderOp::ApplyOverlay { from, to: next, z }]
                } else {
                    state.cur = next;
                    state.overlay_depth = 0;
                    vec![RenderOp::PresentBase(next)]
                }
            }
        },
        Action::Retreat => {
            if state.overlay_depth > 0 {
                state.overlay_depth -= 1;
                state.cur = FrameId(state.cur.0 - 1);
                vec![RenderOp::RetreatOverlay]
            } else {
                match deck.retreat(state.cur) {
                    None => vec![RenderOp::Noop],
                    Some(prev) => {
                        state.cur = prev;
                        state.overlay_depth = offset_within_slide(deck, prev);
                        vec![RenderOp::PresentBase(prev)]
                    }
                }
            }
        }
        Action::NextSlide => goto_present(state, deck, deck.next_slide(state.cur)),
        Action::PrevSlide => goto_present(state, deck, deck.prev_slide(state.cur)),
        Action::Goto(n) => {
            if (n.0 as usize) < deck.frame_count() {
                goto_present(state, deck, Some(n))
            } else {
                vec![RenderOp::Noop]
            }
        }
        Action::Reload => {
            let fc = deck.frame_count();
            if fc == 0 {
                *state = PresentState::start();
                return vec![RenderOp::Noop];
            }
            if state.cur.0 as usize >= fc {
                state.cur = FrameId((fc - 1) as u32);
            }
            state.overlay_depth = offset_within_slide(deck, state.cur);
            vec![RenderOp::PresentBase(state.cur)]
        }
        Action::Resize(_) => vec![RenderOp::Rerender],
        Action::Quit => Vec::new(),
    }
}

/// 与えられた移動先（`Some`）へ全提示、`None` なら no-op。
fn goto_present(state: &mut PresentState, deck: &Deck, target: Option<FrameId>) -> Vec<RenderOp> {
    match target {
        Some(f) => {
            state.cur = f;
            state.overlay_depth = offset_within_slide(deck, f);
            vec![RenderOp::PresentBase(f)]
        }
        None => vec![RenderOp::Noop],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paladocs_core::{Deck, DeckMeta, SizePt, Slide, SlideIdx, Step};
    use paladocs_term::{CellSize, Viewport};

    /// `steps_per_slide` から発表順 `FrameId(0..)` を採番してデッキを作る。
    fn deck(steps_per_slide: &[u32]) -> Deck {
        let mut slides = Vec::new();
        let mut frame = 0u32;
        for (i, &n) in steps_per_slide.iter().enumerate() {
            let mut steps = Vec::new();
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
        let d = Deck {
            meta: DeckMeta {
                title: None,
                page_pt: SizePt { w: 100.0, h: 100.0 },
            },
            slides,
        };
        d.validate().unwrap();
        d
    }

    fn at(cur: u32, depth: u32) -> PresentState {
        PresentState {
            cur: FrameId(cur),
            overlay_depth: depth,
        }
    }

    fn viewport() -> Viewport {
        Viewport {
            cols: 80,
            rows: 24,
            cell: CellSize { w_px: 10, h_px: 20 },
        }
    }

    #[test]
    fn advance_within_slide_is_overlay_with_increasing_z() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(0, 0);
        let ops = step(&mut s, &d, Action::Advance);
        assert_eq!(
            ops,
            vec![RenderOp::ApplyOverlay {
                from: FrameId(0),
                to: FrameId(1),
                z: 1,
            }]
        );
        assert_eq!(s, at(1, 1));
    }

    #[test]
    fn advance_at_slide_end_presents_next_slide_first_frame() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(1, 1); // slide0 の末尾
        let ops = step(&mut s, &d, Action::Advance);
        assert_eq!(ops, vec![RenderOp::PresentBase(FrameId(2))]);
        assert_eq!(s, at(2, 0));
    }

    #[test]
    fn advance_at_last_frame_is_noop() {
        let d = deck(&[2, 3, 1]); // total 6, last = 5
        let mut s = at(5, 0);
        let ops = step(&mut s, &d, Action::Advance);
        assert_eq!(ops, vec![RenderOp::Noop]);
        assert_eq!(s, at(5, 0));
    }

    #[test]
    fn retreat_with_overlay_soft_deletes() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(1, 1);
        let ops = step(&mut s, &d, Action::Retreat);
        assert_eq!(ops, vec![RenderOp::RetreatOverlay]);
        assert_eq!(s, at(0, 0));
    }

    #[test]
    fn retreat_at_slide_start_presents_prev_slide_last_frame() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(2, 0); // slide1 先頭
        let ops = step(&mut s, &d, Action::Retreat);
        // 前スライド(slide0)の最終フレーム = 1、その reveal 済み深さ = 1。
        assert_eq!(ops, vec![RenderOp::PresentBase(FrameId(1))]);
        assert_eq!(s, at(1, 1));
    }

    #[test]
    fn retreat_at_first_frame_is_noop() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(0, 0);
        let ops = step(&mut s, &d, Action::Retreat);
        assert_eq!(ops, vec![RenderOp::Noop]);
        assert_eq!(s, at(0, 0));
    }

    #[test]
    fn next_slide_presents_first_frame() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(0, 0);
        let ops = step(&mut s, &d, Action::NextSlide);
        assert_eq!(ops, vec![RenderOp::PresentBase(FrameId(2))]);
        assert_eq!(s, at(2, 0));
    }

    #[test]
    fn next_slide_within_last_slide_is_noop() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(5, 0); // 最終スライド
        let ops = step(&mut s, &d, Action::NextSlide);
        assert_eq!(ops, vec![RenderOp::Noop]);
    }

    #[test]
    fn prev_slide_presents_first_frame() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(3, 1);
        let ops = step(&mut s, &d, Action::PrevSlide);
        assert_eq!(ops, vec![RenderOp::PresentBase(FrameId(0))]);
        assert_eq!(s, at(0, 0));
    }

    #[test]
    fn goto_in_range_presents_with_correct_depth() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(0, 0);
        let ops = step(&mut s, &d, Action::Goto(FrameId(3)));
        // frame3 は slide1（first=2）の 2 ステップ目 → 深さ 1。
        assert_eq!(ops, vec![RenderOp::PresentBase(FrameId(3))]);
        assert_eq!(s, at(3, 1));
    }

    #[test]
    fn goto_out_of_range_is_noop() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(2, 0);
        let ops = step(&mut s, &d, Action::Goto(FrameId(99)));
        assert_eq!(ops, vec![RenderOp::Noop]);
        assert_eq!(s, at(2, 0));
    }

    #[test]
    fn reload_clamps_cur_to_new_frame_count() {
        // 新デッキはフレーム 3 個（last = 2）。cur=10 をクランプ。
        let d = deck(&[1, 1, 1]);
        let mut s = at(10, 0);
        let ops = step(&mut s, &d, Action::Reload);
        assert_eq!(ops, vec![RenderOp::PresentBase(FrameId(2))]);
        assert_eq!(s, at(2, 0));
    }

    #[test]
    fn reload_keeps_in_range_cur() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(3, 1);
        let ops = step(&mut s, &d, Action::Reload);
        assert_eq!(ops, vec![RenderOp::PresentBase(FrameId(3))]);
        assert_eq!(s, at(3, 1));
    }

    #[test]
    fn reload_on_empty_deck_resets_and_noops() {
        let d = deck(&[]);
        let mut s = at(5, 2);
        let ops = step(&mut s, &d, Action::Reload);
        assert_eq!(ops, vec![RenderOp::Noop]);
        assert_eq!(s, PresentState::start());
    }

    #[test]
    fn resize_rerenders() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(3, 1);
        let ops = step(&mut s, &d, Action::Resize(viewport()));
        assert_eq!(ops, vec![RenderOp::Rerender]);
        // Resize は cur を変えない（解像度のみ変化）。
        assert_eq!(s, at(3, 1));
    }

    #[test]
    fn quit_returns_no_ops() {
        let d = deck(&[2, 3, 1]);
        let mut s = at(0, 0);
        let ops = step(&mut s, &d, Action::Quit);
        assert!(ops.is_empty());
    }
}
