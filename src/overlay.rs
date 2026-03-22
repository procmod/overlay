use crate::color::Color;
use crate::error::{Error, Result};
use crate::font::GlyphAtlas;
use crate::renderer::Renderer;
use crate::vertex::{DrawList, Vertex};
use crate::window::{OverlayTarget, OverlayWindow};
use std::f32::consts::PI;

const DEFAULT_FONT_SIZE: f32 = 16.0;
const CIRCLE_SEGMENTS: usize = 32;

/// A transparent, click-through overlay rendered on top of a target window.
///
/// Create an overlay by specifying the target window, then draw shapes and text
/// each frame between `begin_frame` and `end_frame` calls.
pub struct Overlay {
    window: OverlayWindow,
    renderer: Renderer,
    draw_list: DrawList,
    font_atlas: GlyphAtlas,
    in_frame: bool,
}

impl Overlay {
    /// Create a new overlay positioned over the target window.
    pub fn new(target: OverlayTarget) -> Result<Self> {
        let window = OverlayWindow::create(&target)?;
        let (w, h) = window.size();
        let mut renderer = Renderer::new(window.hwnd, w, h)?;

        let font_atlas = GlyphAtlas::new(DEFAULT_FONT_SIZE);
        renderer.upload_font_atlas(&font_atlas)?;

        Ok(Self {
            window,
            renderer,
            draw_list: DrawList::new(),
            font_atlas,
            in_frame: false,
        })
    }

    /// Returns true if the target window is the foreground window.
    pub fn is_visible(&self) -> bool {
        self.window.is_target_visible()
    }

    /// Begin a new frame. Call drawing methods after this, then call `end_frame`.
    pub fn begin_frame(&mut self) -> Result<()> {
        if !self.window.pump_messages() {
            return Err(Error::WindowClosed);
        }
        if !self.window.sync_position() {
            return Err(Error::WindowClosed);
        }

        let (w, h) = self.window.size();
        self.renderer.resize(w, h)?;

        self.draw_list.clear();
        self.renderer.begin_frame();
        self.in_frame = true;
        Ok(())
    }

    /// Finish the frame and present the rendered content.
    pub fn end_frame(&mut self) -> Result<()> {
        if !self.in_frame {
            return Err(Error::NoActiveFrame);
        }
        self.renderer.submit(
            &self.draw_list.vertices,
            &self.draw_list.indices,
            &self.draw_list.commands,
        )?;
        self.renderer.end_frame()?;
        self.in_frame = false;
        Ok(())
    }

