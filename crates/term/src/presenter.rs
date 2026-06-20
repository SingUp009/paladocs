//! スライド提示のライフサイクル管理（バックエンド非依存）。
//!
//! [`Presenter`] は sink + backend + ID 割当器 + 現在スライドの (image/placement)
//! 状態を持ち、present / overlay（部分更新）/ retreat / clear / resize を提供する。
//! 画像/placement を確実に解放し、リークさせない。

use crate::backend::Backend;
use crate::geometry::{Placement, Viewport, place_geometry};
use crate::ids::{IdAllocator, ImageId, PlacementId};
use paladocs_render::{Frame, Rect, changed_region, crop, fit};
use std::io::{self, Write};

/// 現在提示中スライドの内部状態。
struct SlideState {
    /// base の配置矩形（fit 結果）。オーバーレイのオフセット基準に使う。
    base_rect: Rect,
    /// このスライドで送信した全画像（base + 全オーバーレイ）。clear で全ハード削除。
    all_images: Vec<ImageId>,
    /// 提示順のオーバーレイ placement スタック（retreat で上から戻す）。
    overlays: Vec<(ImageId, PlacementId)>,
}

/// スライド提示のステートフルなドライバ。
///
/// バックエンド `B` 越しに画像送信・placement・削除を行う。viewport は構築時に
/// 渡し、[`Presenter::resize`] で更新する。
pub struct Presenter<B: Backend> {
    backend: B,
    ids: IdAllocator,
    viewport: Viewport,
    slide: Option<SlideState>,
}

impl<B: Backend> Presenter<B> {
    /// バックエンドと viewport から構築する。スライド未提示の状態。
    pub fn new(backend: B, viewport: Viewport) -> Self {
        Self {
            backend,
            ids: IdAllocator::new(),
            viewport,
            slide: None,
        }
    }

    /// 現在の viewport。
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// バックエンドへの参照（テスト・検査用）。
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// スライド base を提示する。
    ///
    /// 既に提示中スライドがあれば先に [`clear_slide`](Self::clear_slide) する。
    /// `fit(base.image.size(), viewport)` で配置矩形を決め、新 image を `transmit`
    /// → そのアンカーに `z=0` で `place`。状態を記録する。
    pub fn present_slide(&mut self, sink: &mut dyn Write, base: &Frame) -> io::Result<()> {
        if self.slide.is_some() {
            self.clear_slide(sink)?;
        }
        let base_rect = fit(base.image.size(), self.viewport.pixel_size());
        let image = self.ids.next_image();
        self.backend.transmit(sink, image, &base.image)?;
        let (cell, offset) = place_geometry(base_rect.x, base_rect.y, self.viewport.cell)
            .ok_or_else(invalid_geometry)?;
        let pid = self.ids.next_placement();
        self.backend.place(
            sink,
            &Placement {
                image,
                id: pid,
                cell,
                offset,
                z: 0,
            },
        )?;
        self.slide = Some(SlideState {
            base_rect,
            all_images: vec![image],
            overlays: Vec::new(),
        });
        Ok(())
    }

