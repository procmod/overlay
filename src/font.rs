use fontdue::{Font, FontSettings};

const DEFAULT_FONT: &[u8] = include_bytes!("../assets/Roboto-Regular.ttf");

/// Pre-rasterized glyph stored in the atlas.
#[derive(Debug, Clone)]
pub(crate) struct RasterizedGlyph {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub advance: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

/// A texture atlas of rasterized glyphs for a specific font size.
pub(crate) struct GlyphAtlas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    glyphs: Vec<Option<RasterizedGlyph>>,
    font: Font,
    size: f32,
}

impl GlyphAtlas {
    pub fn new(size: f32) -> Self {
        let font = Font::from_bytes(DEFAULT_FONT, FontSettings::default())
            .expect("embedded font must be valid");
        let mut atlas = Self {
            width: 512,
            height: 512,
            pixels: vec![0; 512 * 512],
            glyphs: vec![None; 128],
            font,
            size,
        };
        atlas.rasterize_ascii();
        atlas
    }

    fn rasterize_ascii(&mut self) {
        let mut cursor_x: u32 = 0;
        let mut cursor_y: u32 = 0;
        let mut row_height: u32 = 0;

        for c in 32u8..127 {
            let (metrics, bitmap) = self.font.rasterize(c as char, self.size);
            if metrics.width == 0 || metrics.height == 0 {
                self.glyphs[c as usize] = Some(RasterizedGlyph {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                    advance: metrics.advance_width,
                    offset_x: metrics.xmin as f32,
                    offset_y: metrics.ymin as f32,
                });
                continue;
            }

            let w = metrics.width as u32;
            let h = metrics.height as u32;

            if cursor_x + w + 1 > self.width {
                cursor_x = 0;
                cursor_y += row_height + 1;
                row_height = 0;
            }

            if cursor_y + h + 1 > self.height {
                self.grow();
                self.rasterize_ascii();
                return;
            }

            for row in 0..h {
                let src_start = (row * w) as usize;
                let dst_start = ((cursor_y + row) * self.width + cursor_x) as usize;
                self.pixels[dst_start..dst_start + w as usize]
                    .copy_from_slice(&bitmap[src_start..src_start + w as usize]);
            }

            self.glyphs[c as usize] = Some(RasterizedGlyph {
                x: cursor_x,
                y: cursor_y,
                width: w,
                height: h,
                advance: metrics.advance_width,
                offset_x: metrics.xmin as f32,
                offset_y: metrics.ymin as f32,
            });

            cursor_x += w + 1;
            row_height = row_height.max(h);
        }
    }

    fn grow(&mut self) {
        self.height *= 2;
        self.pixels.resize((self.width * self.height) as usize, 0);
    }

    pub fn glyph(&self, c: char) -> Option<&RasterizedGlyph> {
        let idx = c as usize;
        if idx < self.glyphs.len() {
            self.glyphs[idx].as_ref()
        } else {
            None
        }
    }

    /// Measure the bounding box of a string at this atlas's font size.
    pub fn measure(&self, text: &str) -> (f32, f32) {
        let mut width: f32 = 0.0;
        let line_height = self.size;
        let mut max_height: f32 = line_height;
        let mut lines = 1.0_f32;

        for c in text.chars() {
            if c == '\n' {
                lines += 1.0;
                max_height = lines * line_height;
                width = width.max(0.0);
                continue;
            }
            if let Some(g) = self.glyph(c) {
                width += g.advance;
            }
        }
        (width, max_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_creation() {
        let atlas = GlyphAtlas::new(16.0);
        assert!(atlas.width >= 512);
        assert!(atlas.height >= 512);
        assert!(!atlas.pixels.is_empty());
    }

    #[test]
    fn ascii_glyphs_rasterized() {
        let atlas = GlyphAtlas::new(16.0);
        assert!(atlas.glyph('A').is_some());
        assert!(atlas.glyph('z').is_some());
        assert!(atlas.glyph(' ').is_some());
    }

    #[test]
    fn non_ascii_returns_none() {
        let atlas = GlyphAtlas::new(16.0);
        assert!(atlas.glyph('\u{1F600}').is_none());
    }

    #[test]
    fn glyph_has_positive_dimensions() {
        let atlas = GlyphAtlas::new(16.0);
        let g = atlas.glyph('A').unwrap();
        assert!(g.width > 0);
        assert!(g.height > 0);
        assert!(g.advance > 0.0);
    }

    #[test]
    fn measure_empty_string() {
        let atlas = GlyphAtlas::new(16.0);
        let (w, h) = atlas.measure("");
        assert_eq!(w, 0.0);
        assert_eq!(h, 16.0);
    }

    #[test]
    fn measure_single_char() {
        let atlas = GlyphAtlas::new(16.0);
        let (w, _h) = atlas.measure("A");
        assert!(w > 0.0);
    }

    #[test]
    fn measure_multiline() {
        let atlas = GlyphAtlas::new(16.0);
        let (_w, h) = atlas.measure("A\nB");
        assert!((h - 32.0).abs() < f32::EPSILON);
    }

    #[test]
    fn space_glyph_has_zero_dimensions_but_advance() {
        let atlas = GlyphAtlas::new(16.0);
        let g = atlas.glyph(' ').unwrap();
        assert_eq!(g.width, 0);
        assert!(g.advance > 0.0);
    }
}
