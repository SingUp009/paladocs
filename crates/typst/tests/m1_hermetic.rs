//! M1 の hermetic 統合テスト（ネットワーク不要）。
//!
//! Touying を使わない最小 Typst の inline ソースを一時ディレクトリに書き出し、
//! [`compile_deck`] からのフルパイプライン（compile → Deck → render → PDF →
//! 診断 → ページ→セル投影）を検証する。埋め込みフォントのみを使うためオフライン
//! で完結する。
//!
//! 構造抽出は strict（pdfpc 必須）なので、各ソースには [`pdfpc`] ヘルパで
//! `<pdfpc-file>` メタデータを inline 注入する（Touying 非依存のまま）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use paladocs_core::FrameId;
use paladocs_render::{CellWidth, DEFAULT, PixelSize};
use paladocs_typst::{
    CompiledDeck, EngineError, PaladocsWorld, RenderOpts, compile_deck, render_step,
};

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

/// `<pdfpc-file>` メタデータ（pdfpcFormat 2）を inline 生成する。
///
/// `pages` は `(idx, label)` の列。連続する同一 `label` が 1 スライドの overlay 群
/// に畳まれる（Touying 非使用でも strict 構造抽出が通るようにするため）。
fn pdfpc(pages: &[(u32, &str)]) -> String {
    let entries: String = pages
        .iter()
        .map(|(idx, label)| format!("(idx: {idx}, label: \"{label}\", overlay: 0, hidden: false),"))
        .collect();
    format!("#metadata((pdfpcFormat: 2, pages: ({entries})))<pdfpc-file>\n")
}

/// `source` をコンパイルして `(World, CompiledDeck)` を返す（成功前提）。
fn compile_ok(project: &TempProject) -> (PaladocsWorld, CompiledDeck) {
    let world = PaladocsWorld::new(&project.root()).expect("world builds");
    let compiled = compile_deck(&world).expect("compile should succeed");
    (world, compiled)
}

#[test]
fn compile_single_page_and_validate() {
    let project = TempProject::new(&(pdfpc(&[(0, "1")]) + "= Hello, Paladocs"));
    let (_world, compiled) = compile_ok(&project);
    let deck = &compiled.deck;
    assert_eq!(deck.frame_count(), 1);
    assert_eq!(deck.slides.len(), 1);
    assert_eq!(deck.slides[0].steps.len(), 1);
    deck.validate().expect("deck must satisfy invariants");
}

#[test]
fn compile_multi_page() {
    let project = TempProject::new(
        &(pdfpc(&[(0, "1"), (1, "2"), (2, "3")])
            + "First\n#pagebreak()\nSecond\n#pagebreak()\nThird"),
    );
    let (_world, compiled) = compile_ok(&project);
    assert_eq!(compiled.deck.frame_count(), 3);
    // 各ページが独立 label なので 1-step スライド×3。
    assert_eq!(compiled.deck.slides.len(), 3);
}

#[test]
fn pdfpc_groups_overlays_into_one_slide() {
    // 連続同一 label ("1","1") は 2 step に畳まれ、"2" は別スライド 1 step。
    let project = TempProject::new(
        &(pdfpc(&[(0, "1"), (1, "1"), (2, "2")])
            + "First\n#pagebreak()\nFirst.2\n#pagebreak()\nSecond"),
    );
    let (_world, compiled) = compile_ok(&project);
    let deck = &compiled.deck;
    assert_eq!(deck.slides.len(), 2);
    assert_eq!(deck.slides[0].steps.len(), 2);
    assert_eq!(deck.slides[1].steps.len(), 1);
    assert_eq!(deck.frame_count(), 3);
    assert_eq!(deck.slides[0].steps[0].frame, FrameId(0));
    assert_eq!(deck.slides[0].steps[1].frame, FrameId(1));
    assert_eq!(deck.slides[1].steps[0].frame, FrameId(2));
}

#[test]
fn pdfpc_missing_is_explicit_error() {
    // pdfpc メタ無し（Touying 非使用）→ 無音フォールバックせず PdfpcMissing。
    let project = TempProject::new("= No pdfpc here");
    let world = PaladocsWorld::new(&project.root()).unwrap();
    let err = compile_deck(&world).unwrap_err();
    assert!(matches!(err, EngineError::PdfpcMissing), "got {err:?}");
}

