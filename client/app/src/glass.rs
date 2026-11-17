//! The "Vidro" theme (glassmorphism): translucent panels that blur what is behind them, over a
//! dark wallpaper with colored lights. The same look and the same five knobs as the HTML sketch
//! in `design/sketch/glass.css`.
//!
//! The blur is live. Each glass leaves a placeholder where it is painted; at the end of the
//! frame, [`finish`] takes the shapes already painted behind it (the layers below and the shapes
//! before it in its own layer), and a paint callback draws them again into a texture of its own,
//! blurs it on the GPU and draws it back masked to the rounded rectangle. Text scrolling behind
//! the message box goes soft as in the sketch, also while the window changes.

use std::collections::HashMap;

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor};
use eframe::wgpu;
use egui::epaint::{ClippedPrimitive, ClippedShape, Primitive, TextureId};
use egui::{Color32, Context, CornerRadius, Id, LayerId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::appearance::GlassLook;

/// The thin light border of every glass, and the light line along its top edge.
pub const EDGE: Color32 = Color32::from_rgba_premultiplied(41, 41, 41, 41);
pub const SHINE: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26);
/// The two ends of the gradient of what is chosen or pressed: the rail, the main buttons.
pub const ACCENT_FROM: Color32 = Color32::from_rgb(0x6d, 0x72, 0xff);
pub const ACCENT_TO: Color32 = Color32::from_rgb(0xa8, 0x55, 0xf7);
/// The color of the window under the wallpaper.
const NIGHT: Color32 = Color32::from_rgb(0x0b, 0x0a, 0x1c);
/// How much backdrop-filter `saturate(170%)` adds to the colors behind.
const SATURATION: f32 = 1.7;

fn look_id() -> Id {
    Id::unique("glass-look")
}

fn render_id() -> Id {
    Id::unique("glass-render-state")
}

fn requests_id() -> Id {
    Id::unique("glass-requests")
}

fn sharp_id() -> Id {
    Id::unique("glass-sharp")
}

/// Draws every glass without its blur, as a control for the check that the blur works: the
/// same picture, with what is behind each glass sharp.
pub fn set_sharp(context: &Context, sharp: bool) {
    context.data_mut(|data| data.insert_temp(sharp_id(), sharp));
}

/// Gives the blur the GPU the window draws with. Without it the glass is only tinted.
pub fn install(context: &Context, render: RenderState) {
    context.data_mut(|data| data.insert_temp(render_id(), render));
}

/// The knobs of the theme, or None when another theme is on.
pub fn set_look(context: &Context, look: Option<GlassLook>) {
    context.data_mut(|data| {
        if let Some(look) = look {
            data.insert_temp(look_id(), look);
        } else {
            data.remove::<GlassLook>(look_id());
        }
    });
}

pub fn look(context: &Context) -> Option<GlassLook> {
    context.data(|data| data.get_temp::<GlassLook>(look_id()))
}

