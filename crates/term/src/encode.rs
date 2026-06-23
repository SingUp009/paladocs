//! Kitty graphics protocol の低レベルワイヤエンコーダ。
//!
//! APC 構造は `ESC _ G <key=val,...> ; <base64 payload> ESC \`（ESC=0x1b,
//! ST=`ESC \`）。本モジュールは抽象 sink（`&mut dyn Write`）へバイトを書くだけで、
//! 端末も状態も所有しない。
//!
//! Knightty 実機で確定した制約に従う:
//! - 送信は `a=t,f=32`（RGBA 直送）。`s=`(幅)/`v=`(高さ) 必須・非ゼロ。`t=d`（直接）。
//! - base64 ペイロードは **1 チャンク ≤ 4096 バイト**に分割。**先頭チャンク**に全
//!   制御キー（`i=` 含む）と複数時 `m=1`、**継続チャンクは `m=` のみ**、最終 `m=0`。
//! - 配置 `a=p` はペイロードを持たず（`;` を付けない）、単一エスケープで送る。
//!   `X`/`Y` はセル内オフセット（セルサイズ未満）、`z` は z-index、`C=1` でカーソル不動。
//! - 削除: `d=i`（小文字, ソフト＝画像データ保持）＋ `i=`＋`p=` で **1 placement のみ**削除。
//!   `d=I`（大文字, ハード＝解放）＋ `i=` で画像を破棄。
//! - 応答ノイズを避けるため全コマンドに `q=1`（成功 OK を抑制・エラーは残す）。

use crate::geometry::Placement;
use crate::medium::Medium;
use std::io::{self, Write};

/// 1 チャンクあたりの base64 最大バイト数（Knightty / Kitty 仕様）。
pub const MAX_CHUNK_BYTES: usize = 4096;

/// ペイロード付き APC を書く（`ESC _ G keys ; payload ESC \`）。
fn apc_with_payload(sink: &mut dyn Write, keys: &str, payload: &[u8]) -> io::Result<()> {
    sink.write_all(b"\x1b_G")?;
    sink.write_all(keys.as_bytes())?;
    sink.write_all(b";")?;
    sink.write_all(payload)?;
    sink.write_all(b"\x1b\\")
}

/// ペイロードなし APC を書く（`ESC _ G keys ESC \`）。
fn apc_keys_only(sink: &mut dyn Write, keys: &str) -> io::Result<()> {
    sink.write_all(b"\x1b_G")?;
    sink.write_all(keys.as_bytes())?;
    sink.write_all(b"\x1b\\")
}

/// 正準 RGBA を `a=t,f=32` で送信する（placement は別途 [`place`]）。
///
/// `payload` は `width*height*4` バイトの RGBA8 ストレート。base64 化して
/// ≤ [`MAX_CHUNK_BYTES`] のチャンクに分割し、先頭/継続/最終の `m` フラグを付ける。
/// `payload` が空のときは何も書かない。
pub fn transmit(
    sink: &mut dyn Write,
    image: u32,
    width: u32,
    height: u32,
    payload: &[u8],
) -> io::Result<()> {
    let b64 = crate::base64::encode(payload);
    let mut chunks = b64.chunks(MAX_CHUNK_BYTES).peekable();
    if chunks.peek().is_none() {
        return Ok(());
    }
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = chunks.peek().is_some();
        let m = u8::from(more);
        if first {
            let keys = format!("a=t,f=32,s={width},v={height},i={image},t=d,q=1,m={m}");
            apc_with_payload(sink, &keys, chunk)?;
            first = false;
        } else {
            let keys = format!("m={m}");
            apc_with_payload(sink, &keys, chunk)?;
        }
    }
    Ok(())
}

/// 参照 medium（`t=s` 共有メモリ / `t=f` 一時ファイル）で画像を送信する。
///
/// `reference` は共有メモリ名（`t=s`）または一時ファイルパス（`t=f`）のバイト列で、
/// base64 化してペイロードに載せる（参照は短いので単一 APC）。`f=32`・`s=`/`v=`・
/// `i=`・`t=<medium>`・`q=1`・`m=0` を付ける。
///
/// **参照オブジェクトの確保は呼び出し側の責務**（term は I/O を持たない）。画素を
/// 直送する場合は [`Backend::transmit`](crate::Backend::transmit)（`t=d`）を使うこと。
/// 本関数は `t=d` フォールバックを持たず、与えられた `medium` の `t=` を素直に書く。
pub fn transmit_reference(
    sink: &mut dyn Write,
    image: u32,
    width: u32,
    height: u32,
    medium: Medium,
    reference: &[u8],
) -> io::Result<()> {
    let b64 = crate::base64::encode(reference);
    let t = medium.wire_key();
    let keys = format!("a=t,f=32,s={width},v={height},i={image},t={t},q=1,m=0");
    apc_with_payload(sink, &keys, &b64)
}

