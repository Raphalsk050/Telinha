//! Small ease-in-out transitions, so hovers, toggles and panels slide between states instead of
//! jumping. Each one is keyed by an id and follows a bool or a number over a fraction of a second.

use egui::{Color32, Context, Id};

/// How long a hover takes to light up or fade out.
pub const HOVER: f32 = 0.16;
/// How long a button takes to change state (muted, sharing, camera on).
pub const STATE: f32 = 0.22;
/// How long a panel or popup takes to open.
pub const OPEN: f32 = 0.26;

/// 0 when `on` is false, 1 when true, easing between the two over `seconds`.
pub fn ease(ctx: &Context, id: Id, on: bool, seconds: f32) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, on, seconds, egui::emath::easing::cubic_in_out)
}

/// The hover of `id`, eased.
pub fn hover(ctx: &Context, id: Id, hovered: bool) -> f32 {
    ease(ctx, id.with("motion-hover"), hovered, HOVER)
}

/// A color that slides to `target` instead of jumping: each channel follows the target over
/// `seconds`. Good for a fill whose state can change in many ways at once.
pub fn color(ctx: &Context, id: Id, target: Color32, seconds: f32) -> Color32 {
    let [r, g, b, a] = target.to_array();
    let channel = |which: &str, value: u8| ctx.animate_value_with_time(id.with(("motion-color", which)), value as f32, seconds).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_premultiplied(channel("r", r), channel("g", g), channel("b", b), channel("a", a))
}

/// `from` at 0, `to` at 1.
pub fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    from.lerp_to_gamma(to, t.clamp(0.0, 1.0))
}

/// A number from `from` at 0 to `to` at 1.
pub fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_lands_on_both_ends() {
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 1.0), Color32::WHITE);
        assert_eq!(lerp(1.0, 1.1, 0.5), 1.05);
    }
}
