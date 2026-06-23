//! M2 統合テスト（ネットワーク/パッケージキャッシュ依存）。
//!
//! `@preview/touying` の取得が必要なため全テストを `#[ignore]` で分離する。
//! キャッシュ済み環境で `cargo test -p paladocs-typst -- --ignored` で実行する。

use std::path::PathBuf;

use paladocs_typst::{PaladocsWorld, compile_deck};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/touying_pause.typ")
}

#[test]
#[ignore = "requires network: fetches @preview/touying"]
fn pdfpc_groups_overlays_into_steps_and_carries_notes() {
    let world = PaladocsWorld::new(&fixture()).expect("world builds");
    let compiled = compile_deck(&world).expect("touying deck should compile");
    let deck = &compiled.deck;
    deck.validate().expect("deck invariants hold");

    // 2 論理スライド: 1 枚目は #pause により 2 step、2 枚目は 1 step。
    assert_eq!(deck.slides.len(), 2, "deck = {deck:?}");
    assert_eq!(deck.slides[0].steps.len(), 2, "first slide overlays");
    assert_eq!(deck.slides[1].steps.len(), 1, "second slide");
    assert_eq!(deck.frame_count(), 3);

    // 発表順の通しフレーム番号（I3）。
    assert_eq!(deck.slides[0].steps[0].frame.0, 0);
    assert_eq!(deck.slides[0].steps[1].frame.0, 1);
    assert_eq!(deck.slides[1].steps[0].frame.0, 2);

    // 先頭スライドに発表者ノートが復元される。
    let note = deck.slides[0]
        .notes
        .as_deref()
        .expect("first slide should carry a speaker note");
    assert!(!note.is_empty(), "note = {note:?}");
}

#[test]
#[ignore = "requires network: fetches @preview/touying"]
fn touying_deck_renders_and_exports_pdf() {
    use paladocs_core::FrameId;

    let world = PaladocsWorld::new(&fixture()).expect("world builds");
    let compiled = compile_deck(&world).expect("touying deck should compile");
    // 各フレームが描画でき、PDF も出る（pdfpc とは独立に動く）。
    for i in 0..compiled.deck.frame_count() as u32 {
        let frame = compiled.render_frame(FrameId(i), 1.0).unwrap();
        assert!(frame.image.size().w > 0 && frame.image.size().h > 0);
    }
    let pdf = compiled.to_pdf(&world).unwrap();
    assert_eq!(&pdf[..4], b"%PDF");
}
