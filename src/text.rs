use crate::color::Color;
use crate::font::{GlyphAtlas, RasterizedGlyph, ATLAS_FONT_SIZE, MAX_OUTLINE_TEXELS};
use crate::vertex::{DrawList, Vertex};

/// Horizontal placement of a line relative to the position passed to the draw call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// The position is the left edge of the line.
    #[default]
    Left,
    /// The position is the horizontal center of the line.
    Center,
    /// The position is the right edge of the line.
    Right,
}

/// How a string is drawn: size, color, optional outline, and alignment.
///
/// Each line of a multi-line string is aligned independently.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub color: Color,
    /// Outline color and width in pixels.
    ///
    /// The outline is rasterized from the glyph's distance field, so its thickness is
    /// uniform around curves and corners. Width is clamped to
    /// [`TextStyle::max_outline_width`]. Outlines of adjacent glyphs overlap slightly
    /// where they meet, which is only visible if the outline color is translucent.
    pub outline: Option<(Color, f32)>,
    pub align: TextAlign,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: ATLAS_FONT_SIZE,
            color: Color::WHITE,
            outline: None,
            align: TextAlign::default(),
        }
    }
}

impl TextStyle {
    pub fn new(size: f32, color: Color) -> Self {
        Self {
            size,
            color,
            ..Self::default()
        }
    }

    pub fn outlined(self, color: Color, width: f32) -> Self {
        Self {
            outline: Some((color, width)),
            ..self
        }
    }

    pub fn aligned(self, align: TextAlign) -> Self {
        Self { align, ..self }
    }