#[test]
fn pdfpc_wrong_format_is_schema_error() {
    // pdfpcFormat != 2 → スキーマエラー（無音フォールバック禁止）。
    let project =
        TempProject::new("#metadata((pdfpcFormat: 1, pages: ()))<pdfpc-file>\n= Bad format");
    let world = PaladocsWorld::new(&project.root()).unwrap();
    let err = compile_deck(&world).unwrap_err();
    assert!(matches!(err, EngineError::PdfpcSchema(_)), "got {err:?}");
}

#[test]
fn render_frame_has_expected_size() {
    // ページサイズを固定し、ppp から決まる pixel 寸法を確認する。
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")]) + "#set page(width: 100pt, height: 50pt, margin: 0pt)\nHi"),
    );
    let (_world, compiled) = compile_ok(&project);
    let frame = compiled.render_frame(FrameId(0), 2.0).unwrap();
    assert_eq!(frame.id, FrameId(0));
    // pxw = round(2.0 * 100) = 200, pxh = round(2.0 * 50) = 100
    assert_eq!(frame.image.size(), PixelSize { w: 200, h: 100 });
}

#[test]
fn render_frame_out_of_range() {
    let project = TempProject::new(&(pdfpc(&[(0, "1")]) + "= Only one page"));
    let (_world, compiled) = compile_ok(&project);
    let err = compiled.render_frame(FrameId(5), 1.0).unwrap_err();
    assert!(matches!(err, EngineError::Render(_)), "got {err:?}");
}

#[test]
fn opaque_page_fill_roundtrips_exactly() {
    // 不透明なページ背景色がストレートアルファでそのまま出る（a=255）。
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")])
            + "#set page(width: 8pt, height: 8pt, margin: 0pt, fill: rgb(20, 40, 60))\n#none"),
    );
    let (_world, compiled) = compile_ok(&project);
    let frame = compiled.render_frame(FrameId(0), 1.0).unwrap();
    let center = frame.image.pixel(4, 4).unwrap();
    assert_eq!(center, [20, 40, 60, 255]);
}

#[test]
fn semitransparent_fill_is_straight_alpha() {
    // 半透明背景: ストレートアルファ（非プリマルチプライド）であることを確認する。
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")])
            + "#set page(width: 8pt, height: 8pt, margin: 0pt, fill: rgb(20, 40, 60, 128))\n#none"),
    );
    let (_world, compiled) = compile_ok(&project);
    let frame = compiled.render_frame(FrameId(0), 1.0).unwrap();
    let [r, g, b, a] = frame.image.pixel(4, 4).unwrap();
    assert_eq!(a, 128, "alpha should be preserved");
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
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")]) + "#set page(width: 100pt, height: 50pt, margin: 0pt)\nHi"),
    );
    let (_world, compiled) = compile_ok(&project);
    let frame = compiled
        .render_fit(FrameId(0), PixelSize { w: 200, h: 200 })
        .unwrap();
    let size = frame.image.size();
    assert!((199..=201).contains(&size.w), "w = {}", size.w);
    assert!((99..=101).contains(&size.h), "h = {}", size.h);
}

#[test]
fn to_pdf_starts_with_pdf_magic() {
    let project = TempProject::new(&(pdfpc(&[(0, "1")]) + "= PDF export"));
    let (world, compiled) = compile_ok(&project);
    let bytes = compiled.to_pdf(&world).unwrap();
    assert!(bytes.len() > 4);
    assert_eq!(&bytes[..4], b"%PDF");
}

#[test]
fn bad_source_yields_compile_error_with_location() {
    // 未知変数の参照は span 付きのコンパイルエラーになる（pdfpc 以前に失敗する）。
    let project = TempProject::new("Intro line\n#undefined_variable");
    let world = PaladocsWorld::new(&project.root()).unwrap();
    let err = match compile_deck(&world) {
        Ok(_) => panic!("expected compilation to fail"),
        Err(e) => e,
    };
    let EngineError::Compile(diags) = err else {
        panic!("expected Compile error, got {err:?}");
    };
    assert!(!diags.is_empty(), "should have at least one diagnostic");
    let d = &diags[0];
    assert_eq!(d.line, 2, "diagnostic: {d:?}");
    assert!(d.col >= 1, "col should be 1-based: {d:?}");
    assert_eq!(d.file, "root.typ", "diagnostic: {d:?}");
    assert!(!d.message.is_empty());
}

