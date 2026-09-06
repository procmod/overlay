use crate::vertex::{DrawList, Vertex};

fn points(input: &[[f32; 2]], closed: bool) -> Option<Vec<[f64; 2]>> {
    if input.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let mut p: Vec<_> = input.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
    p.dedup();
    if closed && p.len() > 1 && p.first() == p.last() {
        p.pop();
    }
    Some(p)
}

fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn emit(dl: &mut DrawList, p: &[[f64; 2]], indices: &[u32], color: [f32; 4]) {
    let verts: Vec<_> = p
        .iter()
        .map(|p| Vertex::new(p[0] as f32, p[1] as f32, color))
        .collect();
    if verts.iter().flat_map(|v| v.position).all(|v| v.is_finite()) {
        dl.add_solid_triangles(&verts, indices);
    }
}

pub(crate) fn convex_polygon_filled(dl: &mut DrawList, input: &[[f32; 2]], color: [f32; 4]) {
    let Some(p) = points(input, true) else { return };
    if p.len() < 3 || p.len() > u32::MAX as usize {
        return;
    }
    for i in 0..p.len() {
        if p[i + 1..].contains(&p[i]) {
            return;
        }
        let a = p[(i + p.len() - 1) % p.len()];
        let b = p[i];
        let c = p[(i + 1) % p.len()];
        if cross(a, b, c) == 0.0
            && (b[0] - a[0]) * (c[0] - b[0]) + (b[1] - a[1]) * (c[1] - b[1]) < 0.0
        {
            return;
        }
    }
    // every vertex must lie in the same half-plane of every directed edge;
    // unlike a turn-only check this also rejects self-intersecting stars
    let mut sign = 0.0_f64;
    for i in 0..p.len() {
        let a = p[i];
        let b = p[(i + 1) % p.len()];
        for &c in &p {
            let turn = cross(a, b, c);
            if turn != 0.0 {
                if sign != 0.0 && sign != turn.signum() {
                    return;
                }
                sign = turn.signum();
            }
        }
    }
    if sign == 0.0 {
        return;
    }
    let mut indices = Vec::new();
    for i in 1..p.len() - 1 {
        if cross(p[0], p[i], p[i + 1]) != 0.0 {
            indices.extend_from_slice(&[0, i as u32, i as u32 + 1]);
        }
    }
    emit(dl, &p, &indices, color);
}