    /// Widest outline available at the given font size, in pixels.
    ///
    /// The distance field baked into the glyph atlas has a fixed range, so the usable
    /// outline width grows with the font size. Wider requests are clamped to this value.
    pub fn max_outline_width(size: f32) -> f32 {
        MAX_OUTLINE_TEXELS * size / ATLAS_FONT_SIZE
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Rect {
    fn expand(self, by: f32) -> Self {
        Self {
            x0: self.x0 - by,
            y0: self.y0 - by,
            x1: self.x1 + by,
            y1: self.y1 + by,
        }
    }
}

/// Append the quads for a string to the draw list.
///
/// Outlines are emitted for the whole string before any fill, so a glyph's outline never
/// covers the neighbour it overlaps.
pub(crate) fn emit(
    draw_list: &mut DrawList,
    atlas: &GlyphAtlas,
    x: f32,
    y: f32,
    text: &str,
    style: &TextStyle,
) {
    if style.size <= 0.0 || text.is_empty() {
        return;
    }
    let scale = style.size / ATLAS_FONT_SIZE;

    if let Some((color, width)) = style.outline {
        let width = width.clamp(0.0, TextStyle::max_outline_width(style.size));
        if width > 0.0 {
            let texels = width / scale + 1.0;
            let params = [width, scale];
            let color = color.to_f32_array();
            layout(atlas, x, y, text, style, |glyph, rect| {
                let uv = glyph_uv(atlas, glyph, texels);
                let rect = rect.expand(texels * scale);
                draw_list.add_glyph_outline_quad(
                    Vertex::with_uv(rect.x0, rect.y0, color, uv.x0, uv.y0),
                    Vertex::with_uv(rect.x1, rect.y0, color, uv.x1, uv.y0),
                    Vertex::with_uv(rect.x1, rect.y1, color, uv.x1, uv.y1),
                    Vertex::with_uv(rect.x0, rect.y1, color, uv.x0, uv.y1),
                    params,
                );
            });
        }
    }

    let color = style.color.to_f32_array();
    layout(atlas, x, y, text, style, |glyph, rect| {
        let uv = glyph_uv(atlas, glyph, 0.0);
        draw_list.add_glyph_quad(
            Vertex::with_uv(rect.x0, rect.y0, color, uv.x0, uv.y0),
            Vertex::with_uv(rect.x1, rect.y0, color, uv.x1, uv.y0),
            Vertex::with_uv(rect.x1, rect.y1, color, uv.x1, uv.y1),
            Vertex::with_uv(rect.x0, rect.y1, color, uv.x0, uv.y1),
        );
    });
}

fn layout(
    atlas: &GlyphAtlas,
    x: f32,
    y: f32,
    text: &str,
    style: &TextStyle,
    mut place: impl FnMut(&RasterizedGlyph, Rect),
) {
    let scale = style.size / ATLAS_FONT_SIZE;
    let mut line_y = y;

    for line in text.split('\n') {
        let mut cursor = x + align_offset(style.align, atlas.line_width(line) * scale);
        for c in line.chars() {
            let Some(glyph) = atlas.glyph(c) else {
                continue;
            };
            if glyph.width > 0 && glyph.height > 0 {
                let x0 = cursor + glyph.offset_x * scale;
                let y0 = line_y + style.size - (glyph.offset_y + glyph.height as f32) * scale;
                place(
                    glyph,
                    Rect {
                        x0,
                        y0,
                        x1: x0 + glyph.width as f32 * scale,
                        y1: y0 + glyph.height as f32 * scale,
                    },
                );
            }
            cursor += glyph.advance * scale;
        }
        line_y += style.size;
    }
}

fn align_offset(align: TextAlign, width: f32) -> f32 {
    match align {
        TextAlign::Left => 0.0,
        TextAlign::Center => -width / 2.0,
        TextAlign::Right => -width,
    }
}

fn glyph_uv(atlas: &GlyphAtlas, glyph: &RasterizedGlyph, expand: f32) -> Rect {
    let width = atlas.width as f32;
    let height = atlas.height as f32;
    Rect {
        x0: (glyph.x as f32 - expand) / width,
        y0: (glyph.y as f32 - expand) / height,
        x1: (glyph.x as f32 + glyph.width as f32 + expand) / width,
        y1: (glyph.y as f32 + glyph.height as f32 + expand) / height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vertex::CommandKind;

    fn draw(text: &str, style: &TextStyle) -> DrawList {
        let atlas = GlyphAtlas::new();
        let mut list = DrawList::new();
        emit(&mut list, &atlas, 100.0, 50.0, text, style);
        list
    }

    fn kinds(list: &DrawList) -> Vec<CommandKind> {
        list.commands.iter().map(|c| c.kind).collect()
    }

    fn bounds(vertices: &[Vertex]) -> Rect {
        vertices.iter().fold(
            Rect {
                x0: f32::MAX,
                y0: f32::MAX,
                x1: f32::MIN,
                y1: f32::MIN,
            },
            |acc, v| Rect {
                x0: acc.x0.min(v.position[0]),
                y0: acc.y0.min(v.position[1]),
                x1: acc.x1.max(v.position[0]),
                y1: acc.y1.max(v.position[1]),
            },
        )
    }

    #[test]
    fn plain_text_emits_one_glyph_command() {
        let list = draw("Hi", &TextStyle::new(16.0, Color::WHITE));
        assert_eq!(kinds(&list), vec![CommandKind::Glyph]);
        assert_eq!(list.vertices.len(), 8);
    }

    #[test]
    fn outlined_text_emits_outline_before_fill() {
        let style = TextStyle::new(16.0, Color::WHITE).outlined(Color::BLACK, 2.0);
        let list = draw("Hi", &style);
        assert_eq!(
            kinds(&list),
            vec![CommandKind::GlyphOutline, CommandKind::Glyph]
        );
        assert_eq!(list.vertices.len(), 16);
    }

    #[test]
    fn a_whole_string_costs_two_draw_calls() {
        let style = TextStyle::new(16.0, Color::WHITE).outlined(Color::BLACK, 1.5);
        let list = draw("player one [100]", &style);
        assert_eq!(list.commands.len(), 2);
    }

    #[test]
    fn outline_quads_wrap_the_fill_quads() {
        let style = TextStyle::new(16.0, Color::WHITE).outlined(Color::BLACK, 2.0);
        let list = draw("H", &style);
        let outline = bounds(&list.vertices[..4]);
        let fill = bounds(&list.vertices[4..]);

        assert!(outline.x0 < fill.x0);
        assert!(outline.y0 < fill.y0);
        assert!(outline.x1 > fill.x1);
        assert!(outline.y1 > fill.y1);
    }

    #[test]
    fn outline_carries_width_and_scale() {
        let style = TextStyle::new(32.0, Color::WHITE).outlined(Color::BLACK, 3.0);
        let list = draw("H", &style);
        assert_eq!(list.vertices[0].params, [3.0, 2.0]);
        assert_eq!(list.vertices[4].params, [0.0, 0.0]);
    }

    #[test]
    fn outline_width_is_clamped_to_the_field_range() {
        let style = TextStyle::new(16.0, Color::WHITE).outlined(Color::BLACK, 1000.0);
        let list = draw("H", &style);
        assert_eq!(
            list.vertices[0].params[0],
            TextStyle::max_outline_width(16.0)
        );
    }

    #[test]
    fn max_outline_width_scales_with_font_size() {
        assert_eq!(TextStyle::max_outline_width(16.0), 7.0);
        assert_eq!(TextStyle::max_outline_width(32.0), 14.0);
    }

    #[test]
    fn outline_samples_stay_inside_the_atlas() {
        let atlas = GlyphAtlas::new();
        let style = TextStyle::new(8.0, Color::WHITE).outlined(Color::BLACK, 1000.0);
        let mut list = DrawList::new();
        emit(&mut list, &atlas, 0.0, 0.0, "Wg", &style);

        for vertex in &list.vertices {
            assert!(vertex.uv[0] >= 0.0 && vertex.uv[0] <= 1.0);
            assert!(vertex.uv[1] >= 0.0 && vertex.uv[1] <= 1.0);
        }
    }

    #[test]
    fn zero_width_outline_is_skipped() {
        let style = TextStyle::new(16.0, Color::WHITE).outlined(Color::BLACK, 0.0);
        let list = draw("H", &style);
        assert_eq!(kinds(&list), vec![CommandKind::Glyph]);
    }

    #[test]
    fn non_positive_size_draws_nothing() {
        let list = draw("H", &TextStyle::new(0.0, Color::WHITE));
        assert!(list.commands.is_empty());
        let list = draw("H", &TextStyle::new(-4.0, Color::WHITE));
        assert!(list.commands.is_empty());
    }

    #[test]
    fn blank_input_draws_nothing() {
        assert!(draw("", &TextStyle::default()).commands.is_empty());
        assert!(draw("   ", &TextStyle::default()).commands.is_empty());
    }

    #[test]
    fn centered_text_straddles_the_position() {
        let atlas = GlyphAtlas::new();
        let left = draw("centered", &TextStyle::default());
        let centered = draw("centered", &TextStyle::default().aligned(TextAlign::Center));

        let width = atlas.line_width("centered");
        let shift = bounds(&left.vertices).x0 - bounds(&centered.vertices).x0;
        assert!((shift - width / 2.0).abs() < 0.001);
    }

    #[test]
    fn right_aligned_text_ends_at_the_position() {
        let atlas = GlyphAtlas::new();
        let left = draw("right", &TextStyle::default());
        let right = draw("right", &TextStyle::default().aligned(TextAlign::Right));

        let width = atlas.line_width("right");
        let shift = bounds(&left.vertices).x0 - bounds(&right.vertices).x0;
        assert!((shift - width).abs() < 0.001);
    }

    #[test]
    fn each_line_aligns_independently() {
        let list = draw(
            "wide line here\nx",
            &TextStyle::default().aligned(TextAlign::Center),
        );

        let atlas = GlyphAtlas::new();
        let long = atlas.line_width("wide line here");
        let short = atlas.line_width("x");

        let first_line_start = list.vertices[0].position[0];
        let last_line_start = list.vertices[list.vertices.len() - 4].position[0];

        assert!((first_line_start - (100.0 - long / 2.0)).abs() < 2.0);
        assert!((last_line_start - (100.0 - short / 2.0)).abs() < 2.0);
    }

    #[test]
    fn newlines_advance_by_the_font_size() {
        let list = draw("a\na", &TextStyle::new(16.0, Color::WHITE));
        let first = list.vertices[0].position[1];
        let second = list.vertices[4].position[1];
        assert!((second - first - 16.0).abs() < 0.001);
    }

    #[test]
    fn scaling_the_size_scales_the_quads() {
        let small = draw("A", &TextStyle::new(16.0, Color::WHITE));
        let large = draw("A", &TextStyle::new(32.0, Color::WHITE));
        let small = bounds(&small.vertices);
        let large = bounds(&large.vertices);
        assert!(((large.x1 - large.x0) - (small.x1 - small.x0) * 2.0).abs() < 0.001);
    }

    #[test]
    fn default_style_matches_the_atlas_size() {
        let style = TextStyle::default();
        assert_eq!(style.size, ATLAS_FONT_SIZE);
        assert_eq!(style.align, TextAlign::Left);
        assert!(style.outline.is_none());
    }
}
