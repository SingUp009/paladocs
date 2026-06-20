//! 公開 API を `KittyBackend` 越しに実エンコーダで駆動する統合テスト。
//!
//! ライフサイクル present → overlay（部分更新）→ retreat（ソフト `d=i`）→
//! 次スライド（ハード `d=I`）を実バイト列で検証する。実機 Knightty への
//! round-trip は `#[ignore]`（hermetic CI では走らせない）。

use paladocs_core::FrameId;
use paladocs_render::{Frame, PixelSize, Rgba};
use paladocs_term::{CellSize, KittyBackend, Presenter, Viewport};

fn solid(w: u32, h: u32, rgba: [u8; 4], id: u32) -> Frame {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        data.extend_from_slice(&rgba);
    }
    Frame {
        id: FrameId(id),
        image: Rgba::new(PixelSize { w, h }, data).unwrap(),
    }
}

fn with_pixel(base: &Frame, x: u32, y: u32, rgba: [u8; 4]) -> Frame {
    let size = base.image.size();
    let mut data = base.image.as_bytes().to_vec();
    let i = (y as usize * size.w as usize + x as usize) * 4;
    data[i..i + 4].copy_from_slice(&rgba);
    Frame {
        id: base.id,
        image: Rgba::new(size, data).unwrap(),
    }
}

fn viewport() -> Viewport {
    Viewport {
        cols: 80,
        rows: 24,
        cell: CellSize { w_px: 10, h_px: 20 },
    }
}

/// バイト列中の `needle` の開始位置（最初の一致）。
fn find(hay: &[u8], needle: &str) -> Option<usize> {
    let n = needle.as_bytes();
    hay.windows(n.len()).position(|w| w == n)
}

#[test]
fn full_lifecycle_wire_stream() {
    let mut sink: Vec<u8> = Vec::new();
    let mut p = Presenter::new(KittyBackend, viewport());

    let base = solid(600, 480, [10, 20, 30, 255], 0);
    p.present_slide(&mut sink, &base).unwrap();

    // base 送信(a=t,f=32,i=1) と placement(a=p,p=1) が出ている。
    assert!(find(&sink, "a=t,f=32,s=600,v=480,i=1").is_some());
    assert!(find(&sink, "a=p,i=1,p=1").is_some());

    // 部分更新オーバーレイ: 1px 変化 → 1x1 のみ送信、z=1 placement。
    let mark = sink.len();
    let next = with_pixel(&base, 0, 0, [99, 0, 0, 255]);
    p.apply_overlay(&mut sink, &base, &next, 1).unwrap();
    let overlay_bytes = &sink[mark..];
    assert!(
        find(overlay_bytes, "s=1,v=1,i=2").is_some(),
        "overlay transmits 1x1"
    );
    assert!(find(overlay_bytes, "a=p,i=2,p=2").is_some());
    assert!(find(overlay_bytes, "z=1").is_some());

    // retreat → ソフト削除 d=i + i=2 + p=2（画像は保持）。
    let mark = sink.len();
    p.retreat(&mut sink).unwrap();
    let retreat_bytes = &sink[mark..];
    assert!(
        find(retreat_bytes, "a=d,d=i,i=2,p=2").is_some(),
        "soft placement delete"
    );

    // 次スライド → 旧画像(1,2)をハード削除 d=I してから新提示。
    let mark = sink.len();
    let base2 = solid(600, 480, [40, 50, 60, 255], 1);
    p.present_slide(&mut sink, &base2).unwrap();
    let next_bytes = &sink[mark..];
    assert!(
        find(next_bytes, "a=d,d=I,i=1").is_some(),
        "hard delete image 1"
    );
    assert!(
        find(next_bytes, "a=d,d=I,i=2").is_some(),
        "hard delete image 2"
    );
    assert!(find(next_bytes, "i=3").is_some(), "new slide uses fresh id");
}

/// Knightty 実機への round-trip（配置の 1px 精度確認）。
///
/// PTY 経由で Knightty を起動し、既知矩形を配置して位置を確認する手動/CI 限定の
/// 統合テスト。hermetic CI では走らせない。実装は `cli`(#5) の PTY 基盤が整ってから。
#[test]
#[ignore = "requires a running Knightty terminal over a PTY"]
fn knightty_placement_roundtrip() {
    // TODO(#5): cli の Unix PTY 基盤を使い、既知矩形を配置 → 応答/フレームバッファで
    // アンカーセル + X/Y の 1px 精度を裏取りする。
}