#[test]
fn datetime_today_is_available() {
    // `datetime.today()` を使うソースは、World が現在日付を提供するためコンパイルできる。
    let project = TempProject::new(&(pdfpc(&[(0, "1")]) + "#datetime.today().display()"));
    let world = PaladocsWorld::new(&project.root()).unwrap();
    compile_deck(&world).expect("compile with datetime.today() should succeed");
}

#[test]
fn datetime_today_with_offset_is_available() {
    let project = TempProject::new(&(pdfpc(&[(0, "1")]) + "#datetime.today(offset: 9).display()"));
    let world = PaladocsWorld::new(&project.root()).unwrap();
    compile_deck(&world).expect("compile with datetime.today(offset) should succeed");
}

#[test]
fn reload_picks_up_source_changes() {
    let project = TempProject::new(&(pdfpc(&[(0, "1"), (1, "2")]) + "One\n#pagebreak()\nTwo"));
    let mut world = PaladocsWorld::new(&project.root()).unwrap();
    let compiled = compile_deck(&world).unwrap();
    assert_eq!(compiled.deck.frame_count(), 2);

    project.rewrite(&(pdfpc(&[(0, "1")]) + "Only one page now"));
    world.reset_files();
    let compiled = compile_deck(&world).unwrap();
    assert_eq!(compiled.deck.frame_count(), 1);
}

// ---- B: ページ→セル投影（render_step）----

#[test]
fn render_step_dims_and_invariants() {
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")]) + "#set page(width: 100pt, height: 50pt, margin: 0pt)\n= Hi"),
    );
    let (_world, compiled) = compile_ok(&project);
    let opts = RenderOpts {
        cols: 20,
        rows: 8,
        pixel_per_pt: 3.0,
    };
    let grid = render_step(&compiled, 0, &opts).unwrap().grid;
    assert_eq!(grid.dims(), (20, 8)); // dims 不変条件
    for row in grid.rows() {
        for cell in row {
            // 色は不透明 truecolor（a==255）か端末既定（a==0）のいずれか。
            assert!(cell.fg[3] == 0 || cell.fg[3] == 255, "fg a∈{{0,255}}");
            assert!(cell.bg[3] == 0 || cell.bg[3] == 255, "bg a∈{{0,255}}");
            // モザイクは使わない: ▀ は出ない。
            assert_ne!(
                cell.ch, '\u{2580}',
                "semantic projection must not use mosaic"
            );
        }
        // 不変条件: Wide の右隣は必ず Continuation。
        for (c, cell) in row.iter().enumerate() {
            if cell.width == CellWidth::Wide {
                assert_eq!(row[c + 1].width, CellWidth::Continuation);
            }
        }
    }
}

#[test]
fn render_step_no_text_no_shape_is_transparent() {
    // 単色ページ・テキスト無し・図形無し → 地色は焼かれず全セル透過の空白。
    // 判別: モザイク実装なら ▀ かページ色セルになり落ちる。
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")])
            + "#set page(width: 100pt, height: 50pt, margin: 0pt, fill: rgb(20, 40, 60))\n#none"),
    );
    let (_world, compiled) = compile_ok(&project);
    let opts = RenderOpts {
        cols: 16,
        rows: 6,
        pixel_per_pt: 2.0,
    };
    let grid = render_step(&compiled, 0, &opts).unwrap().grid;
    for row in grid.rows() {
        for cell in row {
            assert_eq!(cell.ch, ' ', "no content → blank");
            assert_eq!(cell.fg, DEFAULT, "fg terminal-default (transparent)");
            assert_eq!(cell.bg, DEFAULT, "bg terminal-default (transparent)");
        }
    }
}

