use fontdue::{Font, FontSettings};

const DEFAULT_FONT: &[u8] = include_bytes!("../assets/Roboto-Regular.ttf");

/// Size the glyph atlas is rasterized at. Drawn text is scaled from this size.
pub(crate) const ATLAS_FONT_SIZE: f32 = 16.0;

/// Texels of padding reserved around every glyph, and the range of the distance field.
///
/// An outline can never be wider than this, because the field is only computed inside
/// the padded box.
pub(crate) const OUTLINE_PAD: u32 = 8;

/// Widest outline the distance field can represent, in atlas texels.
///
/// One texel is held back so the antialiasing ramp at the outer edge stays inside the box.
pub(crate) const MAX_OUTLINE_TEXELS: f32 = OUTLINE_PAD as f32 - 1.0;

/// Resolution multiplier used when locating the glyph contour for the distance field.
const SUPERSAMPLE: usize = 5;

/// Pre-rasterized glyph stored in the atlas.
///
/// `x` and `y` locate the glyph itself. [`OUTLINE_PAD`] texels of distance field surround it.
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

/// A texture atlas of rasterized glyphs.
///
/// Two channels per texel: coverage for the glyph fill, and a distance field encoding the
/// distance from the texel to the glyph contour, used to draw outlines of any width in a
/// single pass.
pub(crate) struct GlyphAtlas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    glyphs: Vec<Option<RasterizedGlyph>>,
    font: Font,
}

impl GlyphAtlas {
    pub fn new() -> Self {
        let font = Font::from_bytes(DEFAULT_FONT, FontSettings::default())
            .expect("embedded font must be valid");
        let mut atlas = Self {
            width: 512,
            height: 512,
            pixels: vec![0; 512 * 512 * 2],
            glyphs: vec![None; 128],
            font,
        };
        atlas.rasterize_ascii();
        atlas
    }

    fn rasterize_ascii(&mut self) {
        let pad = OUTLINE_PAD;
        let mut cursor_x: u32 = 0;
        let mut cursor_y: u32 = 0;
        let mut row_height: u32 = 0;

        for c in 32u8..127 {
            let (metrics, bitmap) = self.font.rasterize(c as char, ATLAS_FONT_SIZE);
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
            let box_w = w + pad * 2;
            let box_h = h + pad * 2;

            if cursor_x + box_w + 1 > self.width {
                cursor_x = 0;
                cursor_y += row_height + 1;
                row_height = 0;
            }

            if cursor_y + box_h + 1 > self.height {
                self.grow();
                self.rasterize_ascii();
                return;
            }

            let x = cursor_x + pad;
            let y = cursor_y + pad;
            self.write_glyph(&bitmap, w, h, x, y);

            self.glyphs[c as usize] = Some(RasterizedGlyph {
                x,
                y,
                width: w,
                height: h,
                advance: metrics.advance_width,
                offset_x: metrics.xmin as f32,
                offset_y: metrics.ymin as f32,
            });

            cursor_x += box_w + 1;
            row_height = row_height.max(box_h);
        }
    }

    fn write_glyph(&mut self, bitmap: &[u8], width: u32, height: u32, x: u32, y: u32) {
        let pad = OUTLINE_PAD as usize;
        let (w, h) = (width as usize, height as usize);
        let (box_w, box_h) = (w + pad * 2, h + pad * 2);

        let mut coverage = vec![0.0f32; box_w * box_h];
        for row in 0..h {
            for col in 0..w {
                coverage[(row + pad) * box_w + col + pad] = bitmap[row * w + col] as f32 / 255.0;
            }
        }

        let field = distance_field(&coverage, box_w, box_h);
        let origin_x = x as usize - pad;
        let origin_y = y as usize - pad;

        for row in 0..box_h {
            for col in 0..box_w {
                let source = row * box_w + col;
                let target = ((origin_y + row) * self.width as usize + origin_x + col) * 2;
                let nearness = 1.0 - (field[source] / OUTLINE_PAD as f32).min(1.0);
                self.pixels[target] = (coverage[source] * 255.0).round() as u8;
                self.pixels[target + 1] = (nearness * 255.0).round() as u8;
            }
        }
    }

    fn grow(&mut self) {
        self.height *= 2;
        self.pixels.clear();
        self.pixels
            .resize((self.width * self.height * 2) as usize, 0);
    }

