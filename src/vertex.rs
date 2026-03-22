/// A vertex for the overlay's 2D rendering pipeline.
///
/// Position is in pixel coordinates (top-left origin). Color is normalized RGBA.
/// UV coordinates are used for font atlas sampling (0,0 for solid-color geometry).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub uv: [f32; 2],
}

impl Vertex {
    pub fn new(x: f32, y: f32, color: [f32; 4]) -> Self {
        Self {
            position: [x, y],
            color,
            uv: [0.0, 0.0],
        }
    }

    pub fn with_uv(x: f32, y: f32, color: [f32; 4], u: f32, v: f32) -> Self {
        Self {
            position: [x, y],
            color,
            uv: [u, v],
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum DrawCommand {
    Solid {
        vertex_offset: u32,
        index_offset: u32,
        index_count: u32,
    },
    Textured {
        vertex_offset: u32,
        index_offset: u32,
        index_count: u32,
    },
}

/// Accumulates vertices and indices during a frame.
pub(crate) struct DrawList {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub commands: Vec<DrawCommand>,
}

impl DrawList {
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            commands: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.commands.clear();
    }

    pub fn add_solid_quad(&mut self, v0: Vertex, v1: Vertex, v2: Vertex, v3: Vertex) {
        let base = self.vertices.len() as u32;
        let idx_offset = self.indices.len() as u32;
        self.vertices.extend_from_slice(&[v0, v1, v2, v3]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.commands.push(DrawCommand::Solid {
            vertex_offset: base,
            index_offset: idx_offset,
            index_count: 6,
        });
    }

    pub fn add_textured_quad(&mut self, v0: Vertex, v1: Vertex, v2: Vertex, v3: Vertex) {
        let base = self.vertices.len() as u32;
        let idx_offset = self.indices.len() as u32;
        self.vertices.extend_from_slice(&[v0, v1, v2, v3]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        self.commands.push(DrawCommand::Textured {
            vertex_offset: base,
            index_offset: idx_offset,
            index_count: 6,
        });
    }

    pub fn add_solid_triangles(&mut self, verts: &[Vertex], idxs: &[u32]) {
        let base = self.vertices.len() as u32;
        let idx_offset = self.indices.len() as u32;
        self.vertices.extend_from_slice(verts);
        self.indices.extend(idxs.iter().map(|i| i + base));
        self.commands.push(DrawCommand::Solid {
            vertex_offset: base,
            index_offset: idx_offset,
            index_count: idxs.len() as u32,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_new_sets_zero_uv() {
        let v = Vertex::new(10.0, 20.0, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(v.uv, [0.0, 0.0]);
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
        let c = [1.0, 1.0, 1.0, 1.0];
        dl.add_solid_quad(
            Vertex::new(0.0, 0.0, c),
            Vertex::new(1.0, 0.0, c),
            Vertex::new(1.0, 1.0, c),
            Vertex::new(0.0, 1.0, c),
        );
        assert_eq!(dl.vertices.len(), 4);
        assert_eq!(dl.indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(dl.commands.len(), 1);
    }

    #[test]
    fn draw_list_second_quad_offsets_indices() {
        let mut dl = DrawList::new();
        let c = [1.0, 1.0, 1.0, 1.0];
        let quad = |dl: &mut DrawList| {
            dl.add_solid_quad(
                Vertex::new(0.0, 0.0, c),
                Vertex::new(1.0, 0.0, c),
                Vertex::new(1.0, 1.0, c),
                Vertex::new(0.0, 1.0, c),
            );
        };
        quad(&mut dl);
        quad(&mut dl);
        assert_eq!(dl.vertices.len(), 8);
        assert_eq!(&dl.indices[6..], &[4, 5, 6, 4, 6, 7]);
    }

    #[test]
    fn draw_list_clear_resets() {
        let mut dl = DrawList::new();
        let c = [1.0; 4];
        dl.add_solid_quad(
            Vertex::new(0.0, 0.0, c),
            Vertex::new(1.0, 0.0, c),
            Vertex::new(1.0, 1.0, c),
            Vertex::new(0.0, 1.0, c),
        );
        dl.clear();
        assert!(dl.vertices.is_empty());
        assert!(dl.indices.is_empty());
        assert!(dl.commands.is_empty());
    }

    #[test]
    fn solid_triangles_offsets_correctly() {
        let mut dl = DrawList::new();
        let c = [1.0; 4];
        dl.add_solid_quad(
            Vertex::new(0.0, 0.0, c),
            Vertex::new(1.0, 0.0, c),
            Vertex::new(1.0, 1.0, c),
            Vertex::new(0.0, 1.0, c),
        );
        dl.add_solid_triangles(
            &[
                Vertex::new(0.0, 0.0, c),
                Vertex::new(1.0, 0.0, c),
                Vertex::new(0.5, 1.0, c),
            ],
            &[0, 1, 2],
        );
        assert_eq!(dl.indices.last(), Some(&6));
    }
}