/// The wallpaper: four colored lights over a dark gradient, as `body::before` of the sketch.
/// `light` below 1 fades it into the night color, above 1 brightens it.
pub fn wallpaper(painter: &Painter, rect: Rect, look: &GlassLook) {
    painter.rect_filled(rect, CornerRadius::ZERO, NIGHT);
    let bright = look.light.max(1.0);
    let lift = |color: Color32| {
        let up = |value: u8| (f32::from(value) * bright).round().min(255.0) as u8;
        Color32::from_rgba_premultiplied(up(color.r()), up(color.g()), up(color.b()), color.a())
    };
    let base = crate::appearance::Gradient {
        light: false,
        colors: [0x1b1446, 0x10183a, 0x2a0f3a].iter().map(|hex| lift(Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, *hex as u8))).collect(),
        angle: 135.0,
        intensity: 100.0,
        motion: false,
    };
    base.paint(painter, rect, 0.0);
    // The lights, from the bottom one up: in CSS the first background is on top.
    let unit = look.light_size * rect.width() / 100.0;
    let lights: [(f32, f32, f32, u32, f32); 4] = [
        (0.15, 1.0, 55.0, 0x2563eb, 0.8),
        (0.85, 1.0, 60.0, 0xdb2777, 0.75),
        (0.95, 0.0, 50.0, 0x06b6d4, 0.7),
        (0.08, 0.1, 60.0, 0x7c3aed, 0.85),
    ];
    for (x, y, size, hex, alpha) in lights {
        let center = rect.min + Vec2::new(rect.width() * x, rect.height() * y);
        // `transparent 70%`: the color fades out at 70% of the radius.
        let reach = size * unit * 0.7;
        let color = lift(Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)).gamma_multiply(alpha);
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(center, color);
        const STEPS: u32 = 72;
        for step in 0..=STEPS {
            mesh.colored_vertex(center + Vec2::angled(std::f32::consts::TAU * step as f32 / STEPS as f32) * reach, Color32::TRANSPARENT);
        }
        for step in 1..=STEPS {
            mesh.add_triangle(0, step, step + 1);
        }
        painter.add(Shape::mesh(mesh));
    }
    // `opacity: min(1, light)`: the night color shows through the rest.
    let fade = 1.0 - look.light.clamp(0.0, 1.0);
    if fade > 0.0 {
        painter.rect_filled(rect, CornerRadius::ZERO, NIGHT.gamma_multiply(fade));
    }
}

/// How a glass surface looks besides the blur.
#[derive(Debug, Clone, Copy)]
pub struct Pane {
    pub tint: Color32,
    pub corners: CornerRadius,
    /// The blur in points, as CSS `blur()`. None takes the one of the theme.
    pub blur: Option<f32>,
    pub shadow: Option<egui::Shadow>,
    /// The light border.
    pub edge: bool,
    /// The light line along the top edge, as `inset 0 1px 0`.
    pub shine: bool,
    /// The white haze and the grain of the frost knob.
    pub frost: bool,
}

impl Pane {
    pub fn new(tint: Color32, radius: f32) -> Self {
        Self { tint, corners: CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8), blur: None, shadow: None, edge: true, shine: true, frost: false }
    }

    /// The large panels: the shadow under them and the frost on them, as `.sidebar, .main`.
    pub fn panel(tint: Color32) -> Self {
        Self { shadow: Some(egui::Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(89) }), frost: true, ..Self::new(tint, 16.0) }
    }
}

/// Room kept in a painter for a glass whose size is known only after what goes on it is laid
/// out: [`reserve`] before the contents, [`fill`] after.
#[derive(Debug, Clone, Copy)]
pub struct Reserved {
    behind: usize,
    first: usize,
}

/// The shadow, the blur, the tint, the haze, the grain, the border and the shine.
const SLOTS: usize = 7;

pub fn reserve(painter: &Painter) -> Reserved {
    let behind = painter.ctx().graphics(|layers| layers.get(painter.layer_id()).map_or(0, |list| list.next_idx().0));
    let first = painter.add(Shape::Noop).0;
    for _ in 1..SLOTS {
        painter.add(Shape::Noop);
    }
    Reserved { behind, first }
}

/// Paints a glass surface where [`reserve`] kept room, under what was painted since.
pub fn fill(painter: &Painter, reserved: Reserved, rect: Rect, pane: &Pane) {
    let look = look(painter.ctx()).unwrap_or_default();
    let radius = pane.corners;
    let set = |offset: usize, shape: Shape| painter.set(egui::layers::ShapeIdx(reserved.first + offset), shape);
    if let Some(shadow) = pane.shadow {
        set(0, shadow.as_shape(rect, radius).into());
    }
    request(painter, reserved.first + 1, rect, radius, pane.blur.unwrap_or(look.blur), reserved.behind);
    set(2, Shape::rect_filled(rect, radius, pane.tint));
    if pane.frost && look.frost > 0.0 {
        set(3, Shape::rect_filled(rect, radius, Color32::from_white_alpha((look.frost * 0.12 * 255.0).round() as u8)));
        set(4, grain(painter, rect, f32::from(radius.nw.max(radius.se)), look.frost * 0.35));
    }
    if pane.edge {
        set(5, Shape::rect_stroke(rect, radius, Stroke::new(1.0, EDGE), egui::StrokeKind::Inside));
    }
    if pane.shine {
        let inset = f32::from(radius.nw.max(radius.ne)).max(2.0);
        set(6, Shape::hline((rect.left() + inset)..=(rect.right() - inset), rect.top() + 1.5, Stroke::new(1.0, SHINE)));
    }
}

