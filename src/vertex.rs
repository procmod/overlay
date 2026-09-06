use crate::clip::ClipRect;

/// A vertex for the overlay's 2D rendering pipeline.
///
/// Position is in pixel coordinates (top-left origin). Color is normalized RGBA.
/// UV coordinates are used for font atlas sampling (0,0 for solid-color geometry).
/// Params carry the outline width in pixels and the atlas-to-screen scale, and are read
/// only by the glyph outline shader.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub uv: [f32; 2],
    pub params: [f32; 2],
}

impl Vertex {
    pub fn new(x: f32, y: f32, color: [f32; 4]) -> Self {
        Self {
            position: [x, y],
            color,
            uv: [0.0, 0.0],
            params: [0.0, 0.0],
        }
    }

    pub fn with_uv(x: f32, y: f32, color: [f32; 4], u: f32, v: f32) -> Self {
        Self {
            position: [x, y],
            color,
            uv: [u, v],
            params: [0.0, 0.0],
        }
    }
}

/// Which pipeline state a run of indices needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandKind {
    Solid,
    Glyph,
    GlyphOutline,
}

#[derive(Debug, Clone)]
pub(crate) struct DrawCommand {
    pub kind: CommandKind,
    pub clip: Option<ClipRect>,
    pub index_offset: u32,
    pub index_count: u32,
}

/// Accumulates vertices and indices during a frame.
pub(crate) struct DrawList {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub commands: Vec<DrawCommand>,
    clips: Vec<ClipRect>,
}

