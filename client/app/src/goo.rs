//! Circles that melt into each other, as drops of a liquid: the buttons of a call come out of one
//! pill and pull apart, stretching thin necks between them before they let go, as the circles of
//! the Spotlight of the Mac.
//!
//! Each circle is its distance field, negative inside; the fields are joined with a smooth
//! minimum, which rounds the corner where two circles meet into a neck as wide as `melt`. Circles
//! whose edges are closer than half of `melt` stay one piece; farther ones part.

use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Vec2};

/// The smooth minimum of two distances, rounding their meeting over `melt`.
fn smooth_min(a: f32, b: f32, melt: f32) -> f32 {
    if melt <= 0.0 {
        return a.min(b);
    }
    let h = (melt - (a - b).abs()).max(0.0) / melt;
    a.min(b) - h * h * melt / 4.0
}

/// The distance from `at` to the melted shape, negative inside.
fn distance(at: Pos2, centers: &[Pos2], radius: f32, melt: f32) -> f32 {
    centers.iter().map(|center| (at - *center).length() - radius).reduce(|a, b| smooth_min(a, b, melt)).unwrap_or(f32::INFINITY)
}

/// Paints the melted shape of the circles at `centers`, all of `radius`, each in its color, with
/// soft edges a pixel wide. Where two circles melt, their colors run into each other.
pub fn paint(painter: &Painter, centers: &[Pos2], colors: &[Color32], radius: f32, melt: f32) {
    if centers.is_empty() || radius < 0.5 {
        return;
    }
    let mut bounds = Rect::NOTHING;
    for center in centers {
        bounds = bounds.union(Rect::from_center_size(*center, Vec2::splat(radius * 2.0 + 4.0)));
    }
    let bounds = bounds.intersect(painter.clip_rect());
    let step = 1.0 / painter.pixels_per_point().max(1.0);
    let mut mesh = Mesh::default();
    let mut y = bounds.top();
    while y < bounds.bottom() {
        let mut x = bounds.left();
        while x < bounds.right() {
            let at = Pos2::new(x + step / 2.0, y + step / 2.0);
            let alpha = (0.5 - distance(at, centers, radius, melt) / step).clamp(0.0, 1.0);
            if alpha > 0.0 {
                // The color of each circle counts the more the nearer it is.
                let (mut total, mut paint) = (0.0, [0.0f32; 4]);
                for (center, color) in centers.iter().zip(colors) {
                    let weight = (-((at - *center).length() - radius).max(0.0) / 6.0).exp();
                    total += weight;
                    for (channel, value) in paint.iter_mut().zip(color.to_array()) {
                        *channel += weight * f32::from(value);
                    }
                }
                let [r, g, b, a] = paint.map(|channel| (channel / total * alpha).round().clamp(0.0, 255.0) as u8);
                mesh.add_colored_rect(Rect::from_min_size(Pos2::new(x, y), Vec2::splat(step)), Color32::from_rgba_premultiplied(r, g, b, a));
            }
            x += step;
        }
        y += step;
    }
    painter.add(Shape::mesh(mesh));
}

/// Whether the circles at `centers` still touch, so the shape is one piece.
pub fn joined(centers: &[Pos2], radius: f32, melt: f32) -> bool {
    centers.windows(2).all(|pair| distance(pair[0] + (pair[1] - pair[0]) / 2.0, centers, radius, melt) <= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_circles_melt_and_far_ones_keep_apart() {
        let row = |gap: f32| -> Vec<Pos2> { (0..4).map(|index| Pos2::new(index as f32 * gap, 0.0)).collect() };
        assert!(joined(&row(18.0), 24.0, 20.0));
        assert!(joined(&row(57.0), 24.0, 20.0));
        // The buttons of the call sit 60 apart: apart.
        assert!(!joined(&row(60.0), 24.0, 20.0));
        // Close together, with a small melt, they make a pill no taller than one of them.
        assert!(distance(Pos2::new(27.0, -24.5), &row(18.0), 24.0, 8.0) > 0.0);
    }
}