/// Paints a glass surface: the shadow, the blur of what is behind, the tint, the frost and the
/// edge. What is painted after it in the same painter goes on top.
pub fn pane(painter: &Painter, rect: Rect, pane: &Pane) {
    let reserved = reserve(painter);
    fill(painter, reserved, rect, pane);
}

/// A `.card` of the glass theme: white at 6% over a blur of 12, rounder, with the light edge.
pub(crate) fn glass_card(painter: &egui::Painter, rect: egui::Rect, hovered: bool, palette: &crate::theme::Palette) {
    let tint = egui::Color32::from_white_alpha(if hovered { 26 } else { 15 });
    let pane = crate::glass::Pane { blur: Some(12.0), shine: false, edge: false, ..crate::glass::Pane::new(tint, 14.0) };
    crate::glass::pane(painter, rect, &pane);
    let edge = if hovered { palette.accent } else { crate::glass::EDGE };
    painter.rect_stroke(rect, egui::CornerRadius::same(14), egui::Stroke::new(1.0, edge), egui::StrokeKind::Inside);
}

/// A modal window. On glass it is a glass panel and what is behind it goes dark and soft, as
/// `.overlay::before { background: rgba(5, 6, 14, .35); backdrop-filter: blur(6px) }`;
/// otherwise it is `frame` over a dark backdrop, as always.
pub fn modal<R>(context: &Context, id: Id, frame: egui::Frame, contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::ModalResponse<R> {
    let Some(look) = look(context) else {
        return egui::Modal::new(id).frame(frame).backdrop_color(Color32::from_black_alpha(178)).show(context, contents);
    };
    let margin = frame.inner_margin;
    egui::Modal::new(id).frame(egui::Frame::new().inner_margin(margin)).backdrop_color(Color32::TRANSPARENT).show(context, |ui| {
        let (veil, pane) = (reserve(ui.painter()), reserve(ui.painter()));
        let inner = contents(ui);
        let content = ui.min_rect();
        let rect = Rect::from_min_max(
            Pos2::new(content.left() - f32::from(margin.left), content.top() - f32::from(margin.top)),
            Pos2::new(content.right() + f32::from(margin.right), content.bottom() + f32::from(margin.bottom)),
        );
        let shade = Pane { blur: Some(6.0), edge: false, shine: false, ..Pane::new(Color32::from_rgba_unmultiplied(5, 6, 14, 89), 0.0) };
        fill(ui.painter(), veil, context.content_rect(), &shade);
        fill(ui.painter(), pane, rect, &Pane::panel(look.main()));
        inner
    })
}

/// The grain of the frost: white noise that repeats, faded to `amount`.
fn grain(painter: &Painter, rect: Rect, radius: f32, amount: f32) -> Shape {
    let context = painter.ctx();
    let id = Id::unique("glass-grain");
    let texture = context.data(|data| data.get_temp::<egui::TextureHandle>(id)).unwrap_or_else(|| {
        const SIDE: usize = 180;
        let mut seed: u32 = 0x9e37_79b9;
        let pixels = (0..SIDE * SIDE)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                Color32::from_white_alpha((seed % 128) as u8)
            })
            .collect();
        let image = egui::ColorImage::new([SIDE, SIDE], pixels);
        let options = egui::TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..egui::TextureOptions::LINEAR };
        let handle = context.load_texture("glass-grain", image, options);
        context.data_mut(|data| data.insert_temp(id, handle.clone()));
        handle
    });
    // Kept inside the rounded corners by drawing it a little smaller than the glass.
    let area = rect.shrink(radius * 0.3);
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(area.width() / 180.0, area.height() / 180.0));
    let mut mesh = egui::Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(area, uv, Color32::WHITE.gamma_multiply(amount));
    Shape::mesh(mesh)
}