impl DrawList {
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            commands: Vec::new(),
            clips: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.commands.clear();
        self.clips.clear();
    }

    pub fn push_clip_rect(&mut self, rect: ClipRect) {
        let rect = rect.canonical();
        self.clips.push(
            self.clips
                .last()
                .map_or(rect, |parent| parent.intersect(rect)),
        );
    }

    pub fn pop_clip_rect(&mut self) {
        self.clips.pop();
    }

    pub fn add_solid_quad(&mut self, v0: Vertex, v1: Vertex, v2: Vertex, v3: Vertex) {
        self.push_quad(CommandKind::Solid, [v0, v1, v2, v3]);
    }

    pub fn add_glyph_quad(&mut self, v0: Vertex, v1: Vertex, v2: Vertex, v3: Vertex) {
        self.push_quad(CommandKind::Glyph, [v0, v1, v2, v3]);
    }

    pub fn add_glyph_outline_quad(
        &mut self,
        v0: Vertex,
        v1: Vertex,
        v2: Vertex,
        v3: Vertex,
        params: [f32; 2],
    ) {
        let mut quad = [v0, v1, v2, v3];
        for vertex in &mut quad {
            vertex.params = params;
        }
        self.push_quad(CommandKind::GlyphOutline, quad);
    }

    pub fn add_solid_triangles(&mut self, verts: &[Vertex], idxs: &[u32]) {
        if idxs.is_empty() {
            return;
        }
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(verts);
        self.indices.extend(idxs.iter().map(|i| i + base));
        self.push_command(CommandKind::Solid, idxs.len() as u32);
    }

    fn push_quad(&mut self, kind: CommandKind, quad: [Vertex; 4]) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&quad);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.push_command(kind, 6);
    }

    /// Append a run of indices, extending the previous command when it needs the same state.
    fn push_command(&mut self, kind: CommandKind, index_count: u32) {
        let clip = self.clips.last().copied();
        let index_offset = self.indices.len() as u32 - index_count;
        if let Some(last) = self.commands.last_mut() {
            if last.kind == kind
                && last.clip == clip
                && last.index_offset + last.index_count == index_offset
            {
                last.index_count += index_count;
                return;
            }
        }
        self.commands.push(DrawCommand {
            kind,
            clip,
            index_offset,
            index_count,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    fn quad(dl: &mut DrawList) {
        dl.add_solid_quad(
            Vertex::new(0.0, 0.0, C),
            Vertex::new(1.0, 0.0, C),
            Vertex::new(1.0, 1.0, C),
            Vertex::new(0.0, 1.0, C),
        );
    }

    fn glyph_quad(dl: &mut DrawList) {
        dl.add_glyph_quad(
            Vertex::with_uv(0.0, 0.0, C, 0.0, 0.0),
            Vertex::with_uv(1.0, 0.0, C, 1.0, 0.0),
            Vertex::with_uv(1.0, 1.0, C, 1.0, 1.0),
            Vertex::with_uv(0.0, 1.0, C, 0.0, 1.0),
        );
    }

    #[test]
    fn clip_stack_is_snapshotted_intersected_and_reset() {
        let mut dl = DrawList::new();
        let outer = ClipRect::new(1.0, 2.0, 20.0, 30.0);
        quad(&mut dl);
        dl.push_clip_rect(outer);
        quad(&mut dl);
        quad(&mut dl);
        dl.push_clip_rect(ClipRect::new(10.0, 0.0, 40.0, 12.0));
        glyph_quad(&mut dl);
        let v = Vertex::new(0.0, 0.0, C);
        dl.add_glyph_outline_quad(v, v, v, v, [1.0, 1.0]);
        dl.pop_clip_rect();
        quad(&mut dl);
        dl.pop_clip_rect();
        dl.pop_clip_rect();
        quad(&mut dl);
        assert_eq!(dl.commands.len(), 6);
        assert_eq!(dl.commands[0].clip, None);
        assert_eq!(dl.commands[1].clip, Some(outer));
        assert_eq!(dl.commands[1].index_count, 12);
        assert_eq!(
            dl.commands[2].clip,
            Some(ClipRect::new(10.0, 2.0, 20.0, 12.0))
        );
        assert_eq!(dl.commands[3].clip, dl.commands[2].clip);
        assert_eq!(dl.commands[4].clip, Some(outer));
        assert_eq!(dl.commands[5].clip, None);
        dl.push_clip_rect(outer);
        dl.clear();
        quad(&mut dl);
        assert_eq!(dl.commands[0].clip, None);
    }

    #[test]
    fn empty_nested_clip_cannot_be_reopened_until_popped() {
        let mut dl = DrawList::new();
        dl.push_clip_rect(ClipRect::new(f32::NAN, 0.0, 1.0, 1.0));
        dl.push_clip_rect(ClipRect::new(-10.0, -10.0, 10.0, 10.0));
        quad(&mut dl);
        assert_eq!(crate::clip::scissor(dl.commands[0].clip, 100, 100), [0; 4]);
        dl.pop_clip_rect();
        quad(&mut dl);
        assert_eq!(dl.commands.len(), 1);
    }

    #[test]
    fn vertex_new_sets_zero_uv() {
        let v = Vertex::new(10.0, 20.0, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(v.uv, [0.0, 0.0]);
        assert_eq!(v.params, [0.0, 0.0]);
    }

    #[test]
    fn vertex_with_uv_preserves_all_fields() {
        let v = Vertex::with_uv(5.0, 10.0, [0.5, 0.5, 0.5, 1.0], 0.25, 0.75);
        assert_eq!(v.position, [5.0, 10.0]);
        assert_eq!(v.uv, [0.25, 0.75]);
    }

    #[test]
    fn draw_list_solid_quad_indices() {
        let mut dl = DrawList::new();
        quad(&mut dl);
        assert_eq!(dl.vertices.len(), 4);
        assert_eq!(dl.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(dl.commands.len(), 1);
    }

    #[test]
    fn draw_list_second_quad_offsets_indices() {
        let mut dl = DrawList::new();
        quad(&mut dl);
        quad(&mut dl);
        assert_eq!(dl.vertices.len(), 8);
        assert_eq!(&dl.indices[6..], &[4, 5, 6, 4, 6, 7]);
    }

    #[test]
    fn draw_list_clear_resets() {
        let mut dl = DrawList::new();
        quad(&mut dl);
        dl.clear();
        assert!(dl.vertices.is_empty());
        assert!(dl.indices.is_empty());
        assert!(dl.commands.is_empty());
    }

    #[test]
    fn solid_triangles_offsets_correctly() {
        let mut dl = DrawList::new();
        quad(&mut dl);
        dl.add_solid_triangles(
            &[
                Vertex::new(0.0, 0.0, C),
                Vertex::new(1.0, 0.0, C),
                Vertex::new(0.5, 1.0, C),
            ],
            &[0, 1, 2],
        );
        assert_eq!(dl.indices.last(), Some(&6));
    }

    #[test]
    fn matching_commands_merge() {
        let mut dl = DrawList::new();
        quad(&mut dl);
        quad(&mut dl);
        quad(&mut dl);
        assert_eq!(dl.commands.len(), 1);
        assert_eq!(dl.commands[0].index_count, 18);
        assert_eq!(dl.commands[0].index_offset, 0);
    }

    #[test]
    fn differing_commands_do_not_merge() {
        let mut dl = DrawList::new();
        quad(&mut dl);
        glyph_quad(&mut dl);
        quad(&mut dl);
        assert_eq!(dl.commands.len(), 3);
        assert_eq!(dl.commands[1].kind, CommandKind::Glyph);
        assert_eq!(dl.commands[1].index_offset, 6);
        assert_eq!(dl.commands[2].index_offset, 12);
    }

    #[test]
    fn merged_commands_cover_every_index() {
        let mut dl = DrawList::new();
        quad(&mut dl);
        glyph_quad(&mut dl);
        glyph_quad(&mut dl);
        dl.add_solid_triangles(
            &[
                Vertex::new(0.0, 0.0, C),
                Vertex::new(1.0, 0.0, C),
                Vertex::new(0.5, 1.0, C),
            ],
            &[0, 1, 2],
        );

        let covered: u32 = dl.commands.iter().map(|c| c.index_count).sum();
        assert_eq!(covered as usize, dl.indices.len());

        let mut expected = 0;
        for command in &dl.commands {
            assert_eq!(command.index_offset, expected);
            expected += command.index_count;
        }
    }

    #[test]
    fn outline_quad_stamps_params_on_every_vertex() {
        let mut dl = DrawList::new();
        dl.add_glyph_outline_quad(
            Vertex::with_uv(0.0, 0.0, C, 0.0, 0.0),
            Vertex::with_uv(1.0, 0.0, C, 1.0, 0.0),
            Vertex::with_uv(1.0, 1.0, C, 1.0, 1.0),
            Vertex::with_uv(0.0, 1.0, C, 0.0, 1.0),
            [2.0, 1.5],
        );
        assert!(dl.vertices.iter().all(|v| v.params == [2.0, 1.5]));
        assert_eq!(dl.commands[0].kind, CommandKind::GlyphOutline);
    }
}
