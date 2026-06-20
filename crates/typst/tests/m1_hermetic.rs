//! M1 の hermetic 統合テスト（ネットワーク不要）。
//!
//! Touying を使わない最小 Typst の inline ソースを一時ディレクトリに書き出し、
//! [`Engine::compile`] からのフルパイプライン（compile → Deck → render → PDF →
//! 診断）を検証する。埋め込みフォントのみを使うためオフラインで完結する。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use paladocs_core::FrameId;
use paladocs_render::PixelSize;
use paladocs_typst::{Engine, EngineError};

/// テスト用の一意な一時ディレクトリ。Drop で再帰削除する。
struct TempProject {
    dir: PathBuf,
}

impl TempProject {
    /// `root.typ` に `source` を書いた一時プロジェクトを作る。
    fn new(source: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("paladocs-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("root.typ"), source).unwrap();
        Self { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.join("root.typ")
    }

    /// `root.typ` を上書きする（reload テスト用）。
    fn rewrite(&self, source: &str) {
        std::fs::write(self.dir.join("root.typ"), source).unwrap();
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn compile_single_page_and_validate() {
    let project = TempProject::new("= Hello, Paladocs");
    let engine = Engine::compile(&project.root()).expect("compile should succeed");
    let deck = engine.deck();
    assert_eq!(deck.frame_count(), 1);
    assert_eq!(deck.slides.len(), 1);
    assert_eq!(deck.slides[0].steps.len(), 1);
    deck.validate().expect("deck must satisfy invariants");
}

#[test]
fn compile_multi_page() {
    let project = TempProject::new("First\n#pagebreak()\nSecond\n#pagebreak()\nThird");
    let engine = Engine::compile(&project.root()).unwrap();
    assert_eq!(engine.deck().frame_count(), 3);
    // フォールバックでは各ページが 1-step スライド。
    assert_eq!(engine.deck().slides.len(), 3);
}

#[test]
fn render_frame_has_expected_size() {
    // ページサイズを固定し、ppp から決まる pixel 寸法を確認する。
    let project = TempProject::new("#set page(width: 100pt, height: 50pt, margin: 0pt)\nHi");
    let engine = Engine::compile(&project.root()).unwrap();
    let frame = engine.render_frame(FrameId(0), 2.0).unwrap();
    assert_eq!(frame.id, FrameId(0));
    // pxw = round(2.0 * 100) = 200, pxh = round(2.0 * 50) = 100
    assert_eq!(frame.image.size(), PixelSize { w: 200, h: 100 });
}

#[test]
fn render_frame_out_of_range() {
    let project = TempProject::new("= Only one page");
    let engine = Engine::compile(&project.root()).unwrap();
    let err = engine.render_frame(FrameId(5), 1.0).unwrap_err();
    assert!(matches!(err, EngineError::Render(_)), "got {err:?}");
}

#[test]
fn opaque_page_fill_roundtrips_exactly() {
    // 不透明なページ背景色がストレートアルファでそのまま出る（a=255）。
    let project = TempProject::new(
        "#set page(width: 8pt, height: 8pt, margin: 0pt, fill: rgb(20, 40, 60))\n#none",
    );
    let engine = Engine::compile(&project.root()).unwrap();
    let frame = engine.render_frame(FrameId(0), 1.0).unwrap();
    let center = frame.image.pixel(4, 4).unwrap();
    assert_eq!(center, [20, 40, 60, 255]);
}

#[test]
fn semitransparent_fill_is_straight_alpha() {
    // 半透明背景: ストレートアルファ（非プリマルチプライド）であることを確認する。
    // プリマルチプライドのままなら R ≈ round(20*128/255) ≈ 10 になるが、
    // ストレートなら R ≈ 20 に戻る。
    let project = TempProject::new(
        "#set page(width: 8pt, height: 8pt, margin: 0pt, fill: rgb(20, 40, 60, 128))\n#none",
    );
    let engine = Engine::compile(&project.root()).unwrap();
    let frame = engine.render_frame(FrameId(0), 1.0).unwrap();
    let [r, g, b, a] = frame.image.pixel(4, 4).unwrap();
    assert_eq!(a, 128, "alpha should be preserved");
    // ストレート値（~20,40,60）に近く、プリマルチプライド値（~10,20,30）から離れる。
    assert!(
        (15..=25).contains(&r),
        "r = {r} (expected ~20, not premultiplied ~10)"
    );
    assert!(
        (35..=45).contains(&g),
        "g = {g} (expected ~40, not premultiplied ~20)"
    );
    assert!(
        (55..=65).contains(&b),
        "b = {b} (expected ~60, not premultiplied ~30)"
    );
}

#[test]
fn render_fit_letterboxes_into_viewport() {
    // 100x50pt (2:1) を 200x200 ビューポートに収める → 200x100 に収まる。
    let project = TempProject::new("#set page(width: 100pt, height: 50pt, margin: 0pt)\nHi");
    let engine = Engine::compile(&project.root()).unwrap();
    let frame = engine
        .render_fit(FrameId(0), PixelSize { w: 200, h: 200 })
        .unwrap();
    let size = frame.image.size();
    // 実寸は fit 矩形と ±1px ずれうる。
    assert!((199..=201).contains(&size.w), "w = {}", size.w);
    assert!((99..=101).contains(&size.h), "h = {}", size.h);
}

#[test]
fn to_pdf_starts_with_pdf_magic() {
    let project = TempProject::new("= PDF export");
    let engine = Engine::compile(&project.root()).unwrap();
    let bytes = engine.to_pdf().unwrap();
    assert!(bytes.len() > 4);
    assert_eq!(&bytes[..4], b"%PDF");
}

#[test]
fn bad_source_yields_compile_error_with_location() {
    // 未知変数の参照は span 付きのコンパイルエラーになる。
    let project = TempProject::new("Intro line\n#undefined_variable");
    let err = match Engine::compile(&project.root()) {
        Ok(_) => panic!("expected compilation to fail"),
        Err(e) => e,
    };
    let EngineError::Compile(diags) = err else {
        panic!("expected Compile error, got {err:?}");
    };
    assert!(!diags.is_empty(), "should have at least one diagnostic");
    let d = &diags[0];
    // 2 行目を指し、行/列が 1 始まりで取れている。
    assert_eq!(d.line, 2, "diagnostic: {d:?}");
    assert!(d.col >= 1, "col should be 1-based: {d:?}");
    assert_eq!(d.file, "root.typ", "diagnostic: {d:?}");
    assert!(!d.message.is_empty());
}

#[test]
fn reload_picks_up_source_changes() {
    let project = TempProject::new("One\n#pagebreak()\nTwo");
    let mut engine = Engine::compile(&project.root()).unwrap();
    assert_eq!(engine.deck().frame_count(), 2);

    project.rewrite("Only one page now");
    engine.reload().unwrap();
    assert_eq!(engine.deck().frame_count(), 1);
}