    /// Draw a rectangle outline.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let t = 1.0;
        self.line(x, y, x + w, y, t, color);
        self.line(x + w, y, x + w, y + h, t, color);
        self.line(x + w, y + h, x, y + h, t, color);
        self.line(x, y + h, x, y, t, color);
    }

    /// Draw a filled rectangle.
    pub fn rect_filled(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let c = color.to_f32_array();
        self.draw_list.add_solid_quad(
            Vertex::new(x, y, c),
            Vertex::new(x + w, y, c),
            Vertex::new(x + w, y + h, c),
            Vertex::new(x, y + h, c),
        );
    }

    /// Draw a line with the given thickness.
    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, color: Color) {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len = (dx * dx + dy * dy).sqrt();
        if len < f32::EPSILON {
            return;
        }
        let nx = -dy / len * thickness * 0.5;
        let ny = dx / len * thickness * 0.5;
        let c = color.to_f32_array();
        self.draw_list.add_solid_quad(
            Vertex::new(x1 + nx, y1 + ny, c),
            Vertex::new(x1 - nx, y1 - ny, c),
            Vertex::new(x2 - nx, y2 - ny, c),
            Vertex::new(x2 + nx, y2 + ny, c),
        );
    }

    /// Draw a circle outline.
    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        let step = 2.0 * PI / CIRCLE_SEGMENTS as f32;
        for i in 0..CIRCLE_SEGMENTS {
            let a1 = step * i as f32;
            let a2 = step * (i + 1) as f32;
            self.line(
                cx + a1.cos() * radius,
                cy + a1.sin() * radius,
                cx + a2.cos() * radius,
                cy + a2.sin() * radius,
                1.0,
                color,
            );
        }
    }

    /// Draw a filled circle.
    pub fn circle_filled(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        let c = color.to_f32_array();
        let step = 2.0 * PI / CIRCLE_SEGMENTS as f32;
        let center = Vertex::new(cx, cy, c);
        let mut verts = vec![center];
        for i in 0..=CIRCLE_SEGMENTS {
            let angle = step * i as f32;
            verts.push(Vertex::new(
                cx + angle.cos() * radius,
                cy + angle.sin() * radius,
                c,
            ));
        }

        let mut indices = Vec::with_capacity(CIRCLE_SEGMENTS * 3);
        for i in 1..=CIRCLE_SEGMENTS {
            indices.push(0);
            indices.push(i as u32);
            indices.push((i % CIRCLE_SEGMENTS + 1) as u32);
        }

        self.draw_list.add_solid_triangles(&verts, &indices);
    }

    /// Draw text at the given position.
    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, color: Color) {
        let scale = size / DEFAULT_FONT_SIZE;
        let c = color.to_f32_array();
        let atlas_w = self.font_atlas.width as f32;
        let atlas_h = self.font_atlas.height as f32;
        let mut cursor_x = x;
        let mut cursor_y = y;

        for ch in text.chars() {
            if ch == '\n' {
                cursor_x = x;
                cursor_y += size;
                continue;
            }
            let glyph = match self.font_atlas.glyph(ch) {
                Some(g) => g,
                None => continue,
            };
            if glyph.width == 0 || glyph.height == 0 {
                cursor_x += glyph.advance * scale;
                continue;
            }

            let gx = cursor_x + glyph.offset_x * scale;
            let gy = cursor_y + (size - glyph.offset_y * scale - glyph.height as f32 * scale);
            let gw = glyph.width as f32 * scale;
            let gh = glyph.height as f32 * scale;

            let u0 = glyph.x as f32 / atlas_w;
            let v0 = glyph.y as f32 / atlas_h;
            let u1 = (glyph.x + glyph.width) as f32 / atlas_w;
            let v1 = (glyph.y + glyph.height) as f32 / atlas_h;

            self.draw_list.add_textured_quad(
                Vertex::with_uv(gx, gy, c, u0, v0),
                Vertex::with_uv(gx + gw, gy, c, u1, v0),
                Vertex::with_uv(gx + gw, gy + gh, c, u1, v1),
                Vertex::with_uv(gx, gy + gh, c, u0, v1),
            );

            cursor_x += glyph.advance * scale;
        }
    }

    /// Measure the bounding box of text at the given size.
    pub fn text_bounds(&self, text: &str, size: f32) -> (f32, f32) {
        let scale = size / DEFAULT_FONT_SIZE;
        let (w, h) = self.font_atlas.measure(text);
        (w * scale, h * scale)
    }

    /// Draw a crosshair.
    pub fn crosshair(&mut self, x: f32, y: f32, size: f32, thickness: f32, color: Color) {
        let half = size * 0.5;
        self.line(x - half, y, x + half, y, thickness, color);
        self.line(x, y - half, x, y + half, thickness, color);
    }

    /// Draw a health bar with foreground fill and background.
    pub fn health_bar(&mut self, x: f32, y: f32, w: f32, h: f32, pct: f32, fg: Color, bg: Color) {
        let pct = pct.clamp(0.0, 1.0);
        self.rect_filled(x, y, w, h, bg);
        if pct > 0.0 {
            self.rect_filled(x, y, w * pct, h, fg);
        }
        self.rect(x, y, w, h, Color::BLACK);
    }

    /// Draw an ESP-style bounding box with an optional label.
    pub fn esp_box(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color, label: Option<&str>) {
        self.rect(x, y, w, h, color);
        if let Some(text) = label {
            let (tw, _th) = self.text_bounds(text, DEFAULT_FONT_SIZE);
            let tx = x + (w - tw) * 0.5;
            let ty = y - DEFAULT_FONT_SIZE - 2.0;
            self.text(tx, ty, text, DEFAULT_FONT_SIZE, color);
        }
    }
}