/// 既送信画像の placement を作る（`a=p`）。
///
/// アンカーセルへ CSI CUP（1-based）でカーソルを移動してから `a=p` を送り、`C=1`
/// でカーソルを動かさない。前後を DECSC(`ESC 7`)/DECRC(`ESC 8`) で囲み、カーソル
/// 状態を保全する。
pub fn place(sink: &mut dyn Write, p: &Placement) -> io::Result<()> {
    // カーソル退避。
    sink.write_all(b"\x1b7")?;
    // アンカーセルへ移動（CUP は 1-based の row;col）。
    let row = p.cell.row.saturating_add(1);
    let col = p.cell.col.saturating_add(1);
    write!(sink, "\x1b[{row};{col}H")?;
    let keys = format!(
        "a=p,i={img},p={pid},X={x},Y={y},z={z},C=1,q=1",
        img = p.image.0,
        pid = p.id.0,
        x = p.offset.x,
        y = p.offset.y,
        z = p.z,
    );
    apc_keys_only(sink, &keys)?;
    // カーソル復帰。
    sink.write_all(b"\x1b8")
}

/// 1 placement をソフト削除する（`d=i` 小文字＋`i=`＋`p=`）。画像データは保持。
///
/// 注: Knightty の `d=p` は**セル**削除で base placement も巻き込むため、単一
/// placement の削除には `d=i`＋`p=` を使う（仕様＋実機で確定した是正点）。
pub fn delete_placement(sink: &mut dyn Write, image: u32, placement: u32) -> io::Result<()> {
    let keys = format!("a=d,d=i,i={image},p={placement},q=1");
    apc_keys_only(sink, &keys)
}