/// Fills a shape with the accent gradient at 135 degrees, keeping its smooth edges.
pub fn accent_fill(painter: &Painter, shape: Shape) {
    let context = painter.ctx();
    let rect = shape.visual_bounding_rect();
    let clipped = ClippedShape::new(painter.clip_rect(), shape);
    let along = Vec2::new(1.0, 1.0).normalized();
    let span = (rect.width() + rect.height()) * std::f32::consts::FRAC_1_SQRT_2;
    for primitive in context.tessellate(vec![clipped], context.pixels_per_point()) {
        let Primitive::Mesh(mut mesh) = primitive.primitive else { continue };
        for vertex in &mut mesh.vertices {
            let place = if span > 0.0 { ((vertex.pos - rect.min).dot(along) / span).clamp(0.0, 1.0) } else { 0.0 };
            let color = lerp(ACCENT_FROM, ACCENT_TO, place);
            vertex.color = color.gamma_multiply(f32::from(vertex.color.a()) / 255.0);
        }
        painter.add(Shape::mesh(mesh));
    }
}

fn lerp(from: Color32, to: Color32, amount: f32) -> Color32 {
    let part = |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8;
    Color32::from_rgb(part(from.r(), to.r()), part(from.g(), to.g()), part(from.b(), to.b()))
}

#[derive(Clone, Default)]
struct Requests(Vec<Request>);

#[derive(Clone)]
struct Request {
    layer: LayerId,
    index: usize,
    clip: Rect,
    /// How many shapes of its own layer are behind it: the shadow of the glass is not.
    behind: usize,
    rect: Rect,
    corners: CornerRadius,
    blur: f32,
}

/// Blurs what is behind `rect`, inside the rounded corners, at the placeholder `index`.
/// `behind` is how many shapes of the painter's layer count as behind it.
fn request(painter: &Painter, index: usize, rect: Rect, corners: CornerRadius, blur: f32, behind: usize) {
    let request = Request { layer: painter.layer_id(), index, clip: painter.clip_rect(), behind: behind.min(index), rect, corners, blur };
    painter.ctx().data_mut(|data| data.get_temp_mut_or_default::<Requests>(requests_id()).0.push(request));
}

/// The layers in the order they are painted, the bottom one first.
fn layer_order(context: &Context, extra: &[LayerId]) -> Vec<LayerId> {
    let mut layers: Vec<LayerId> = vec![LayerId::background()];
    context.memory(|memory| {
        for layer in memory.layer_ids() {
            if !layers.contains(&layer) {
                layers.push(layer);
            }
        }
    });
    for layer in extra {
        if !layers.contains(layer) {
            layers.push(*layer);
        }
    }
    // Stable: within an order, the order of the areas stays.
    layers.sort_by_key(|layer| layer.order as u8);
    layers
}

