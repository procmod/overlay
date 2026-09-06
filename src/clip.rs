/// An axis-aligned clipping rectangle in overlay pixel coordinates.
///
/// Bounds are left/top inclusive and right/bottom exclusive. The renderer rounds
/// left/top down and right/bottom up to integer pixels, then intersects the result
/// with the viewport. Empty, inverted, or non-finite bounds clip all drawing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipRect {
    /// Left edge.
    pub left: f32,
    /// Top edge.
    pub top: f32,
    /// Right edge (exclusive).
    pub right: f32,
    /// Bottom edge (exclusive).
    pub bottom: f32,
}

impl ClipRect {
    /// Construct a rectangle from its minimum and maximum coordinates.
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub(crate) fn canonical(self) -> Self {
        if ![self.left, self.top, self.right, self.bottom]
            .iter()
            .all(|v| v.is_finite())
            || self.left >= self.right
            || self.top >= self.bottom
        {
            Self::new(0.0, 0.0, 0.0, 0.0)
        } else {
            self
        }
    }

    pub(crate) fn intersect(self, other: Self) -> Self {
        Self::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        )
        .canonical()
    }
}

pub(crate) fn scissor(clip: Option<ClipRect>, width: u32, height: u32) -> [i32; 4] {
    let w = width.min(i32::MAX as u32) as i32;
    let h = height.min(i32::MAX as u32) as i32;
    match clip.map(ClipRect::canonical) {
        None => [0, 0, w, h],
        Some(c) => [
            (c.left.floor() as i32).clamp(0, w),
            (c.top.floor() as i32).clamp(0, h),
            (c.right.ceil() as i32).clamp(0, w),
            (c.bottom.ceil() as i32).clamp(0, h),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scissor_rounds_outward_and_clamps_to_viewport() {
        assert_eq!(
            scissor(Some(ClipRect::new(-2.1, 2.9, 7.1, 100.0)), 10, 20),
            [0, 2, 8, 20]
        );
        assert_eq!(scissor(None, 30, 40), [0, 0, 30, 40]);
        assert_eq!(
            scissor(None, u32::MAX, u32::MAX),
            [0, 0, i32::MAX, i32::MAX]
        );
        assert_eq!(
            scissor(Some(ClipRect::new(20.0, 30.0, 40.0, 50.0)), 10, 10),
            [10, 10, 10, 10]
        );
    }

    #[test]
    fn invalid_and_empty_rectangles_never_expand() {
        for c in [
            ClipRect::new(0.1, 0.1, 0.1, 5.0),
            ClipRect::new(2.0, 3.0, 1.0, 8.0),
            ClipRect::new(f32::NAN, 0.0, 2.0, 2.0),
            ClipRect::new(0.0, 0.0, f32::INFINITY, 2.0),
        ] {
            assert_eq!(scissor(Some(c), 100, 100), [0; 4]);
        }
        let a = ClipRect::new(0.0, 0.0, 1.0, 1.0);
        assert_eq!(
            a.intersect(ClipRect::new(2.0, 2.0, 3.0, 3.0)),
            ClipRect::new(0.0, 0.0, 0.0, 0.0)
        );
        assert_eq!(scissor(None, 0, 0), [0; 4]);
    }
}