#[test]
fn render_step_latin_text_is_crisp_default_fg() {
    // ラテン文字を含むページ → 鮮明な文字セル（モザイク ▀ ではない）、前景は端末既定。
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")])
            + "#set page(width: 200pt, height: 40pt, margin: 4pt, fill: white)\n#text(size: 20pt)[Hello]"),
    );
    let (_world, compiled) = compile_ok(&project);
    let opts = RenderOpts {
        cols: 80,
        rows: 16,
        pixel_per_pt: 4.0,
    };
    let grid = render_step(&compiled, 0, &opts).unwrap().grid;
    let letters: Vec<&_> = grid
        .rows()
        .flat_map(|r| r.iter())
        .filter(|c| c.ch.is_ascii_alphabetic())
        .collect();
    assert!(!letters.is_empty(), "latin text should place letter cells");
    assert!(
        letters.iter().all(|c| c.fg == DEFAULT),
        "v1 text fg is terminal-default"
    );
    // モザイクは使わない。
    assert!(
        grid.rows()
            .flat_map(|r| r.iter())
            .all(|c| c.ch != '\u{2580}'),
        "no half-block mosaic"
    );
}

#[test]
fn render_step_stroked_rect_draws_box_outline() {
    // 枠線付き矩形 → アウトライン罫線（┌ 等）が出る。判別: 図形を無視する実装は落ちる。
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")])
            + "#set page(width: 120pt, height: 80pt, margin: 0pt, fill: white)\n#place(top + left, dx: 10pt, dy: 10pt, rect(width: 80pt, height: 50pt, stroke: 1pt + black))"),
    );
    let (_world, compiled) = compile_ok(&project);
    let opts = RenderOpts {
        cols: 60,
        rows: 30,
        pixel_per_pt: 2.0,
    };
    let grid = render_step(&compiled, 0, &opts).unwrap().grid;
    let box_chars = ['┌', '┐', '└', '┘', '─', '│'];
    let has_box = grid
        .rows()
        .flat_map(|r| r.iter())
        .any(|c| box_chars.contains(&c.ch));
    assert!(has_box, "stroked rect must render as box-drawing outline");
}

#[test]
fn render_step_big_heading_emits_span() {
    // 本文 10pt が支配的・見出し 30pt（ratio 3）→ 拡大 span が出る。
    // 判別: span を作らない実装は空になり落ちる。
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")])
            + "#set page(width: 200pt, height: 100pt, margin: 4pt, fill: white)\n#text(size: 30pt)[BIG]\n\n#text(size: 10pt)[body body body body body body body]"),
    );
    let (_world, compiled) = compile_ok(&project);
    let opts = RenderOpts {
        cols: 80,
        rows: 20,
        pixel_per_pt: 2.0,
    };
    let out = render_step(&compiled, 0, &opts).unwrap();
    assert!(!out.spans.is_empty(), "big heading should produce a span");
    assert!(
        out.spans.iter().any(|s| s.text.contains("BIG")),
        "span text should include the heading"
    );
    // 見出しは grid にも通常セルとして残る（フォールバック）。
    assert!(
        out.grid.rows().flat_map(|r| r.iter()).any(|c| c.ch == 'B'),
        "heading also kept as normal cells"
    );
}

#[test]
fn render_step_fill_only_rect_draws_no_box() {
    // 塗りのみ（stroke 無し）の装飾矩形 → 罫線を描かない。判別: 全 rect を罫線化する
    // 実装はここで落ちる。
    let project = TempProject::new(
        &(pdfpc(&[(0, "1")])
            + "#set page(width: 120pt, height: 80pt, margin: 0pt, fill: white)\n#place(top + left, dx: 10pt, dy: 10pt, rect(width: 80pt, height: 50pt, fill: black))"),
    );
    let (_world, compiled) = compile_ok(&project);
    let opts = RenderOpts {
        cols: 60,
        rows: 30,
        pixel_per_pt: 2.0,
    };
    let grid = render_step(&compiled, 0, &opts).unwrap().grid;
    let box_chars = ['┌', '┐', '└', '┘', '─', '│'];
    let has_box = grid
        .rows()
        .flat_map(|r| r.iter())
        .any(|c| box_chars.contains(&c.ch));
    assert!(!has_box, "fill-only rect must not draw an outline");
}
