/// An RGBA color with 8-bit components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[allow(dead_code)]
    pub(crate) fn to_f32_array(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }

    pub const RED: Self = Self::rgb(255, 0, 0);
    pub const GREEN: Self = Self::rgb(0, 255, 0);
    pub const BLUE: Self = Self::rgb(0, 0, 255);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const YELLOW: Self = Self::rgb(255, 255, 0);
    pub const CYAN: Self = Self::rgb(0, 255, 255);
    pub const MAGENTA: Self = Self::rgb(255, 0, 255);
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_sets_full_alpha() {
        let c = Color::rgb(10, 20, 30);
        assert_eq!(c.a, 255);
    }

    #[test]
    fn rgba_preserves_all_channels() {
        let c = Color::rgba(10, 20, 30, 40);
        assert_eq!((c.r, c.g, c.b, c.a), (10, 20, 30, 40));
    }

    #[test]
    fn to_f32_array_normalizes() {
        let c = Color::rgba(255, 0, 128, 255);
        let f = c.to_f32_array();
        assert!((f[0] - 1.0).abs() < f32::EPSILON);
        assert!((f[1] - 0.0).abs() < f32::EPSILON);
        assert!((f[2] - 128.0 / 255.0).abs() < 0.001);
        assert!((f[3] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn constants_are_correct() {
        assert_eq!(Color::RED, Color::rgb(255, 0, 0));
        assert_eq!(Color::TRANSPARENT, Color::rgba(0, 0, 0, 0));
    }
}
