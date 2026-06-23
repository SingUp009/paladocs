//! M2: Touying の **pdfpc メタデータ**から論理スライド / overlay step / 発表者
//! ノートを復元する。
//!
//! # スキーマ（実測・Touying 0.7.x / `src/pdfpc.typ`）
//!
//! Touying は文書末に `[#metadata(pdfpc)<pdfpc-file>]` を 1 つ emit する。値は
//! 辞書で、`pages` キーに各物理ページの辞書配列を持つ:
//!
//! ```text
//! (
//!   pdfpcFormat: 2,
//!   disableMarkdown: false,
//!   pages: (
//!     ( idx: 0, label: "1", overlay: 0, forcedOverlay: false, hidden: false, note: "..." ),
//!     ( idx: 1, label: "1", overlay: 1, forcedOverlay: true,  hidden: false, note: "..." ),
//!     ( idx: 2, label: "2", overlay: 0, forcedOverlay: false, hidden: false ),
//!     ...
//!   ),
//! )
//! ```
//!
//! - `idx`: 物理ページ番号（0 始まり・発表順、整数）。`FrameId` に対応。
//! - `label`: 論理スライド番号。**文字列**（例 `"1"`, `"2"`）であることに注意
//!   （実測。整数ではない）。連続する同一 `label` が 1 スライドの overlay 群。
//! - `overlay`: スライド内 overlay 番号（0 始まり）。本実装では `label` 境界での
//!   グルーピングのみ使い、`overlay` 値自体は参照しない。
//! - `note`: 発表者ノート（無いページではキーが無い）。
//! - `hidden`: 隠しスライドフラグ。
//!
//! # 設計判断
//!
//! - **ノート**: 論理スライドの**先頭 overlay** のノートを `Slide.notes` に採用する。
//! - **hidden**: 物理ページ番号（`idx`）を `FrameId` に保つため、hidden ページも
//!   step として**含める**（除外すると frame 列に隙間ができ I3 に違反する）。
//!   ナビゲーション上のスキップは上位（`cli`）の関心事とする。
//! - `label` の値はそのまま `SlideIdx` にせず、出現順に 0 始まりで採番して I1 を
//!   満たす。

use paladocs_core::{Deck, DeckMeta, FrameId, Slide, SlideIdx, Step};
use typst::foundations::Label;
use typst::foundations::Value;
use typst::introspection::{Introspector, MetadataElem};
use typst::utils::PicoStr;
use typst_layout::PagedDocument;

use crate::EngineError;
use crate::deck::validated;

/// pdfpc メタデータの label 名。
const PDFPC_LABEL: &str = "pdfpc-file";

/// パース済みの 1 物理ページ分の pdfpc 情報。
struct PdfpcPage {
    idx: u32,
    /// 論理スライド番号（pdfpc 上は文字列）。グルーピングの境界判定にのみ使う。
    label: String,
    note: Option<String>,
}

/// pdfpc メタデータから [`Deck`] を構築する。
///
/// 戻り値:
/// - `None`: pdfpc メタデータが見つからない（Touying 非使用など）→ 呼び出し側で
///   フォールバックへ。
/// - `Some(Ok(deck))`: 構築成功（[`Deck::validate`] 済み）。
/// - `Some(Err(e))`: メタデータはあるが解釈に失敗（スキーマ不整合など）。
pub(crate) fn pdfpc_deck(doc: &PagedDocument, meta: DeckMeta) -> Option<Result<Deck, EngineError>> {
    let pages = read_pages(doc)?;
    Some(build(&pages, meta))
}

/// `<pdfpc-file>` メタデータを読み、`pages` 配列をパースする。
///
/// メタデータが存在しない、または `pages` を辞書配列として取り出せない場合は
/// `None`（→ フォールバック）。個々のページに必須キー（`idx`/`label`）が欠ける
/// 場合も `None` を返す。
fn read_pages(doc: &PagedDocument) -> Option<Vec<PdfpcPage>> {
    let label = Label::new(PicoStr::intern(PDFPC_LABEL))?;
    let content = doc.introspector().query_label(label).ok()?;
    let value = &content.to_packed::<MetadataElem>()?.value;

    let Value::Dict(dict) = value else {
        return None;
    };
    let Value::Array(arr) = dict.get("pages").ok()? else {
        return None;
    };

    let mut out = Vec::with_capacity(arr.len());
    for item in arr.iter() {
        let Value::Dict(page) = item else {
            return None;
        };
        let idx = as_int(page.get("idx").ok()?)?;
        // 実測: `label` は整数ではなく文字列（"1", "2", ...）。
        let label = as_str(page.get("label").ok()?)?;
        let note = page.get("note").ok().and_then(as_str);
        out.push(PdfpcPage {
            idx: u32::try_from(idx).ok()?,
            label,
            note,
        });
    }
    Some(out)
}