    /// オーバーレイ（reveal の 1 ステップ）を部分更新で適用する。
    ///
    /// `changed_region(prev, next)` が `None`（変化なし）なら no-op。変化矩形を
    /// `crop` して**その部分画像だけ**を新 image として `transmit` し、base アンカー
    /// ＋矩形オフセットの位置へ `z=step` で `place`。**全再送しない**。
    ///
    /// 提示中スライドが無いときは [`io::ErrorKind::Other`] を返す。
    pub fn apply_overlay(
        &mut self,
        sink: &mut dyn Write,
        prev: &Frame,
        next: &Frame,
        step: i32,
    ) -> io::Result<()> {
        let base_rect = match &self.slide {
            Some(s) => s.base_rect,
            None => {
                return Err(io::Error::other("apply_overlay without a current slide"));
            }
        };
        let rect = match changed_region(prev, next) {
            Some(r) => r,
            None => return Ok(()),
        };
        let sub = crop(&next.image, rect).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "changed region out of bounds")
        })?;
        let image = self.ids.next_image();
        self.backend.transmit(sink, image, &sub)?;
        let origin_x = base_rect.x.saturating_add(rect.x);
        let origin_y = base_rect.y.saturating_add(rect.y);
        let (cell, offset) =
            place_geometry(origin_x, origin_y, self.viewport.cell).ok_or_else(invalid_geometry)?;
        let pid = self.ids.next_placement();
        self.backend.place(
            sink,
            &Placement {
                image,
                id: pid,
                cell,
                offset,
                z: step,
            },
        )?;
        // ここまで成功してから状態を更新（途中失敗で不整合を残さない）。
        let state = self.slide.as_mut().expect("slide present checked above");
        state.all_images.push(image);
        state.overlays.push((image, pid));
        Ok(())
    }

    /// 最上位オーバーレイの reveal を 1 段戻す。
    ///
    /// 最上位 placement を**ソフト削除**（画像データは保持）。base は保持する。
    /// 戻すオーバーレイが無い場合は no-op。画像 ID は `all_images` に残り、
    /// [`clear_slide`](Self::clear_slide) で確実にハード回収される。
    pub fn retreat(&mut self, sink: &mut dyn Write) -> io::Result<()> {
        let popped = match self.slide.as_mut() {
            Some(s) => s.overlays.pop(),
            None => return Ok(()),
        };
        if let Some((image, pid)) = popped {
            self.backend.delete_placement(sink, image, pid)?;
        }
        Ok(())
    }

    /// 現在スライドの全画像をハード削除し、状態をリセットする。
    ///
    /// `all_images`（base + 全オーバーレイ）を漏れなく `delete_image` するため、
    /// placement の無い image が溜まらない（quota 回収に協調）。
    pub fn clear_slide(&mut self, sink: &mut dyn Write) -> io::Result<()> {
        if let Some(state) = self.slide.take() {
            for image in state.all_images {
                self.backend.delete_image(sink, image)?;
            }
        }
        Ok(())
    }

    /// viewport を更新し、新解像度で base を再提示する。
    ///
    /// 現在スライドをハードクリアしてから新 `viewport` で `present_slide` する。
    /// `base` は `cli` が新解像度で再ラスタしたものを渡す。オーバーレイは `cli` が
    /// 続けて [`apply_overlay`](Self::apply_overlay) で再適用する（term は overlay
    /// 列の内容を保持しない）。
    pub fn resize(
        &mut self,
        sink: &mut dyn Write,
        viewport: Viewport,
        base: &Frame,
    ) -> io::Result<()> {
        self.clear_slide(sink)?;
        self.viewport = viewport;
        self.present_slide(sink, base)
    }
}