/// Turns the placeholders of this frame into blurs. Called once, after everything is painted.
pub fn finish(context: &Context) {
    let requests = context.data_mut(|data| std::mem::take(&mut data.get_temp_mut_or_default::<Requests>(requests_id()).0));
    if requests.is_empty() {
        return;
    }
    let Some(render) = context.data(|data| data.get_temp::<RenderState>(render_id())) else { return };
    let pixels_per_point = context.pixels_per_point();
    let screen = context.content_rect();
    let order = layer_order(context, &requests.iter().map(|request| request.layer).collect::<Vec<_>>());
    let pass = context.cumulative_pass_nr();
    let sharp = context.data(|data| data.get_temp::<bool>(sharp_id())).unwrap_or(false);
    for (slot, request) in requests.into_iter().enumerate() {
        let region = request.rect.intersect(screen);
        if region.width() < 1.0 || region.height() < 1.0 {
            continue;
        }
        let shift = -region.min.to_vec2();
        let local = Rect::from_min_size(Pos2::ZERO, region.size());
        let shapes = context.graphics(|layers| {
            let mut shapes = Vec::new();
            for layer in &order {
                let Some(list) = layers.get(*layer) else {
                    if *layer == request.layer {
                        break;
                    }
                    continue;
                };
                let count = if *layer == request.layer { request.behind } else { list.all_entries().len() };
                for entry in list.all_entries().take(count) {
                    if matches!(entry.shape, Shape::Callback(_) | Shape::Noop) {
                        continue;
                    }
                    if !entry.clip_rect.intersects(region) || !entry.shape.visual_bounding_rect().intersects(region) {
                        continue;
                    }
                    shapes.push(ClippedShape {
                        clip_rect: entry.clip_rect.translate(shift).intersect(local),
                        shape: entry.shape.clone(),
                        transform_after_tessellation: egui::emath::TSTransform::from_translation(shift) * entry.transform_after_tessellation,
                    });
                }
                // Layers above the glass's own are in front of it.
                if *layer == request.layer {
                    break;
                }
            }
            shapes
        });
        let mut primitives = context.tessellate(shapes, pixels_per_point);
        let mut textures: Vec<(TextureId, wgpu::Texture)> = Vec::new();
        let mut missing = false;
        {
            let renderer = render.renderer.read();
            primitives.retain(|primitive| {
                let Primitive::Mesh(mesh) = &primitive.primitive else { return false };
                if textures.iter().any(|(id, _)| *id == mesh.texture_id) {
                    return true;
                }
                match renderer.texture(&mesh.texture_id).and_then(|texture| texture.texture.clone()) {
                    Some(texture) => {
                        textures.push((mesh.texture_id, texture));
                        true
                    }
                    None => {
                        missing = true;
                        false
                    }
                }
            });
        }
        if missing {
            // A picture or the letters were not on the GPU yet: the next frame has them.
            context.request_repaint();
        }
        let blur_px = if sharp { 0.0 } else { request.blur.max(0.0) * pixels_per_point };
        // Drawn smaller the stronger the blur: a blur of 4 pixels there is as smooth.
        let scale = if blur_px > 4.0 { 4.0 / blur_px } else { 1.0 };
        let callback = Blur {
            slot,
            pass,
            format: render.target_format,
            primitives,
            textures,
            region,
            rect: request.rect,
            corners: request.corners,
            sigma: blur_px * scale,
            scale,
            pixels_per_point,
        };
        let shape = Shape::Callback(egui_wgpu::Callback::new_paint_callback(request.rect, callback));
        context.graphics_mut(|layers| layers.entry(request.layer).set(egui::layers::ShapeIdx(request.index), request.clip, shape));
    }
}

struct Blur {
    slot: usize,
    pass: u64,
    format: wgpu::TextureFormat,
    primitives: Vec<ClippedPrimitive>,
    textures: Vec<(TextureId, wgpu::Texture)>,
    /// What is drawn again, in points on the screen.
    region: Rect,
    rect: Rect,
    corners: CornerRadius,
    /// The blur in pixels of the small texture.
    sigma: f32,
    scale: f32,
    pixels_per_point: f32,
}

const COMMON: &str = r"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var filtered: sampler;
";

const BLUR_SHADER: &str = r"
struct Blur { step: vec2<f32>, sigma: f32, reach: f32 };
@group(0) @binding(2) var<uniform> blur: Blur;