/// 連続する同一 `label` のページを 1 スライドにまとめ、各ページを step にする。
fn build(pages: &[PdfpcPage], meta: DeckMeta) -> Result<Deck, EngineError> {
    let mut slides: Vec<Slide> = Vec::new();
    let mut cur_label: Option<&str> = None;
    for page in pages {
        if cur_label != Some(page.label.as_str()) {
            cur_label = Some(page.label.as_str());
            slides.push(Slide {
                index: SlideIdx(slides.len() as u32),
                steps: Vec::new(),
                // 論理スライド先頭 overlay のノートを採用する。
                notes: page.note.clone(),
            });
        }
        // 直前で必ず push しているので last_mut は Some。
        if let Some(slide) = slides.last_mut() {
            slide.steps.push(Step {
                frame: FrameId(page.idx),
            });
        }
    }
    validated(Deck { meta, slides })
}

fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        _ => None,
    }
}

fn as_str(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paladocs_core::SizePt;

    fn meta() -> DeckMeta {
        DeckMeta {
            title: None,
            page_pt: SizePt { w: 100.0, h: 75.0 },
        }
    }

    fn page(idx: u32, label: &str, note: Option<&str>) -> PdfpcPage {
        PdfpcPage {
            idx,
            label: label.to_string(),
            note: note.map(str::to_string),
        }
    }

    /// 連続同一 label が複数 step に畳まれ、frame は通し番号、ノートは先頭採用。
    #[test]
    fn groups_overlays_into_one_slide() {
        let pages = [
            page(0, "1", Some("note one")),
            page(1, "1", None),
            page(2, "2", None),
        ];
        let deck = build(&pages, meta()).unwrap();
        assert_eq!(deck.slides.len(), 2);
        assert_eq!(deck.slides[0].steps.len(), 2);
        assert_eq!(deck.slides[0].steps[0].frame, FrameId(0));
        assert_eq!(deck.slides[0].steps[1].frame, FrameId(1));
        assert_eq!(deck.slides[0].notes.as_deref(), Some("note one"));
        assert_eq!(deck.slides[1].steps.len(), 1);
        assert_eq!(deck.slides[1].steps[0].frame, FrameId(2));
        assert_eq!(deck.frame_count(), 3);
    }

    /// 先頭 overlay のノートを採用し、後続 overlay のノートは無視する。
    #[test]
    fn first_overlay_note_wins() {
        let pages = [page(0, "1", None), page(1, "1", Some("late note"))];
        let deck = build(&pages, meta()).unwrap();
        assert_eq!(deck.slides.len(), 1);
        assert_eq!(deck.slides[0].notes, None);
    }

    /// frame に隙間ができる pdfpc（idx が非連続）は validate で弾かれる。
    #[test]
    fn non_contiguous_frames_rejected() {
        let pages = [page(0, "1", None), page(2, "2", None)];
        let err = build(&pages, meta()).unwrap_err();
        assert!(matches!(err, EngineError::Render(_)), "got {err:?}");
    }

    /// 空 pages は空デッキ（validate 緑）。
    #[test]
    fn empty_pages_empty_deck() {
        let deck = build(&[], meta()).unwrap();
        assert!(deck.slides.is_empty());
        assert_eq!(deck.frame_count(), 0);
    }

    /// 実測: Touying fixture をコンパイルし、`<pdfpc-file>` メタデータの値を
    /// repr でダンプしてスナップショット（`tests/fixtures/pdfpc_schema_snapshot.txt`）
    /// に保存する。スキーマを記憶で決め打ちしないための裏取り。
    ///
    /// ネットワーク依存のため `#[ignore]`。
    #[test]
    #[ignore = "requires network: fetches @preview/touying; writes schema snapshot"]
    fn measure_pdfpc_schema_snapshot() {
        use std::path::Path;

        use typst::foundations::Repr;
        use typst_layout::PagedDocument;

        use crate::world::PaladocsWorld;

        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixture = manifest.join("tests/fixtures/touying_pause.typ");

        let world = PaladocsWorld::new(&fixture).expect("world builds");
        let typst::diag::Warned { output, .. } = typst::compile::<PagedDocument>(&world);
        let doc = output.unwrap_or_else(|d| panic!("touying compile failed: {d:?}"));

        let label = Label::new(PicoStr::intern(PDFPC_LABEL)).unwrap();
        let content = doc
            .introspector()
            .query_label(label)
            .expect("<pdfpc-file> metadata must be present");
        let value = &content.to_packed::<MetadataElem>().unwrap().value;
        let snapshot = value.repr();

        let out = manifest.join("tests/fixtures/pdfpc_schema_snapshot.txt");
        std::fs::write(&out, snapshot.as_str()).unwrap();

        // 実測したスキーマの要点（パーサが依存するキー）が含まれることを確認。
        assert!(snapshot.contains("pages"), "snapshot = {snapshot}");
        assert!(snapshot.contains("idx"));
        assert!(snapshot.contains("label"));

        // 実際にパースして 2 スライド・3 ページに畳まれることも確認。
        let pages = read_pages(&doc).expect("pages parse");
        assert_eq!(pages.len(), 3, "expected 3 physical pages");
    }
}