    pub fn glyph(&self, c: char) -> Option<&RasterizedGlyph> {
        let idx = c as usize;
        if idx < self.glyphs.len() {
            self.glyphs[idx].as_ref()
        } else {
            None
        }
    }

    /// Advance width of a single line at the atlas font size.
    pub fn line_width(&self, line: &str) -> f32 {
        line.chars()
            .filter_map(|c| self.glyph(c))
            .map(|g| g.advance)
            .sum()
    }

    /// Measure the bounding box of a string at the atlas font size.
    pub fn measure(&self, text: &str) -> (f32, f32) {
        let mut width: f32 = 0.0;
        let mut lines: u32 = 0;

        for line in text.split('\n') {
            width = width.max(self.line_width(line));
            lines += 1;
        }

        (width, lines as f32 * ATLAS_FONT_SIZE)
    }
}

/// Distance from every texel center to the glyph contour, in texels.
///
/// The contour is the 0.5 coverage level. It is located by upsampling the coverage grid,
/// which keeps the outline aligned with the antialiased fill it wraps, then taking an exact
/// Euclidean distance transform of the upsampled shape. Texels inside the glyph are zero.
fn distance_field(coverage: &[f32], width: usize, height: usize) -> Vec<f32> {
    let (fine_width, fine_height) = (width * SUPERSAMPLE, height * SUPERSAMPLE);
    let step = SUPERSAMPLE as f32;

    let mut seeds = vec![f32::INFINITY; fine_width * fine_height];
    for row in 0..fine_height {
        let y = (row as f32 + 0.5) / step - 0.5;
        for col in 0..fine_width {
            let x = (col as f32 + 0.5) / step - 0.5;
            if sample_bilinear(coverage, width, height, x, y) >= 0.5 {
                seeds[row * fine_width + col] = 0.0;
            }
        }
    }

    let squared = squared_distance_transform(&seeds, fine_width, fine_height);

    let center = SUPERSAMPLE / 2;
    let mut field = vec![0.0f32; width * height];
    for row in 0..height {
        for col in 0..width {
            let fine = (row * SUPERSAMPLE + center) * fine_width + col * SUPERSAMPLE + center;
            // the transform measures to the nearest covered sample center, which overshoots
            // the contour by half a fine texel on average
            field[row * width + col] = ((squared[fine].sqrt() - 0.5) / step).max(0.0);
        }
    }
    field
}

fn sample_bilinear(grid: &[f32], width: usize, height: usize, x: f32, y: f32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;

    let left = clamp_index(x0, width);
    let right = clamp_index(x0 + 1.0, width);
    let top = clamp_index(y0, height);
    let bottom = clamp_index(y0 + 1.0, height);

    let top_row = grid[top * width + left] * (1.0 - fx) + grid[top * width + right] * fx;
    let bottom_row = grid[bottom * width + left] * (1.0 - fx) + grid[bottom * width + right] * fx;
    top_row * (1.0 - fy) + bottom_row * fy
}

fn clamp_index(value: f32, length: usize) -> usize {
    if value < 0.0 {
        0
    } else {
        (value as usize).min(length - 1)
    }
}

/// Exact squared Euclidean distance transform (Felzenszwalb and Huttenlocher).
///
/// `seeds` holds zero where the shape is and infinity elsewhere.
fn squared_distance_transform(seeds: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut result = seeds.to_vec();
    let longest = width.max(height);
    let mut source = vec![0.0f32; longest];
    let mut transformed = vec![0.0f32; longest];
    let mut hull = vec![0usize; longest];
    let mut boundaries = vec![0.0f32; longest + 1];

    for col in 0..width {
        for row in 0..height {
            source[row] = result[row * width + col];
        }
        transform_1d(
            &source[..height],
            &mut transformed[..height],
            &mut hull,
            &mut boundaries,
        );
        for row in 0..height {
            result[row * width + col] = transformed[row];
        }
    }

    for row in 0..height {
        source[..width].copy_from_slice(&result[row * width..row * width + width]);
        transform_1d(
            &source[..width],
            &mut transformed[..width],
            &mut hull,
            &mut boundaries,
        );
        result[row * width..row * width + width].copy_from_slice(&transformed[..width]);
    }

    result
}