/// 画像をハード削除する（`d=I` 大文字＋`i=`）。画像データと配置を解放。
pub fn delete_image(sink: &mut dyn Write, image: u32) -> io::Result<()> {
    let keys = format!("a=d,d=I,i={image},q=1");
    apc_keys_only(sink, &keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{CellPos, PixelOffset};
    use crate::ids::{ImageId, PlacementId};

    /// 生成バイト列から APC を `(keys, payload_b64)` の列に分解する。
    fn parse_apcs(bytes: &[u8]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 3 <= bytes.len() {
            if &bytes[i..i + 3] == b"\x1b_G" {
                let start = i + 3;
                // ST (ESC \) を探す。
                let mut j = start;
                while j + 1 < bytes.len() && !(bytes[j] == 0x1b && bytes[j + 1] == b'\\') {
                    j += 1;
                }
                let body = &bytes[start..j];
                let (keys, payload) = match body.iter().position(|&b| b == b';') {
                    Some(p) => (&body[..p], &body[p + 1..]),
                    None => (body, &body[body.len()..]),
                };
                out.push((
                    String::from_utf8(keys.to_vec()).unwrap(),
                    String::from_utf8(payload.to_vec()).unwrap(),
                ));
                i = j + 2;
            } else {
                i += 1;
            }
        }
        out
    }

    #[test]
    fn transmit_single_chunk_roundtrip() {
        let pixels = vec![1, 2, 3, 4, 5, 6, 7, 8]; // 2x1 RGBA
        let mut out = Vec::new();
        transmit(&mut out, 7, 2, 1, &pixels).unwrap();
        let apcs = parse_apcs(&out);
        assert_eq!(apcs.len(), 1);
        let (keys, payload) = &apcs[0];
        for needle in ["a=t", "f=32", "i=7", "s=2", "v=1", "t=d", "m=0"] {
            assert!(keys.contains(needle), "keys {keys:?} missing {needle}");
        }
        let decoded = crate::base64::decode(payload.as_bytes()).unwrap();
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn transmit_chunks_respect_limit_and_reassemble() {
        // base64 長 > 4096 を強制（4000 bytes → 5336 b64 → 2 chunks）。
        let pixels: Vec<u8> = (0..4000u32).map(|i| (i % 251) as u8).collect();
        let mut out = Vec::new();
        transmit(&mut out, 42, 1000, 1, &pixels).unwrap();
        let apcs = parse_apcs(&out);
        assert!(
            apcs.len() >= 2,
            "expected multiple chunks, got {}",
            apcs.len()
        );

        // 先頭チャンク: 全制御キー + m=1。
        let (first_keys, _) = &apcs[0];
        for needle in ["a=t", "f=32", "i=42", "s=1000", "v=1", "t=d", "m=1"] {
            assert!(
                first_keys.contains(needle),
                "first {first_keys:?} missing {needle}"
            );
        }
        // 継続チャンク: m= のみ（制御キーを含まない）。
        for (keys, _) in &apcs[1..] {
            assert!(
                keys.starts_with("m="),
                "continuation keys must be m= only: {keys:?}"
            );
            assert!(
                !keys.contains("a="),
                "continuation must not repeat a=: {keys:?}"
            );
            assert!(
                !keys.contains("i="),
                "continuation must not repeat i=: {keys:?}"
            );
        }
        // 最終チャンクのみ m=0。
        let (last_keys, _) = apcs.last().unwrap();
        assert_eq!(last_keys, "m=0");

        // 各チャンク ≤ 4096、連結して復号すると入力に一致。
        let mut joined = String::new();
        for (_, payload) in &apcs {
            assert!(
                payload.len() <= MAX_CHUNK_BYTES,
                "chunk too big: {}",
                payload.len()
            );
            joined.push_str(payload);
        }
        let decoded = crate::base64::decode(joined.as_bytes()).unwrap();
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn place_emits_cursor_and_keys() {
        let p = Placement {
            image: ImageId(3),
            id: PlacementId(9),
            cell: CellPos { col: 4, row: 2 },
            offset: PixelOffset { x: 5, y: 6 },
            z: 7,
        };
        let mut out = Vec::new();
        place(&mut out, &p).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains('\u{1b}')); // ESC が含まれる
        assert!(s.starts_with("\x1b7"), "DECSC first"); // カーソル退避
        assert!(s.contains("\x1b[3;5H"), "CUP 1-based row;col"); // (col4,row2) → 3;5
        assert!(s.ends_with("\x1b8"), "DECRC last"); // カーソル復帰
        for needle in ["a=p", "i=3", "p=9", "X=5", "Y=6", "z=7", "C=1"] {
            assert!(s.contains(needle), "missing {needle} in {s:?}");
        }
        // 配置にペイロード区切りは不要（'a=p,...' の直後に ; を付けない）。
        assert!(
            !s.contains("C=1,q=1;"),
            "place must not emit payload separator"
        );
    }

    #[test]
    fn delete_selectors_are_soft_and_hard() {
        let mut soft = Vec::new();
        delete_placement(&mut soft, 3, 9).unwrap();
        let soft = String::from_utf8(soft).unwrap();
        assert!(soft.contains("a=d"));
        assert!(soft.contains("d=i")); // 小文字 = ソフト
        assert!(soft.contains("i=3"));
        assert!(soft.contains("p=9"));

        let mut hard = Vec::new();
        delete_image(&mut hard, 3).unwrap();
        let hard = String::from_utf8(hard).unwrap();
        assert!(hard.contains("a=d"));
        assert!(hard.contains("d=I")); // 大文字 = ハード
        assert!(hard.contains("i=3"));
        assert!(!hard.contains("p="), "image delete targets the whole image");
    }

    #[test]
    fn transmit_reference_emits_medium_key_and_b64_reference() {
        use crate::medium::Medium;

        // 共有メモリ参照: t=s、ペイロードは参照名の base64。
        let mut shm = Vec::new();
        transmit_reference(&mut shm, 7, 64, 48, Medium::SharedMem, b"/paladocs-7").unwrap();
        let apcs = parse_apcs(&shm);
        assert_eq!(apcs.len(), 1);
        let (keys, payload) = &apcs[0];
        for needle in ["a=t", "f=32", "i=7", "s=64", "v=48", "t=s", "m=0"] {
            assert!(keys.contains(needle), "keys {keys:?} missing {needle}");
        }
        assert!(!keys.contains("t=d"), "must not be direct: {keys:?}");
        let decoded = crate::base64::decode(payload.as_bytes()).unwrap();
        assert_eq!(decoded, b"/paladocs-7");

        // 一時ファイル参照: t=f。
        let mut file = Vec::new();
        transmit_reference(&mut file, 9, 10, 10, Medium::TempFile, b"/tmp/p.rgba").unwrap();
        let (keys, payload) = &parse_apcs(&file)[0];
        assert!(keys.contains("t=f"), "keys {keys:?} missing t=f");
        let decoded = crate::base64::decode(payload.as_bytes()).unwrap();
        assert_eq!(decoded, b"/tmp/p.rgba");
    }

    #[test]
    fn transmit_empty_payload_writes_nothing() {
        let mut out = Vec::new();
        transmit(&mut out, 1, 0, 0, &[]).unwrap();
        assert!(out.is_empty());
    }
}