struct Full { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> Full {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Full;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_blur(input: Full) -> @location(0) vec4<f32> {
    var sum = textureSampleLevel(source, filtered, input.uv, 0.0);
    var total = 1.0;
    let reach = i32(blur.reach);
    for (var k = 1; k <= reach; k++) {
        let weight = exp(-f32(k * k) / (2.0 * blur.sigma * blur.sigma));
        let offset = blur.step * f32(k);
        sum += (textureSampleLevel(source, filtered, input.uv + offset, 0.0) + textureSampleLevel(source, filtered, input.uv - offset, 0.0)) * weight;
        total += 2.0 * weight;
    }
    return sum / total;
}
";

const GLASS_SHADER: &str = r"
struct Glass { rect: vec4<f32>, region: vec4<f32>, corners: vec4<f32>, screen: vec2<f32>, ppp: f32, saturation: f32, linear_out: f32, pad0: f32, pad1: f32, pad2: f32 };
@group(0) @binding(2) var<uniform> glass: Glass;

@vertex
fn vs_glass(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let point = (glass.rect.xy + corner * glass.rect.zw) * glass.ppp;
    let ndc = point / glass.screen * 2.0 - 1.0;
    return vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
}

fn to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

@fragment
fn fs_glass(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let point = position.xy / glass.ppp;
    let uv = (point - glass.region.xy) / glass.region.zw;
    var color = textureSampleLevel(source, filtered, uv, 0.0).rgb;
    let gray = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    color = clamp(mix(vec3<f32>(gray), color, glass.saturation), vec3<f32>(0.0), vec3<f32>(1.0));
    let half_size = glass.rect.zw * 0.5;
    let relative = point - (glass.rect.xy + half_size);
    // The radius of the corner the point is nearest to: top left, top right, bottom right, bottom left.
    let top = select(glass.corners.x, glass.corners.y, relative.x > 0.0);
    let bottom = select(glass.corners.w, glass.corners.z, relative.x > 0.0);
    let radius = select(top, bottom, relative.y > 0.0);
    let q = abs(relative) - half_size + vec2<f32>(radius);
    let outside = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    let alpha = clamp(0.5 - outside * glass.ppp, 0.0, 1.0);
    if (glass.linear_out > 0.5) {
        color = to_linear(color);
    }
    return vec4<f32>(color * alpha, alpha);
}
";

/// The small textures of one glass, and the egui renderer that draws what is behind it.
struct Slot {
    renderer: egui_wgpu::Renderer,
    /// The textures of the window, as this renderer knows them.
    textures: HashMap<TextureId, (TextureId, wgpu::Texture)>,
    size: [u32; 2],
    views: Option<(wgpu::TextureView, wgpu::TextureView)>,
    groups: Option<[wgpu::BindGroup; 3]>,
    uniforms: [wgpu::Buffer; 3],
    pass: u64,
}

struct Resources {
    blur: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    slots: HashMap<usize, Slot>,
}

const SMALL: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Resources {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = |code: &str| device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("glass"), source: wgpu::ShaderSource::Wgsl(format!("{COMMON}{code}").into()) });
        let (blur_module, glass_module) = (module(BLUR_SHADER), module(GLASS_SHADER));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { multisampled: false, sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2 },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("glass"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = |module: &wgpu::ShaderModule, vertex: &str, fragment: &str, format: wgpu::TextureFormat, topology: wgpu::PrimitiveTopology, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("glass"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module, entry_point: Some(vertex), buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::OneMinusDstAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let blur = pipeline(&blur_module, "vs_full", "fs_blur", SMALL, wgpu::PrimitiveTopology::TriangleList, None);
        let glass = pipeline(&glass_module, "vs_glass", "fs_glass", format, wgpu::PrimitiveTopology::TriangleStrip, Some(premultiplied));
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glass"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { blur, glass, layout, sampler, slots: HashMap::new() }
    }
}

fn bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_ne_bytes()).collect()
}

