//! ラスタ済みフレームの小さな LRU キャッシュ（不純シェル補助）。
//!
//! ページ移動のたびに走る同期ラスタ（`engine.render_fit` → `typst_render::render`
//! と全画素アンプリマルチプライ）は重い。一度ラスタしたフレームを保持し、戻る・
//! 行き来やアイドル先読み（[`crate::nav::prefetch_targets`]）で再利用する。
//!
//! キーは `(FrameId, PixelSize)`。viewport 解像度が変われば別エントリになり、
//! 古い解像度のエントリは新 viewport の参照で自然にミスして容量淘汰に任せる。
//!
//! 容量はバイト基準（解像度で 1 枚のサイズが大きく変わるため枚数基準は不可）。
//! `paladocs_render::PixelSize` は `Hash` 非実装なので `HashMap` は使わず、容量が
//! 小さい前提の `Vec` 線形走査で実装する（依存を増やさない）。

use paladocs_core::FrameId;
use paladocs_render::Frame;

/// キャッシュキー。`PixelSize` を `(w, h)` へ展開して `Eq` 可能にする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Key {
    pub id: FrameId,
    pub w: u32,
    pub h: u32,
}

/// バイト容量つきの LRU フレームキャッシュ。新しく使ったものほど末尾。
pub(crate) struct FrameCache {
    cap_bytes: usize,
    used_bytes: usize,
    /// `(key, frame)` を最終使用順に保持（先頭 = 最古、末尾 = 最新）。
    entries: Vec<(Key, Frame)>,
}

/// 1 フレームのバイト数。
fn frame_bytes(frame: &Frame) -> usize {
    frame.image.size().byte_len()
}

impl FrameCache {
    /// 容量上限（バイト）を指定して空のキャッシュを作る。容量は最低 1。
    pub(crate) fn new(cap_bytes: usize) -> Self {
        Self {
            cap_bytes: cap_bytes.max(1),
            used_bytes: 0,
            entries: Vec::new(),
        }
    }

    /// `key` のフレームを返す。ヒット時は最近使用として末尾へ移す。
    pub(crate) fn get(&mut self, key: Key) -> Option<&Frame> {
        let pos = self.entries.iter().position(|(k, _)| *k == key)?;
        let entry = self.entries.remove(pos);
        self.entries.push(entry);
        self.entries.last().map(|(_, f)| f)
    }

    /// `key` がキャッシュ済みか（先読みの「もう要らない」判定用、非可変）。
    pub(crate) fn contains(&self, key: Key) -> bool {
        self.entries.iter().any(|(k, _)| *k == key)
    }

    /// `frame` を投入する。同一キーは置換し、容量超過なら最古から淘汰する。
    ///
    /// 1 枚が容量を超える場合でも**最低 1 枚は保持**する（淘汰の暴走・パニック回避）。
    pub(crate) fn insert(&mut self, key: Key, frame: Frame) {
        if let Some(pos) = self.entries.iter().position(|(k, _)| *k == key) {
            let (_, old) = self.entries.remove(pos);
            self.used_bytes = self.used_bytes.saturating_sub(frame_bytes(&old));
        }
        self.used_bytes += frame_bytes(&frame);
        self.entries.push((key, frame));
        self.evict_to_cap();
    }

    /// 容量上限を更新する（resize で 1 フレームのバイト数が変わったとき）。
    pub(crate) fn set_cap(&mut self, cap_bytes: usize) {
        self.cap_bytes = cap_bytes.max(1);
        self.evict_to_cap();
    }

