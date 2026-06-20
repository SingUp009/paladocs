//! Kitty の画像 ID / placement ID と、その単調割当器。
//!
//! Kitty graphics protocol では画像 `i=` と placement `p=` はいずれも **非ゼロ**の
//! u32（0 は「未指定」を意味する）。本モジュールの [`IdAllocator`] は 1 から単調に
//! 採番し、0 を決して返さない。

/// Kitty 画像 ID（`i=`）。**不変条件: 値は非ゼロ**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageId(pub u32);

/// Kitty placement ID（`p=`）。**不変条件: 値は非ゼロ**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlacementId(pub u32);

/// 画像 ID / placement ID を 1 から単調採番する割当器。
///
/// それぞれ独立に増加し、`u32` を使い切った場合は 1 に巻き戻す（0 は返さない）。
#[derive(Debug, Clone)]
pub struct IdAllocator {
    next_image: u32,
    next_placement: u32,
}

impl IdAllocator {
    /// 1 始まりの割当器を作る。
    pub fn new() -> Self {
        Self {
            next_image: 1,
            next_placement: 1,
        }
    }

    /// 次の画像 ID を返す（非ゼロ）。
    pub fn next_image(&mut self) -> ImageId {
        let id = self.next_image;
        self.next_image = self.next_image.checked_add(1).unwrap_or(1);
        ImageId(id)
    }

    /// 次の placement ID を返す（非ゼロ）。
    pub fn next_placement(&mut self) -> PlacementId {
        let id = self.next_placement;
        self.next_placement = self.next_placement.checked_add(1).unwrap_or(1);
        PlacementId(id)
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_nonzero_monotonic() {
        let mut a = IdAllocator::new();
        assert_eq!(a.next_image(), ImageId(1));
        assert_eq!(a.next_image(), ImageId(2));
        assert_eq!(a.next_placement(), PlacementId(1));
        assert_eq!(a.next_placement(), PlacementId(2));
        // 画像と placement は独立。
        assert_eq!(a.next_image(), ImageId(3));
        assert_eq!(a.next_placement(), PlacementId(3));
    }
}
