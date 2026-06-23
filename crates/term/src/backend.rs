//! バックエンド抽象。ライフサイクル（[`crate::Presenter`]）をバックエンド非依存に
//! 保つため、「画像送信／placement 作成／削除」をトレイト越しに語る。
//!
//! v1 の本命は [`KittyBackend`]（Kitty graphics protocol）。iTerm2 / Sixel は将来の
//! 口だけ残す。iTerm2 は placement/z 非対応のため、実装時は `render::composite` で
//! 全画面合成して全送りする縮退版になる（本ファイルでは未実装）。

use crate::geometry::Placement;
use crate::ids::{ImageId, PlacementId};
use paladocs_render::Rgba;
use std::io::{self, Write};

/// 画像送信・placement・削除を抽象する端末バックエンド。
///
/// すべてバイトを `sink` へ書くだけで、端末も状態も所有しない。ID の採番や
/// ライフサイクル管理は [`crate::Presenter`] の責務。
pub trait Backend {
    /// 正準 RGBA を送信する（Kitty では `a=t,f=32`）。placement はしない。
    fn transmit(&mut self, sink: &mut dyn Write, id: ImageId, img: &Rgba) -> io::Result<()>;

    /// 既送信画像の placement を作る（Kitty では `a=p`）。
    fn place(&mut self, sink: &mut dyn Write, p: &Placement) -> io::Result<()>;

    /// 1 placement をソフト削除する（画像データは保持）。
    fn delete_placement(
        &mut self,
        sink: &mut dyn Write,
        id: ImageId,
        pid: PlacementId,
    ) -> io::Result<()>;

    /// 画像をハード削除する（データと配置を解放）。
    fn delete_image(&mut self, sink: &mut dyn Write, id: ImageId) -> io::Result<()>;
}

/// Kitty graphics protocol バックエンド（v1 本命）。
///
/// RGBA 直送（`f=32`）を基本とし、送信と placement を分離する。状態を持たない
/// ゼロサイズ型。
#[derive(Debug, Clone, Copy, Default)]
pub struct KittyBackend;

impl Backend for KittyBackend {
    fn transmit(&mut self, sink: &mut dyn Write, id: ImageId, img: &Rgba) -> io::Result<()> {
        let size = img.size();
        crate::encode::transmit(sink, id.0, size.w, size.h, img.as_bytes())
    }

    fn place(&mut self, sink: &mut dyn Write, p: &Placement) -> io::Result<()> {
        crate::encode::place(sink, p)
    }

    fn delete_placement(
        &mut self,
        sink: &mut dyn Write,
        id: ImageId,
        pid: PlacementId,
    ) -> io::Result<()> {
        crate::encode::delete_placement(sink, id.0, pid.0)
    }

    fn delete_image(&mut self, sink: &mut dyn Write, id: ImageId) -> io::Result<()> {
        crate::encode::delete_image(sink, id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{CellPos, PixelOffset};
    use paladocs_render::PixelSize;

    #[test]
    fn kitty_transmit_uses_image_size() {
        let img = Rgba::transparent(PixelSize { w: 2, h: 3 });
        let mut out = Vec::new();
        let mut be = KittyBackend;
        be.transmit(&mut out, ImageId(5), &img).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("s=2"));
        assert!(s.contains("v=3"));
        assert!(s.contains("i=5"));
    }

    #[test]
    fn kitty_place_delegates() {
        let p = Placement {
            image: ImageId(1),
            id: PlacementId(2),
            cell: CellPos { col: 0, row: 0 },
            offset: PixelOffset { x: 0, y: 0 },
            z: 0,
        };
        let mut out = Vec::new();
        let mut be = KittyBackend;
        be.place(&mut out, &p).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("a=p"));
    }
}
