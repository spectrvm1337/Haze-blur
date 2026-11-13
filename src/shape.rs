//! Squircle corners. Composition rounds rectangles with circular arcs only, so a rectangle with
//! superellipse corners is drawn as a Direct2D path and handed over as a `CompositionPath`.
use windows::core::{implement, Interface, Ref};
use windows::Graphics::{IGeometrySource2D, IGeometrySource2D_Impl};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED};
use windows::Win32::Graphics::Direct2D::{ID2D1Factory, ID2D1Geometry};
use windows::Win32::System::WinRT::Graphics::Direct2D::{IGeometrySource2DInterop, IGeometrySource2DInterop_Impl};
use windows_numerics::Vector2;

/// Points per corner. Every outline has the same number, so Composition can animate one into another.
const STEPS: usize = 16;

/// The outline of a `w`×`h` rectangle whose corners of radius `r` are the superellipse
/// |x|ⁿ + |y|ⁿ = 1 with n = 2^k, as CSS `superellipse(k)` draws them: k = 1 is a circular arc,
/// k = 2 a squircle. Clockwise from the top edge.
pub(crate) fn outline(w: f32, h: f32, r: f32, k: f32) -> Vec<Vector2> {
    use std::f32::consts::FRAC_PI_2;
    let r = r.max(0.0).min(w / 2.0).min(h / 2.0);
    let p = 2.0 / 2f32.powf(k.clamp(0.0, 8.0));
    // Corner centre, which way it bulges, and whether the angle rises or falls, so the outline runs clockwise.
    let corners = [
        (w - r, r, 1.0, -1.0, false),   // top right: from the top edge down to the right edge
        (w - r, h - r, 1.0, 1.0, true), // bottom right: from the right edge to the bottom edge
        (r, h - r, -1.0, 1.0, false),   // bottom left: from the bottom edge to the left edge
        (r, r, -1.0, -1.0, true),       // top left: from the left edge up to the top edge
    ];
    let mut points = Vec::with_capacity(4 * (STEPS + 1));
    for (cx, cy, sx, sy, rising) in corners {
        for i in 0..=STEPS {
            let f = i as f32 / STEPS as f32;
            let t = if rising { f } else { 1.0 - f } * FRAC_PI_2;
            points.push(Vector2 { X: cx + sx * r * t.cos().max(0.0).powf(p), Y: cy + sy * r * t.sin().max(0.0).powf(p) });
        }
    }
    points
}

/// The outline as a geometry Composition can take.
pub(crate) fn source(factory: &ID2D1Factory, points: Vec<Vector2>) -> windows::core::Result<IGeometrySource2D> {
    let own = build(factory, &points)?;
    Ok(Shape { own, points }.into())
}

/// The closed outline through `points`, as a geometry of `factory`.
fn build(factory: &ID2D1Factory, points: &[Vector2]) -> windows::core::Result<ID2D1Geometry> {
    let geometry = unsafe { factory.CreatePathGeometry()? };
    unsafe {
        let sink = geometry.Open()?;
        sink.BeginFigure(points[0], D2D1_FIGURE_BEGIN_FILLED);
        sink.AddLines(&points[1..]);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
    }
    geometry.cast()
}

/// Hands a Direct2D geometry to Composition (what Win2D's `CanvasGeometry` does in C#). Composition
/// draws with a Direct2D factory of its own and may ask for the geometry made on it; a geometry from
/// another factory is an error to Direct2D, so the points are kept and built again on request.
#[implement(IGeometrySource2D, IGeometrySource2DInterop)]
struct Shape {
    own: ID2D1Geometry,
    points: Vec<Vector2>,
}

impl IGeometrySource2D_Impl for Shape_Impl {}

impl IGeometrySource2DInterop_Impl for Shape_Impl {
    fn GetGeometry(&self) -> windows::core::Result<ID2D1Geometry> {
        Ok(self.own.clone())
    }
    fn TryGetGeometryUsingFactory(&self, factory: Ref<'_, ID2D1Factory>) -> windows::core::Result<ID2D1Geometry> {
        build(factory.ok()?, &self.points)
    }
}

#[cfg(test)]
mod tests {
    use super::outline;

    #[test]
    fn k_of_1_is_a_circular_arc() {
        let r = 12.0;
        let mid = outline(100.0, 40.0, r, 1.0)[8];
        let away = r * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
        assert!(((100.0 - mid.X) - away).abs() < 0.05 && (mid.Y - away).abs() < 0.05);
    }

    #[test]
    fn a_squircle_touches_the_edges_and_bulges_past_the_arc() {
        let (w, h, r) = (100.0, 40.0, 12.0);
        let pts = outline(w, h, r, 2.0);
        assert_eq!(pts.len(), 4 * 17);
        assert!(pts.iter().all(|p| p.X >= -1e-3 && p.X <= w + 1e-3 && p.Y >= -1e-3 && p.Y <= h + 1e-3));
        assert!((pts[0].X - (w - r)).abs() < 1e-3 && pts[0].Y.abs() < 1e-3);
        assert!((pts[16].X - w).abs() < 1e-3 && (pts[16].Y - r).abs() < 1e-3);
        // Halfway round the corner a squircle is r·(1 − 2^-¼) ≈ 0.159·r from the corner on each axis;
        // a circular arc would be 0.293·r away.
        let mid = pts[8];
        assert!(((w - mid.X) - 0.159 * r).abs() < 0.05 && (mid.Y - 0.159 * r).abs() < 0.05);
    }

    #[test]
    fn the_radius_is_held_to_half_the_shorter_side() {
        let pts = outline(40.0, 20.0, 50.0, 2.0);
        assert!(pts.iter().all(|p| p.X >= -1e-3 && p.X <= 40.0 + 1e-3 && p.Y >= -1e-3 && p.Y <= 20.0 + 1e-3));
    }
}