pub(crate) fn polyline(
    dl: &mut DrawList,
    input: &[[f32; 2]],
    closed: bool,
    thickness: f32,
    color: [f32; 4],
) {
    if !thickness.is_finite() || thickness <= 0.0 {
        return;
    }
    let Some(p) = points(input, closed) else {
        return;
    };
    if p.len() < 2 || p.len() > (u32::MAX / 2) as usize {
        return;
    }
    let closed = closed && p.len() > 2;
    let count = if closed { p.len() } else { p.len() - 1 };
    let normals: Vec<_> = (0..count)
        .map(|i| {
            let a = p[i];
            let b = p[(i + 1) % p.len()];
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let len = dx.hypot(dy);
            [-dy / len, dx / len]
        })
        .collect();
    let half = thickness as f64 * 0.5;
    let mut verts = Vec::with_capacity(p.len() * 2);
    for (i, &point) in p.iter().enumerate() {
        let n = if !closed && i == 0 {
            normals[0]
        } else if !closed && i == p.len() - 1 {
            normals[count - 1]
        } else {
            let a = normals[(i + count - 1) % count];
            let b = normals[i % count];
            let sum = [a[0] + b[0], a[1] + b[1]];
            let len = sum[0].hypot(sum[1]);
            if len < 1e-12 {
                b
            } else {
                let unit = [sum[0] / len, sum[1] / len];
                let scale = (2.0 / len).min(4.0);
                [unit[0] * scale, unit[1] * scale]
            }
        };
        verts.push([point[0] + n[0] * half, point[1] + n[1] * half]);
        verts.push([point[0] - n[0] * half, point[1] - n[1] * half]);
    }
    let mut indices = Vec::with_capacity(count * 6);
    for i in 0..count {
        let a = (i * 2) as u32;
        let b = (((i + 1) % p.len()) * 2) as u32;
        indices.extend_from_slice(&[a, a + 1, b + 1, a, b + 1, b]);
    }
    emit(dl, &verts, &indices, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    const C: [f32; 4] = [1.0; 4];
    fn area(dl: &DrawList) -> f64 {
        dl.indices
            .chunks_exact(3)
            .map(|t| {
                let p = |i: u32| dl.vertices[i as usize].position.map(f64::from);
                cross(p(t[0]), p(t[1]), p(t[2])).abs() / 2.0
            })
            .sum()
    }
    #[test]
    fn polygon_windings_duplicates_and_collinear_edges() {
        let mut p = vec![
            [0., 0.],
            [0., 0.],
            [1., 0.],
            [2., 0.],
            [2., 2.],
            [0., 2.],
            [0., 0.],
        ];
        for _ in 0..2 {
            let mut dl = DrawList::new();
            convex_polygon_filled(&mut dl, &p, C);
            assert_eq!(area(&dl), 4.0);
            assert!(dl.indices.iter().all(|&i| (i as usize) < dl.vertices.len()));
            p.reverse();
        }
    }
    #[test]
    fn polygon_rejects_invalid_inputs() {
        for p in [
            vec![],
            vec![[0., 0.]],
            vec![[0., 0.], [1., 1.], [2., 2.]],
            vec![[0., 0.], [2., 2.], [0., 2.], [2., 0.]],
            vec![[0., 0.], [2., 0.], [1., 0.5], [2., 2.], [0., 2.]],
            vec![[0., 0.], [1., 0.], [f32::NAN, 1.]],
        ] {
            let mut dl = DrawList::new();
            convex_polygon_filled(&mut dl, &p, C);
            assert!(dl.commands.is_empty());
        }
    }
    #[test]
    fn polygon_rejects_stars_repeated_vertices_and_backtracking() {
        let ring: Vec<_> = (0..5)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 5.0;
                [a.cos(), a.sin()]
            })
            .collect();
        let star: Vec<_> = [0, 2, 4, 1, 3].map(|i| ring[i]).to_vec();
        for p in [
            star,
            vec![[0., 0.], [2., 0.], [2., 2.], [0., 0.], [0., 2.]],
            vec![[0., 0.], [2., 0.], [1., 0.], [2., 2.], [0., 2.]],
        ] {
            let mut dl = DrawList::new();
            convex_polygon_filled(&mut dl, &p, C);
            assert!(dl.indices.is_empty());
        }
    }

    #[test]
    fn polygon_fans_preserve_area_across_sizes_and_windings() {
        for n in 3..50 {
            let mut p: Vec<_> = (0..n)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / n as f32;
                    [a.cos() * 20.0, a.sin() * 10.0]
                })
                .collect();
            for _ in 0..2 {
                let expected: f64 = (0..n)
                    .map(|i| {
                        let a = p[i].map(f64::from);
                        let b = p[(i + 1) % n].map(f64::from);
                        a[0] * b[1] - a[1] * b[0]
                    })
                    .sum::<f64>()
                    .abs()
                    / 2.0;
                let mut dl = DrawList::new();
                convex_polygon_filled(&mut dl, &p, C);
                assert_eq!(dl.indices.len(), (n - 2) * 3);
                assert!((area(&dl) - expected).abs() < 1e-9);
                p.reverse();
            }
        }
    }

    #[test]
    fn polyline_miter_bound_and_atomic_overflow_rejection() {
        let mut dl = DrawList::new();
        let p = [[0., 0.], [10., 0.], [0., 0.001]];
        polyline(&mut dl, &p, false, 2., C);
        for (i, point) in p.iter().enumerate() {
            for v in &dl.vertices[i * 2..i * 2 + 2] {
                assert!((v.position[0] - point[0]).hypot(v.position[1] - point[1]) <= 4.00001);
            }
        }
        let before = dl.indices.len();
        polyline(
            &mut dl,
            &[[f32::MAX, 0.], [f32::MAX, 1.]],
            false,
            f32::MAX,
            C,
        );
        assert_eq!(dl.indices.len(), before);
        for p in [vec![], vec![[1., 1.]], vec![[1., 1.], [1., 1.]]] {
            polyline(&mut dl, &p, true, 1., C);
            assert_eq!(dl.indices.len(), before);
        }
    }

    #[test]
    fn polyline_caps_closure_and_duplicate_points() {
        let mut dl = DrawList::new();
        polyline(&mut dl, &[[0., 0.], [0., 0.], [10., 0.]], false, 2., C);
        assert_eq!(area(&dl), 20.);
        assert_eq!(dl.vertices[0].position, [0., 1.]);
        dl.clear();
        polyline(
            &mut dl,
            &[[0., 0.], [10., 0.], [10., 10.], [0., 10.], [0., 0.]],
            true,
            2.,
            C,
        );
        assert_eq!(dl.vertices.len(), 8);
        assert_eq!(dl.indices.len(), 24);
        assert_eq!(area(&dl), 80.);
    }
    #[test]
    fn polyline_invalid_and_extreme_geometry_is_finite_or_empty() {
        let mut dl = DrawList::new();
        for t in [0., -1., f32::NAN, f32::INFINITY] {
            polyline(&mut dl, &[[0., 0.], [1., 1.]], false, t, C);
        }
        polyline(&mut dl, &[[0., 0.], [f32::INFINITY, 0.]], false, 1., C);
        assert!(dl.commands.is_empty());
        for p in [
            vec![[0., 0.], [1., 0.], [0., 0.]],
            vec![[0., 0.], [1., 0.], [0., 0.000001]],
            vec![[-f32::MAX, 0.], [f32::MAX, 0.]],
            vec![[0., 0.], [f32::MIN_POSITIVE, 0.]],
        ] {
            dl.clear();
            polyline(&mut dl, &p, false, 2., C);
            assert!(!dl.vertices.is_empty());
            assert!(dl
                .vertices
                .iter()
                .flat_map(|v| v.position)
                .all(f32::is_finite));
        }
    }
}