    /// 全エントリを破棄する（reload 後、同 FrameId が別内容になりうるため）。
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.used_bytes = 0;
    }

    /// 容量を超えている間、最古（先頭）から淘汰する。最低 1 枚は残す。
    fn evict_to_cap(&mut self) {
        while self.used_bytes > self.cap_bytes && self.entries.len() > 1 {
            let (_, old) = self.entries.remove(0);
            self.used_bytes = self.used_bytes.saturating_sub(frame_bytes(&old));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paladocs_render::{PixelSize, Rgba};

    /// `w*h` の透明フレームを作る（バイト数 = w*h*4）。
    fn frame(id: u32, w: u32, h: u32) -> Frame {
        Frame {
            id: FrameId(id),
            image: Rgba::transparent(PixelSize { w, h }),
        }
    }

    fn key(id: u32, w: u32, h: u32) -> Key {
        Key {
            id: FrameId(id),
            w,
            h,
        }
    }

    /// `Frame` は `PartialEq` 非実装なので id と image（`Rgba` は `Eq`）で照合する。
    fn assert_frame(got: Option<&Frame>, id: u32, w: u32, h: u32) {
        let f = got.expect("expected cache hit");
        assert_eq!(f.id, FrameId(id));
        assert_eq!(f.image, Rgba::transparent(PixelSize { w, h }));
    }

    #[test]
    fn cache_hit_after_insert() {
        let mut c = FrameCache::new(1024);
        c.insert(key(0, 2, 2), frame(0, 2, 2));
        assert_frame(c.get(key(0, 2, 2)), 0, 2, 2);
    }

    #[test]
    fn cache_miss_for_different_pixelsize() {
        let mut c = FrameCache::new(1024);
        c.insert(key(0, 2, 2), frame(0, 2, 2));
        // 同じ FrameId でも解像度が違えばミス。
        assert!(c.get(key(0, 4, 4)).is_none());
        assert!(!c.contains(key(0, 4, 4)));
        assert!(c.contains(key(0, 2, 2)));
    }

    #[test]
    fn cache_evicts_oldest_when_over_byte_cap() {
        // 1x1 = 4 バイト。cap=8 なら 2 枚まで。
        let mut c = FrameCache::new(8);
        c.insert(key(0, 1, 1), frame(0, 1, 1));
        c.insert(key(1, 1, 1), frame(1, 1, 1));
        c.insert(key(2, 1, 1), frame(2, 1, 1)); // used=12>8 → 最古(0)を淘汰
        assert!(!c.contains(key(0, 1, 1)));
        assert!(c.contains(key(1, 1, 1)));
        assert!(c.contains(key(2, 1, 1)));
    }

    #[test]
    fn cache_get_marks_recently_used() {
        // 1x1 = 4 バイト。cap=12 なら 3 枚。
        let mut c = FrameCache::new(12);
        c.insert(key(0, 1, 1), frame(0, 1, 1));
        c.insert(key(1, 1, 1), frame(1, 1, 1));
        c.insert(key(2, 1, 1), frame(2, 1, 1));
        // 最古(0)を get → 最近使用に昇格（順序 1,2,0）。
        assert!(c.get(key(0, 1, 1)).is_some());
        // 4 枚目投入で最古(1)が淘汰され、0 は残る。
        c.insert(key(3, 1, 1), frame(3, 1, 1));
        assert!(!c.contains(key(1, 1, 1)));
        assert!(c.contains(key(0, 1, 1)));
        assert!(c.contains(key(3, 1, 1)));
    }

    #[test]
    fn cache_keeps_at_least_one_when_frame_exceeds_cap() {
        // cap=4 だが 2x2=16 バイトのフレーム。淘汰の暴走なく 1 枚保持。
        let mut c = FrameCache::new(4);
        c.insert(key(0, 2, 2), frame(0, 2, 2));
        assert!(c.contains(key(0, 2, 2)));
        assert_frame(c.get(key(0, 2, 2)), 0, 2, 2);
    }

    #[test]
    fn cache_replace_same_key_does_not_grow() {
        // 同一キー再投入で used_bytes が二重計上されないこと（淘汰挙動で確認）。
        // cap=8（1x1 2 枚分）。同じキーを 2 回入れても 1 枚分のまま。
        let mut c = FrameCache::new(8);
        c.insert(key(0, 1, 1), frame(0, 1, 1));
        c.insert(key(0, 1, 1), frame(0, 1, 1));
        c.insert(key(1, 1, 1), frame(1, 1, 1));
        // used = 8 ちょうど。両方残る（二重計上していれば 0 が淘汰される）。
        assert!(c.contains(key(0, 1, 1)));
        assert!(c.contains(key(1, 1, 1)));
    }

    #[test]
    fn cache_set_cap_evicts_immediately() {
        let mut c = FrameCache::new(12); // 1x1 3 枚
        c.insert(key(0, 1, 1), frame(0, 1, 1));
        c.insert(key(1, 1, 1), frame(1, 1, 1));
        c.insert(key(2, 1, 1), frame(2, 1, 1));
        c.set_cap(4); // 1 枚分へ縮小 → 最新のみ残す
        assert!(!c.contains(key(0, 1, 1)));
        assert!(!c.contains(key(1, 1, 1)));
        assert!(c.contains(key(2, 1, 1)));
    }

    #[test]
    fn cache_clear_empties() {
        let mut c = FrameCache::new(1024);
        c.insert(key(0, 1, 1), frame(0, 1, 1));
        c.insert(key(1, 1, 1), frame(1, 1, 1));
        c.clear();
        assert!(!c.contains(key(0, 1, 1)));
        assert!(!c.contains(key(1, 1, 1)));
        assert_eq!(c.used_bytes, 0);
    }
}