fn invalid_geometry() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "invalid cell geometry (zero or oversized cell size)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::CellSize;
    use paladocs_core::FrameId;
    use paladocs_render::{PixelSize, Rgba};

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

    /// バックエンド呼び出しを記録する fake。
    #[derive(Default)]
    struct Recorder {
        events: Vec<Ev>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Ev {
        Transmit { id: u32, w: u32, h: u32 },
        Place { image: u32, pid: u32, z: i32 },
        DeletePlacement { image: u32, pid: u32 },
        DeleteImage { image: u32 },
    }

    impl Backend for Recorder {
        fn transmit(&mut self, _sink: &mut dyn Write, id: ImageId, img: &Rgba) -> io::Result<()> {
            let s = img.size();
            self.events.push(Ev::Transmit {
                id: id.0,
                w: s.w,
                h: s.h,
            });
            Ok(())
        }
        fn place(&mut self, _sink: &mut dyn Write, p: &Placement) -> io::Result<()> {
            self.events.push(Ev::Place {
                image: p.image.0,
                pid: p.id.0,
                z: p.z,
            });
            Ok(())
        }
        fn delete_placement(
            &mut self,
            _sink: &mut dyn Write,
            id: ImageId,
            pid: PlacementId,
        ) -> io::Result<()> {
            self.events.push(Ev::DeletePlacement {
                image: id.0,
                pid: pid.0,
            });
            Ok(())
        }
        fn delete_image(&mut self, _sink: &mut dyn Write, id: ImageId) -> io::Result<()> {
            self.events.push(Ev::DeleteImage { image: id.0 });
            Ok(())
        }
    }

    fn viewport() -> Viewport {
        // 800x480 px。content 600x480 を入れると fit scale 1（ネイティブ配置）。
        Viewport {
            cols: 80,
            rows: 24,
            cell: CellSize { w_px: 10, h_px: 20 },
        }
    }

    #[test]
    fn present_transmits_then_places_z0() {
        let mut p = Presenter::new(Recorder::default(), viewport());
        let mut sink = Vec::new();
        let base = solid(600, 480, [10, 20, 30, 255], 0);
        p.present_slide(&mut sink, &base).unwrap();
        assert_eq!(
            p.backend().events,
            vec![
                Ev::Transmit {
                    id: 1,
                    w: 600,
                    h: 480
                },
                Ev::Place {
                    image: 1,
                    pid: 1,
                    z: 0
                },
            ]
        );
    }

    #[test]
    fn overlay_sends_only_changed_region_with_higher_z() {
        let mut p = Presenter::new(Recorder::default(), viewport());
        let mut sink = Vec::new();
        let base = solid(600, 480, [10, 20, 30, 255], 0);
        p.present_slide(&mut sink, &base).unwrap();

        let next = with_pixel(&base, 0, 0, [99, 0, 0, 255]);
        p.apply_overlay(&mut sink, &base, &next, 1).unwrap();

        // base 全送り(600x480) のあと、変化矩形 1x1 のみ送信。
        assert_eq!(
            p.backend().events[2..],
            [
                Ev::Transmit { id: 2, w: 1, h: 1 },
                Ev::Place {
                    image: 2,
                    pid: 2,
                    z: 1
                },
            ]
        );
    }

    #[test]
    fn overlay_noop_when_identical() {
        let mut p = Presenter::new(Recorder::default(), viewport());
        let mut sink = Vec::new();
        let base = solid(600, 480, [10, 20, 30, 255], 0);
        p.present_slide(&mut sink, &base).unwrap();
        let before = p.backend().events.len();
        p.apply_overlay(&mut sink, &base, &base, 1).unwrap();
        assert_eq!(
            p.backend().events.len(),
            before,
            "identical frames → no transmit/place"
        );
    }

    #[test]
    fn retreat_soft_deletes_top_placement() {
        let mut p = Presenter::new(Recorder::default(), viewport());
        let mut sink = Vec::new();
        let base = solid(600, 480, [10, 20, 30, 255], 0);
        p.present_slide(&mut sink, &base).unwrap();
        let next = with_pixel(&base, 0, 0, [99, 0, 0, 255]);
        p.apply_overlay(&mut sink, &base, &next, 1).unwrap();
        p.retreat(&mut sink).unwrap();
        assert_eq!(
            p.backend().events.last(),
            Some(&Ev::DeletePlacement { image: 2, pid: 2 })
        );
    }

    #[test]
    fn next_slide_hard_deletes_all_images_no_leak() {
        let mut p = Presenter::new(Recorder::default(), viewport());
        let mut sink = Vec::new();
        let base = solid(600, 480, [10, 20, 30, 255], 0);
        p.present_slide(&mut sink, &base).unwrap();
        let next = with_pixel(&base, 0, 0, [99, 0, 0, 255]);
        p.apply_overlay(&mut sink, &base, &next, 1).unwrap();
        p.retreat(&mut sink).unwrap();

        // 次スライド提示 → 旧スライドの全画像(1,2)をハード削除してから新提示。
        let base2 = solid(600, 480, [40, 50, 60, 255], 1);
        p.present_slide(&mut sink, &base2).unwrap();

        let ev = &p.backend().events;
        // retreat 直後から: DeleteImage 1, DeleteImage 2（順不同だが両方）, 次に Transmit 3。
        let deletes: Vec<&Ev> = ev
            .iter()
            .filter(|e| matches!(e, Ev::DeleteImage { .. }))
            .collect();
        assert!(deletes.contains(&&Ev::DeleteImage { image: 1 }));
        assert!(deletes.contains(&&Ev::DeleteImage { image: 2 }));
        assert_eq!(
            deletes.len(),
            2,
            "exactly the two slide images freed, no leak"
        );
        // 新スライドは新 ID で送信。
        assert!(ev.contains(&Ev::Transmit {
            id: 3,
            w: 600,
            h: 480
        }));
    }

    #[test]
    fn apply_overlay_without_slide_errors() {
        let mut p = Presenter::new(Recorder::default(), viewport());
        let mut sink = Vec::new();
        let a = solid(600, 480, [0, 0, 0, 255], 0);
        let b = with_pixel(&a, 0, 0, [1, 0, 0, 255]);
        let err = p.apply_overlay(&mut sink, &a, &b, 1).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn resize_clears_then_re_presents() {
        let mut p = Presenter::new(Recorder::default(), viewport());
        let mut sink = Vec::new();
        let base = solid(600, 480, [10, 20, 30, 255], 0);
        p.present_slide(&mut sink, &base).unwrap();

        let new_vp = Viewport {
            cols: 100,
            rows: 30,
            cell: CellSize { w_px: 12, h_px: 24 },
        };
        let base2 = solid(600, 480, [10, 20, 30, 255], 0);
        p.resize(&mut sink, new_vp, &base2).unwrap();
        assert_eq!(p.viewport(), new_vp);
        // 旧画像(1)を削除してから新 ID(2)で再送信。
        let ev = &p.backend().events;
        assert!(ev.contains(&Ev::DeleteImage { image: 1 }));
        assert!(ev.contains(&Ev::Transmit {
            id: 2,
            w: 600,
            h: 480
        }));
    }
}
