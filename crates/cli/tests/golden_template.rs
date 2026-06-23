//! 受け入れデッキ `examples/golden_template.typ` の構造検証（ネットワーク不要）。
//!
//! pdfpc メタを持たないプレーンデッキがフォールバック構築で 9 スライド × 1 step に
//! なることを、実コンパイルで確認する。フォント未インストール環境でも Typst の
//! フォント探索が代替へ落ちるため、ページ構造（寸法ではなく枚数）は不変。

use std::path::PathBuf;

use paladocs_typst::Engine;

#[test]
fn golden_template_is_nine_plain_slides() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/golden_template.typ");
    let engine = Engine::compile(&path).expect("golden_template should compile offline");
    let deck = engine.deck();
    assert_eq!(deck.frame_count(), 9, "golden_template must have 9 pages");
    assert_eq!(deck.slides.len(), 9, "plain deck → 9 single-step slides");
    for (i, slide) in deck.slides.iter().enumerate() {
        assert_eq!(slide.steps.len(), 1, "slide {i} must have exactly one step");
    }
    deck.validate().expect("deck must satisfy IR invariants");
}