fn transform_1d(f: &[f32], d: &mut [f32], hull: &mut [usize], boundaries: &mut [f32]) {
    let n = f.len();
    let mut k = 0usize;
    let mut seeded = false;

    for q in 0..n {
        if !f[q].is_finite() {
            continue;
        }
        if !seeded {
            seeded = true;
            hull[0] = q;
            boundaries[0] = f32::NEG_INFINITY;
            boundaries[1] = f32::INFINITY;
            continue;
        }
        let rise = f[q] + (q * q) as f32;
        loop {
            let p = hull[k];
            let intersection = (rise - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
            if intersection <= boundaries[k] {
                k -= 1;
                continue;
            }
            k += 1;
            hull[k] = q;
            boundaries[k] = intersection;
            boundaries[k + 1] = f32::INFINITY;
            break;
        }
    }

    if !seeded {
        d.fill(f32::INFINITY);
        return;
    }

    let mut k = 0usize;
    for (q, slot) in d.iter_mut().enumerate() {
        while boundaries[k + 1] < q as f32 {
            k += 1;
        }
        let p = hull[k];
        let offset = q as f32 - p as f32;
        *slot = offset * offset + f[p];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distance_channel(atlas: &GlyphAtlas, x: u32, y: u32) -> u8 {
        atlas.pixels[((y * atlas.width + x) * 2 + 1) as usize]
    }

    fn coverage_channel(atlas: &GlyphAtlas, x: u32, y: u32) -> u8 {
        atlas.pixels[((y * atlas.width + x) * 2) as usize]
    }

    #[test]
    fn atlas_creation() {
        let atlas = GlyphAtlas::new();
        assert!(atlas.width >= 512);
        assert!(atlas.height >= 512);
        assert_eq!(
            atlas.pixels.len(),
            (atlas.width * atlas.height * 2) as usize
        );
    }

    #[test]
    fn ascii_glyphs_rasterized() {
        let atlas = GlyphAtlas::new();
        assert!(atlas.glyph('A').is_some());
        assert!(atlas.glyph('z').is_some());
        assert!(atlas.glyph(' ').is_some());
    }

    #[test]
    fn non_ascii_returns_none() {
        let atlas = GlyphAtlas::new();
        assert!(atlas.glyph('\u{1F600}').is_none());
    }

    #[test]
    fn glyph_has_positive_dimensions() {
        let atlas = GlyphAtlas::new();
        let g = atlas.glyph('A').unwrap();
        assert!(g.width > 0);
        assert!(g.height > 0);
        assert!(g.advance > 0.0);
    }

    #[test]
    fn measure_empty_string() {
        let atlas = GlyphAtlas::new();
        let (w, h) = atlas.measure("");
        assert_eq!(w, 0.0);
        assert_eq!(h, 16.0);
    }

    #[test]
    fn measure_single_char() {
        let atlas = GlyphAtlas::new();
        let (w, _h) = atlas.measure("A");
        assert!(w > 0.0);
    }

    #[test]
    fn measure_multiline() {
        let atlas = GlyphAtlas::new();
        let (w, h) = atlas.measure("AAAA\nB");
        let (first_line_width, _) = atlas.measure("AAAA");
        assert!((w - first_line_width).abs() < f32::EPSILON);
        assert!((h - 32.0).abs() < f32::EPSILON);
    }

    #[test]
    fn measure_multiline_uses_widest_line() {
        let atlas = GlyphAtlas::new();
        let (w, _) = atlas.measure("A\nBBBB");
        let (second_line_width, _) = atlas.measure("BBBB");
        assert!((w - second_line_width).abs() < f32::EPSILON);
    }

    #[test]
    fn line_width_ignores_nothing_and_sums_advances() {
        let atlas = GlyphAtlas::new();
        let single = atlas.line_width("A");
        let double = atlas.line_width("AA");
        assert!((double - single * 2.0).abs() < 0.001);
    }

    #[test]
    fn space_glyph_has_zero_dimensions_but_advance() {
        let atlas = GlyphAtlas::new();
        let g = atlas.glyph(' ').unwrap();
        assert_eq!(g.width, 0);
        assert!(g.advance > 0.0);
    }

    #[test]
    fn padded_glyph_boxes_do_not_overlap() {
        let atlas = GlyphAtlas::new();
        let pad = OUTLINE_PAD;
        let boxes: Vec<(u32, u32, u32, u32)> = (32u8..127)
            .filter_map(|c| atlas.glyph(c as char))
            .filter(|g| g.width > 0 && g.height > 0)
            .map(|g| (g.x - pad, g.y - pad, g.width + pad * 2, g.height + pad * 2))
            .collect();

        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                let disjoint =
                    a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1;
                assert!(disjoint, "glyph boxes {a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn padded_boxes_fit_inside_the_atlas() {
        let atlas = GlyphAtlas::new();
        for c in 32u8..127 {
            let g = atlas.glyph(c as char).unwrap();
            if g.width == 0 || g.height == 0 {
                continue;
            }
            assert!(g.x >= OUTLINE_PAD);
            assert!(g.y >= OUTLINE_PAD);
            assert!(g.x + g.width + OUTLINE_PAD <= atlas.width);
            assert!(g.y + g.height + OUTLINE_PAD <= atlas.height);
        }
    }

    #[test]
    fn covered_texels_sit_on_the_contour_or_inside_it() {
        let atlas = GlyphAtlas::new();
        let g = atlas.glyph('B').unwrap();
        let mut checked = 0;
        for y in g.y..g.y + g.height {
            for x in g.x..g.x + g.width {
                if coverage_channel(&atlas, x, y) == 255 {
                    assert_eq!(distance_channel(&atlas, x, y), 255);
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn distance_falls_off_through_the_padding() {
        let atlas = GlyphAtlas::new();
        let g = atlas.glyph('l').unwrap();
        let column = (g.x + g.width / 2).max(g.x);

        let mut previous = distance_channel(&atlas, column, g.y);
        for step in 1..=OUTLINE_PAD {
            let value = distance_channel(&atlas, column, g.y - step);
            assert!(
                value <= previous,
                "distance channel rose walking away from the glyph"
            );
            previous = value;
        }
        assert!(previous < 64);
    }

    #[test]
    fn distance_field_measures_from_the_contour() {
        let (w, h) = (16usize, 16usize);
        let mut coverage = vec![0.0f32; w * h];
        for y in 4..12 {
            for x in 4..12 {
                coverage[y * w + x] = 1.0;
            }
        }

        let field = distance_field(&coverage, w, h);
        let at = |x: usize, y: usize| field[y * w + x];

        assert_eq!(at(8, 8), 0.0);
        assert!((at(12, 8) - 0.5).abs() < 0.1, "got {}", at(12, 8));
        assert!((at(13, 8) - 1.5).abs() < 0.1, "got {}", at(13, 8));
        assert!((at(8, 3) - 0.5).abs() < 0.1, "got {}", at(8, 3));

        // reconstructing the contour from sampled coverage rounds a hard 90 degree corner
        // inward, so the diagonal reads slightly long compared to the ideal 0.707
        assert!((0.70..=1.05).contains(&at(12, 12)), "got {}", at(12, 12));
    }

    #[test]
    fn distance_field_is_monotonic_moving_away() {
        let (w, h) = (16usize, 16usize);
        let mut coverage = vec![0.0f32; w * h];
        for y in 6..10 {
            for x in 6..10 {
                coverage[y * w + x] = 1.0;
            }
        }

        let field = distance_field(&coverage, w, h);
        for x in 10..w - 1 {
            assert!(field[8 * w + x] <= field[8 * w + x + 1]);
        }
    }

    #[test]
    fn distance_field_without_a_shape_is_infinite() {
        let field = distance_field(&vec![0.0f32; 64], 8, 8);
        assert!(field.iter().all(|d| d.is_infinite()));
    }

    #[test]
    fn distance_field_tracks_a_partially_covered_edge() {
        let (w, h) = (12usize, 12usize);
        let mut coverage = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..6 {
                coverage[y * w + x] = 1.0;
            }
            coverage[y * w + 6] = 0.5;
        }

        let field = distance_field(&coverage, w, h);
        assert_eq!(field[6 * w + 6], 0.0);
        assert!(
            (field[6 * w + 7] - 1.0).abs() < 0.15,
            "got {}",
            field[6 * w + 7]
        );
    }
}