impl CallbackTrait for Blur {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<Resources>().is_none() {
            resources.insert(Resources::new(device, self.format));
        }
        let Some(resources) = resources.get_mut::<Resources>() else { return Vec::new() };
        // Glass that was not drawn for a while lets its textures go.
        resources.slots.retain(|_, slot| self.pass.saturating_sub(slot.pass) < 120);
        let size = [
            ((self.region.width() * self.pixels_per_point * self.scale).round() as u32).max(1),
            ((self.region.height() * self.pixels_per_point * self.scale).round() as u32).max(1),
        ];
        let Resources { blur, layout, sampler, slots, .. } = resources;
        let slot = slots.entry(self.slot).or_insert_with(|| {
            let uniform = || device.create_buffer(&wgpu::BufferDescriptor { label: Some("glass"), size: 80, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            Slot {
                renderer: egui_wgpu::Renderer::new(device, SMALL, egui_wgpu::RendererOptions { dithering: false, ..Default::default() }),
                textures: HashMap::new(),
                size: [0, 0],
                views: None,
                groups: None,
                uniforms: [uniform(), uniform(), uniform()],
                pass: 0,
            }
        });
        slot.pass = self.pass;
        if slot.size != size || slot.views.is_none() {
            let texture = || {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("glass"),
                        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: SMALL,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            let (first, second) = (texture(), texture());
            let group = |view: &wgpu::TextureView, uniform: &wgpu::Buffer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("glass"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                        wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
                    ],
                })
            };
            // Across from the first texture into the second, down back into the first, and the
            // first drawn on the window.
            slot.groups = Some([group(&first, &slot.uniforms[0]), group(&second, &slot.uniforms[1]), group(&first, &slot.uniforms[2])]);
            slot.views = Some((first, second));
            slot.size = size;
        }
        // The textures of the window, given to the renderer of this glass under its own ids.
        let mut map = HashMap::new();
        for (id, texture) in &self.textures {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let own = match slot.textures.get(id) {
                Some((own, known)) if known == texture => *own,
                Some((own, _)) => {
                    slot.renderer.update_egui_texture_from_wgpu_texture(device, &view, wgpu::FilterMode::Linear, *own);
                    *own
                }
                None => slot.renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear),
            };
            slot.textures.insert(*id, (own, texture.clone()));
            map.insert(*id, own);
        }
        let primitives: Vec<ClippedPrimitive> = self
            .primitives
            .iter()
            .filter_map(|primitive| {
                let Primitive::Mesh(mesh) = &primitive.primitive else { return None };
                let mut mesh = mesh.clone();
                mesh.texture_id = *map.get(&mesh.texture_id)?;
                Some(ClippedPrimitive { clip_rect: primitive.clip_rect, primitive: Primitive::Mesh(mesh) })
            })
            .collect();
        let small = ScreenDescriptor { size_in_pixels: size, pixels_per_point: self.pixels_per_point * self.scale };
        let mut commands = slot.renderer.update_buffers(device, queue, encoder, &primitives, &small);
        let (Some((first, second)), Some(groups)) = (&slot.views, &slot.groups) else { return commands };
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("glass-behind"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: first,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            slot.renderer.render(&mut pass, &primitives, &small);
        }
        let reach = (self.sigma * 3.0).ceil().min(32.0);
        queue.write_buffer(&slot.uniforms[0], 0, &bytes(&[1.0 / size[0] as f32, 0.0, self.sigma.max(0.01), reach]));
        queue.write_buffer(&slot.uniforms[1], 0, &bytes(&[0.0, 1.0 / size[1] as f32, self.sigma.max(0.01), reach]));
        let linear = if self.format.is_srgb() { 1.0 } else { 0.0 };
        let (rect, region) = (self.rect, self.region);
        queue.write_buffer(
            &slot.uniforms[2],
            0,
            &bytes(&[
                rect.min.x, rect.min.y, rect.width(), rect.height(),
                region.min.x, region.min.y, region.width(), region.height(),
                f32::from(self.corners.nw), f32::from(self.corners.ne), f32::from(self.corners.se), f32::from(self.corners.sw),
                screen.size_in_pixels[0] as f32, screen.size_in_pixels[1] as f32, self.pixels_per_point, SATURATION,
                linear, 0.0, 0.0, 0.0,
            ]),
        );
        for (target, group) in [(second, &groups[0]), (first, &groups[1])] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glass-blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
                ..Default::default()
            });
            pass.set_pipeline(blur);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        commands.shrink_to_fit();
        commands
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        let Some(resources) = resources.get::<Resources>() else { return };
        let Some(slot) = resources.slots.get(&self.slot) else { return };
        let Some(groups) = &slot.groups else { return };
        let [width, height] = info.screen_size_px;
        pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
        pass.set_pipeline(&resources.glass);
        pass.set_bind_group(0, &groups[2], &[]);
        pass.draw(0..4, 0..1);
    }
}
