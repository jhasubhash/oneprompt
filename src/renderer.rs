//! GPU side, the framework every theme draws through (`theme` has the API).
//! Each frame is built in three passes, each running the current theme's
//! stage (its own shader, or the framework's plain default):
//! 1. Ink pass, into an offscreen "ink layer": everything drawn *with ink*
//!    (text via glyphon, underlines, box drawing, the window controls, the
//!    cursor and struck-out names, which are shapes).
//! 2. Main pass, into the offscreen scene: the surface stage, cell
//!    backgrounds / selection, then the ink layer composited by the ink
//!    stage, which decides how ink behaves on the surface and writes new
//!    characters in.
//! 3. Present pass, onto the window: copies the scene, or while a reveal or
//!    switch runs, the incoming theme's transition stage shows it over what
//!    was there before.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Indexed;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};
use glyphon::{
    fontdb, Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping,
    Style, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};
use winit::window::Window;

use crate::animation::{self, TextAnimator, Transition, Writing};
use crate::boxdraw;
use crate::color;
use crate::trace::{self, GlyphKey, Slot, Tracer};
use crate::chrome::{self, Control};
use crate::picker;
use crate::strike::{Strikes, Struck};
use crate::terminal::GridSize;
use crate::theme::{
    self as api, ControlStyle, Ctx, Cursor, CursorKind, Shape, StrikeSpan, Theme, Variant, CROSS, DISC, RING,
};

/// Margin around the grid, in logical pixels.
const PADDING: f32 = 24.0;
/// The grid full-screen apps (btop, vim, ...) get at least: the classic
/// terminal size most of them are designed for. While one runs in a window
/// too small for it at the chosen font size, the text shrinks to fit.
const FULL_SCREEN_MIN: (usize, usize) = (80, 24);
/// Text shrinks to no less than this share of its size.
const MIN_FIT: f32 = 0.5;
/// Height of the drag area at the top of the window, in logical pixels. The
/// window has no visible title bar; this strip is just more of the surface.
pub const TITLE_STRIP: f32 = 30.0;
/// Opacity of coloured cell backgrounds, so the texture still shows through.
const CELL_BG_ALPHA: f32 = 0.85;
/// Opacity of a program's own background colour, laid on as a thin wash so
/// the paper (texture, stains, vignette) shows through it.
const WASH_ALPHA_LIGHT: f32 = 0.13;
const WASH_ALPHA_DARK: f32 = 0.1;

/// What every stage of every theme sees (prelude.wgsl, `Frame`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Frame {
    resolution: [f32; 2],
    scale: f32,
    seed: f32,
    time: f32,
    speed: f32,
    progress: f32,
    _pad0: f32,
    from_color: [f32; 4],
    grid_origin: [f32; 2],
    cell: [f32; 2],
    grid_size: [f32; 2],
    slot_size: [f32; 2],
    write_time: f32,
    atlas_columns: f32,
    _pad1: [f32; 2],
    surface: [f32; 4],
    ink: [f32; 4],
    accent: [f32; 4],
}

/// The framework's shader parts (see `theme` for how stages are put together).
const PRELUDE: &str = include_str!("shaders/prelude.wgsl");
const SURFACE_STAGE: &str = include_str!("shaders/surface_stage.wgsl");
const SHAPES_STAGE: &str = include_str!("shaders/shapes_stage.wgsl");
const INK_STAGE: &str = include_str!("shaders/ink_stage.wgsl");
const TRANSITION_STAGE: &str = include_str!("shaders/transition_stage.wgsl");
const SURFACE_DEFAULT: &str = include_str!("shaders/surface_default.wgsl");
const SHAPES_DEFAULT: &str = include_str!("shaders/shapes_default.wgsl");
const INK_DEFAULT: &str = include_str!("shaders/ink_default.wgsl");
const TRANSITION_DEFAULT: &str = include_str!("shaders/transition_default.wgsl");

/// One stage's full source: the prelude, the theme's common part, the
/// framework's part, and the theme's function (or the default).
fn stage_source(theme: &dyn Theme, stage: &str, own: Option<&str>, default: &str) -> String {
    format!("{PRELUDE}\n{}\n{stage}\n{}", theme.shaders().common, own.unwrap_or(default))
}

/// Every stage's source for `theme`: surface, shapes, ink, transition.
fn stage_sources(theme: &dyn Theme) -> [String; 4] {
    let own = theme.shaders();
    [
        stage_source(theme, SURFACE_STAGE, own.surface, SURFACE_DEFAULT),
        stage_source(theme, SHAPES_STAGE, own.shapes, SHAPES_DEFAULT),
        stage_source(theme, INK_STAGE, own.ink, INK_DEFAULT),
        stage_source(theme, TRANSITION_STAGE, own.transition, TRANSITION_DEFAULT),
    ]
}

/// The bind group layouts stages are built against, and their pipeline
/// layouts. Binding 0 is the frame, 1 the theme's data, in every stage.
struct Layouts {
    /// Surface and shapes: frame and data.
    frame: wgpu::BindGroupLayout,
    /// Ink: also the ink layer, its sampler, births and the pen-order atlas.
    ink: wgpu::BindGroupLayout,
    /// Transition: also the plan, the scene and the previous frame.
    transition: wgpu::BindGroupLayout,
    frame_pipeline: wgpu::PipelineLayout,
    ink_pipeline: wgpu::PipelineLayout,
    transition_pipeline: wgpu::PipelineLayout,
}

impl Layouts {
    fn new(device: &wgpu::Device) -> Self {
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
        };
        let frame = layout("frame layout", &[uniform(0), uniform(1)]);
        let ink = layout(
            "ink layout",
            &[
                uniform(0),
                uniform(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                Renderer::plain_texture_entry(4),
                Renderer::plain_texture_entry(5),
            ],
        );
        let transition = layout(
            "transition layout",
            &[uniform(0), uniform(1), uniform(2), Renderer::plain_texture_entry(3), Renderer::plain_texture_entry(4)],
        );
        let pipeline = |label, group: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(group)],
                immediate_size: 0,
            })
        };
        Self {
            frame_pipeline: pipeline("frame pipeline layout", &frame),
            ink_pipeline: pipeline("ink pipeline layout", &ink),
            transition_pipeline: pipeline("transition pipeline layout", &transition),
            frame,
            ink,
            transition,
        }
    }
}

/// A theme's pipelines, one per stage, built the first time it is drawn.
struct Stages {
    surface: wgpu::RenderPipeline,
    shapes: wgpu::RenderPipeline,
    ink: wgpu::RenderPipeline,
    transition: wgpu::RenderPipeline,
}

impl Stages {
    fn new(device: &wgpu::Device, layouts: &Layouts, format: wgpu::TextureFormat, theme: &dyn Theme) -> Self {
        let [surface, shapes, ink, transition] = stage_sources(theme);
        let build = |label: &str,
                     source: String,
                     layout: &wgpu::PipelineLayout,
                     blend: Option<wgpu::BlendState>,
                     buffers: &[Option<wgpu::VertexBufferLayout>],
                     topology| {
            let label = format!("{} {label}", theme.id());
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers,
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState { topology, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let list = wgpu::PrimitiveTopology::TriangleList;
        let instances = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Shape>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
        })];
        Self {
            surface: build("surface", surface, &layouts.frame_pipeline, None, &[], list),
            shapes: build(
                "shapes",
                shapes,
                &layouts.frame_pipeline,
                Some(wgpu::BlendState::ALPHA_BLENDING),
                &instances,
                wgpu::PrimitiveTopology::TriangleStrip,
            ),
            // The ink layer holds premultiplied colour.
            ink: build("ink", ink, &layouts.ink_pipeline, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), &[], list),
            transition: build("transition", transition, &layouts.transition_pipeline, None, &[], list),
        }
    }
}

/// Uniform buffers every stage reads: the frame, the theme's data, and the
/// running transition's plan.
struct Buffers {
    frame: wgpu::Buffer,
    data: wgpu::Buffer,
    plan: wgpu::Buffer,
}

impl Buffers {
    fn new(device: &wgpu::Device) -> Self {
        let uniform = |label, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        Self {
            frame: uniform("frame", std::mem::size_of::<Frame>()),
            data: uniform("theme data", api::DATA_SIZE),
            plan: uniform("transition plan", api::PLAN_SIZE),
        }
    }
}

/// Seconds to brush one character in, at animation speed 1.
const WRITE_TIME: f32 = 0.38;
/// Pen-order atlas, one cell-sized slot per traced glyph.
const TRACE_ATLAS: (u32, u32) = (2048, 2048);
/// Longest a batch of output takes to write, at speed 1.
const MAX_WRITE: f32 = 1.5;

/// Each further mention of one deleted name is struck this much later.
const STRIKE_STAGGER: f32 = 0.04;
/// Mentions beyond this many get no extras (blood, ...), so deleting a whole
/// tree does not drown the screen.
const MAX_STRIKE_EXTRAS: usize = 60;

/// What is installed for a font family.
#[derive(Clone, Debug)]
struct FamilyInfo {
    /// Weights of its upright faces, e.g. [400, 700] for Courier New.
    weights: Vec<u16>,
    monospaced: bool,
}

/// The installed weight closest to `wanted`. On a tie (550 between 400 and
/// 700) heavier wins above 500 and lighter below, as in CSS font matching.
fn snap_weight(weights: &[u16], wanted: u16) -> u16 {
    weights
        .iter()
        .copied()
        .min_by_key(|&w| {
            let tie_break = if wanted >= 500 { w < wanted } else { w > wanted };
            (w.abs_diff(wanted), tie_break)
        })
        .unwrap_or(wanted)
}

/// A short message shown in the margin below the text.
pub struct Notice {
    pub text: String,
    pub is_error: bool,
}

/// Everything sized by the window or the grid, rebuilt together.
struct Targets {
    /// Ink is drawn here before it is composited onto the surface.
    ink: wgpu::Texture,
    ink_view: wgpu::TextureView,
    ink_bind_group: wgpu::BindGroup,
    /// The finished frame, before the present pass copies it to the window.
    scene: wgpu::Texture,
    scene_view: wgpu::TextureView,
    /// The frame a light/dark transition reveals the new look over.
    previous: wgpu::Texture,
    present_bind_group: wgpu::BindGroup,
    /// Per cell: birth time and pen-order atlas slot (Rg32Float).
    births: wgpu::Texture,
    grid: (u32, u32),
}

impl Targets {
    #[allow(clippy::too_many_arguments)]
    fn new(
        device: &wgpu::Device,
        layouts: &Layouts,
        buffers: &Buffers,
        ink_sampler: &wgpu::Sampler,
        trace_atlas: &wgpu::TextureView,
        config: &wgpu::SurfaceConfiguration,
        grid: (u32, u32),
        keep: Option<&Targets>,
    ) -> Self {
        let size = wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 };
        let texture = |label, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: config.format,
                usage,
                view_formats: &[],
            })
        };
        use wgpu::TextureUsages as U;
        // Only the grid changed (another theme's font): keep the window-sized
        // textures, so a transition still has the previous frame.
        let (ink, scene, previous) = match keep.filter(|t| t.scene.width() == config.width && t.scene.height() == config.height) {
            Some(t) => (t.ink.clone(), t.scene.clone(), t.previous.clone()),
            None => (
                texture("ink layer", U::RENDER_ATTACHMENT | U::TEXTURE_BINDING),
                texture("scene", U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC),
                texture("previous frame", U::TEXTURE_BINDING | U::COPY_DST),
            ),
        };
        let births = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cell births"),
            size: wgpu::Extent3d { width: grid.0.max(1), height: grid.1.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg32Float,
            usage: U::TEXTURE_BINDING | U::COPY_DST,
            view_formats: &[],
        });
        let ink_view = ink.create_view(&Default::default());
        let scene_view = scene.create_view(&Default::default());
        let previous_view = previous.create_view(&Default::default());
        let births_view = births.create_view(&Default::default());
        let view = wgpu::BindingResource::TextureView;
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let ink_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ink stage"),
            layout: &layouts.ink,
            entries: &[
                entry(0, buffers.frame.as_entire_binding()),
                entry(1, buffers.data.as_entire_binding()),
                entry(2, view(&ink_view)),
                entry(3, wgpu::BindingResource::Sampler(ink_sampler)),
                entry(4, view(&births_view)),
                entry(5, view(trace_atlas)),
            ],
        });
        let present_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transition stage"),
            layout: &layouts.transition,
            entries: &[
                entry(0, buffers.frame.as_entire_binding()),
                entry(1, buffers.data.as_entire_binding()),
                entry(2, buffers.plan.as_entire_binding()),
                entry(3, view(&scene_view)),
                entry(4, view(&previous_view)),
            ],
        });
        Self { ink, ink_view, ink_bind_group, scene, scene_view, previous, present_bind_group, births, grid }
    }
}

/// The theme picker as drawn: the chosen card, and per theme the variant
/// its card previews and the picture of it.
struct PickerView {
    selected: usize,
    cards: Vec<PickerCard>,
    title: Buffer,
    hint: Buffer,
}

struct PickerCard {
    variant: Variant,
    /// The screen drawn in `variant`, at the window's size.
    picture: Option<(wgpu::Texture, wgpu::BindGroup)>,
    name: Buffer,
    about: Buffer,
}

/// The pipeline that draws pictures (picker previews) into boxes.
fn picker_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("picture layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("picture pipeline layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let source = format!("{PRELUDE}\n{}", include_str!("shaders/image.wgsl"));
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("picture"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("picture"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Shape>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
            })],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    (layout, pipeline)
}

pub struct Renderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    layouts: Layouts,
    buffers: Buffers,
    /// Frame and theme data, for the surface and shapes stages.
    frame_group: wgpu::BindGroup,
    /// Each theme's stages, by theme id, built the first time it is drawn.
    stages: HashMap<&'static str, Stages>,
    /// Varies the surface and ink patterns per session (see `session_seed`).
    seed: f32,

    rect_buffer: wgpu::Buffer,
    /// Drawn on the surface, under the ink: cell backgrounds, selection.
    rects: Vec<Shape>,
    /// Drawn into the ink layer: underlines, box drawing, window controls,
    /// the cursor, struck-out names.
    ink_rects: Vec<Shape>,
    ink_sampler: wgpu::Sampler,
    targets: Targets,

    /// Animation clock: births and transitions are measured on it.
    clock: Instant,
    /// Animations on, and their speed (1 = normal, 2 = twice as fast).
    animation: (bool, f32),
    /// How long the current theme's ink keeps changing after a character is
    /// written, seconds at speed 1 (for redraws).
    settle_time: f32,
    /// Characters per second when output is written out.
    writing_speed: f32,
    /// Map program-chosen colours onto the sheet (see color.rs).
    harmonize: bool,
    /// Until when a sword is still cutting through a deleted name.
    strikes_until: f32,
    /// Whether struck names fling blood.
    splatter: bool,
    animator: TextAnimator,
    transition: Option<Transition>,
    /// For themes' made-up-at-random reveals.
    rng: animation::Rng,
    /// Pen paths for the writing animation (see trace.rs).
    tracer: Tracer,
    /// Font the traced glyphs were rasterized in; a change starts over.
    traced_font: (String, u16),
    trace_atlas: wgpu::Texture,
    trace_atlas_view: wgpu::TextureView,

    /// Pointer is over the window controls: show their symbols.
    controls_hover: bool,
    /// The theme picker, while it is open.
    picker: Option<PickerView>,
    picker_pipeline: wgpu::RenderPipeline,
    picker_layout: wgpu::BindGroupLayout,
    picker_buffer: wgpu::Buffer,
    picker_text: TextRenderer,
    /// Drawing a picker preview rather than the live screen.
    thumbnail: bool,

    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    text: Buffer,
    /// What the font database has for each family name we have asked about.
    families: HashMap<String, Option<FamilyInfo>>,
    /// Characters the theme's font has, for the current family.
    coverage: (String, HashMap<char, bool>),
    /// Characters drawn on their own, one buffer each, keyed by text, weight
    /// and italic; valid for `loose_metrics`.
    loose_glyphs: HashMap<LooseKey, Buffer>,
    loose_metrics: (String, u32, u32),
    /// This frame's loose characters: cell corner, key and colour.
    loose: Vec<(f32, f32, LooseKey, [u8; 3])>,
    /// Short message in the title strip ("settings applied", config errors).
    notice: Option<Notice>,
    notice_text: Buffer,
    /// Tab labels for the title strip (empty with a single tab), the active
    /// one, a buffer per label, and where each sits: text corner and slot.
    tabs: Vec<String>,
    active_tab: usize,
    /// The tab whose name is being typed (drawn with a caret), and whether
    /// all of it is selected.
    editing_tab: Option<(usize, bool)>,
    tab_text: Vec<Buffer>,
    tab_slots: Vec<TabSlot>,

    /// Text size multiplier (Cmd + / Cmd -).
    zoom: f32,
    /// Whether a full-screen app is running, and the shrink applied for it.
    fit_full_screen: bool,
    fit: f32,
    cell_width: f32,
    cell_height: f32,
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .expect("no suitable GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");

        // Theme colours are authored in sRGB, so render to a non-sRGB target
        // and let the values pass through unchanged.
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        configure_surface(&surface, &device, &config);

        let layouts = Layouts::new(&device);
        let buffers = Buffers::new(&device);
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &layouts.frame,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffers.frame.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: buffers.data.as_entire_binding() },
            ],
        });
        let rect_buffer = Self::create_rect_buffer(&device, 1024);
        let ink_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ink sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let trace_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pen order atlas"),
            size: wgpu::Extent3d { width: TRACE_ATLAS.0, height: TRACE_ATLAS.1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let trace_atlas_view = trace_atlas.create_view(&Default::default());
        let targets = Targets::new(&device, &layouts, &buffers, &ink_sampler, &trace_atlas_view, &config, (1, 1), None);

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::with_color_mode(&device, &queue, &cache, format, ColorMode::Web);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let picker_text = TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let (picker_layout, picker_pipeline) = picker_pipeline(&device, format);
        let picker_buffer = Self::create_rect_buffer(&device, 64);
        let mut text = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
        text.borrow_with(&mut font_system).set_wrap(Wrap::None);
        let mut notice_text = Buffer::new(&mut font_system, Metrics::new(12.0, 16.0));
        notice_text.borrow_with(&mut font_system).set_wrap(Wrap::None);

        Self {
            window,
            surface,
            device,
            queue,
            config,
            layouts,
            buffers,
            frame_group,
            stages: HashMap::new(),
            seed: session_seed(),
            rect_buffer,
            rects: Vec::new(),
            ink_rects: Vec::new(),
            ink_sampler,
            targets,
            clock: Instant::now(),
            animation: (true, 1.0),
            settle_time: 0.0,
            writing_speed: 500.0,
            harmonize: true,
            strikes_until: f32::NEG_INFINITY,
            splatter: true,
            animator: TextAnimator::default(),
            transition: None,
            rng: animation::Rng::new(random_seed()),
            tracer: Tracer::new((1, 1), TRACE_ATLAS),
            traced_font: (String::new(), 0),
            trace_atlas,
            trace_atlas_view,
            controls_hover: false,
            picker: None,
            picker_pipeline,
            picker_layout,
            picker_buffer,
            picker_text,
            thumbnail: false,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text,
            families: HashMap::new(),
            coverage: (String::new(), HashMap::new()),
            loose_glyphs: HashMap::new(),
            loose_metrics: (String::new(), 0, 0),
            loose: Vec::new(),
            notice: None,
            notice_text,
            tabs: Vec::new(),
            active_tab: 0,
            editing_tab: None,
            tab_text: Vec::new(),
            tab_slots: Vec::new(),
            zoom: 1.0,
            fit_full_screen: false,
            fit: 1.0,
            cell_width: 8.0,
            cell_height: 16.0,
        }
    }

    fn create_rect_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rect instances"),
            size: (capacity * std::mem::size_of::<Shape>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// A texture read with `textureLoad` (no filtering) in a fragment shader.
    fn plain_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
        wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }
    }

    /// Rebuilds the window- and grid-sized targets.
    fn rebuild_targets(&mut self, grid: (u32, u32)) {
        self.targets = Targets::new(
            &self.device,
            &self.layouts,
            &self.buffers,
            &self.ink_sampler,
            &self.trace_atlas_view,
            &self.config,
            grid,
            Some(&self.targets),
        );
    }

    /// Starts the pen-path atlas over when the glyphs would come out
    /// different: another cell size (zoom, scale) or another font.
    fn reset_tracer_if_needed(&mut self, variant: &Variant) {
        let slot = (self.cell_width.ceil() as u32 + 2, self.cell_height.ceil() as u32);
        let font = (variant.font_family.clone(), variant.font_weight);
        if slot != self.tracer.slot_size || font != self.traced_font {
            self.tracer = Tracer::new(slot, TRACE_ATLAS);
            self.traced_font = font;
        }
    }

    /// Rasterizes `key` exactly as the terminal draws it and uploads its pen
    /// order to `slot`. Returns false if the glyph has no ink (a space).
    fn trace_glyph(&mut self, variant: &Variant, key: GlyphKey, slot: u32) -> bool {
        let (w, h) = self.tracer.slot_size;
        let metrics = self.metrics(variant);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let mut attrs = Attrs::new().family(Family::Name(&variant.font_family)).weight(Weight(key.weight));
        if key.italic {
            attrs = attrs.style(Style::Italic);
        }
        {
            let mut b = buffer.borrow_with(&mut self.font_system);
            b.set_wrap(Wrap::None);
            b.set_size(Some(w as f32 * 4.0), Some(h as f32 * 2.0));
            b.set_text(&key.c.to_string(), &attrs, Shaping::Advanced, None);
            b.shape_until_scroll(false);
        }
        // Same placement rule glyphon uses, relative to the cell's corner;
        // one pixel of margin on the left for glyphs that overhang.
        let mut alpha = vec![0u8; (w * h) as usize];
        let glyphs: Vec<_> = buffer
            .layout_runs()
            .flat_map(|run| {
                let line_y = run.line_y.round() as i32;
                run.glyphs.iter().map(move |g| {
                    let physical = g.physical((0.0, 0.0), 1.0);
                    (physical.cache_key, physical.x, line_y + physical.y)
                })
            })
            .collect();
        for (cache_key, gx0, gy0) in glyphs {
            {
                let Some(image) = self.swash_cache.get_image_uncached(&mut self.font_system, cache_key)
                else {
                    continue;
                };
                let p = image.placement;
                let x0 = gx0 + p.left + 1;
                let y0 = gy0 - p.top;
                let stride = match image.content {
                    glyphon::SwashContent::Mask => 1,
                    _ => 4,
                };
                for gy in 0..p.height as i32 {
                    for gx in 0..p.width as i32 {
                        let (x, y) = (x0 + gx, y0 + gy);
                        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                            continue;
                        }
                        let i = ((gy * p.width as i32 + gx) as usize) * stride;
                        let a = if stride == 1 { image.data[i] } else { image.data[i + 3] };
                        let o = &mut alpha[(y as u32 * w + x as u32) as usize];
                        *o = (*o).max(a);
                    }
                }
            }
        }
        let order = trace::pen_order(&alpha, w as usize, h as usize);
        if order.iter().all(|&v| v == 0) {
            return false;
        }
        let (ox, oy) = self.tracer.origin(slot);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.trace_atlas,
                mip_level: 0,
                origin: wgpu::Origin3d { x: ox, y: oy, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &order,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        true
    }

    fn now(&self) -> f32 {
        self.clock.elapsed().as_secs_f32()
    }

    pub fn set_splatter(&mut self, splatter: bool) {
        self.splatter = splatter;
    }

    pub fn set_harmonize(&mut self, harmonize: bool) {
        self.harmonize = harmonize;
    }

    pub fn set_animation(&mut self, enabled: bool, speed: f32, writing_speed: f32) {
        self.animation = (enabled, speed.clamp(0.1, 10.0));
        self.writing_speed = writing_speed;
    }

    fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    /// What theme hooks get to know about the window.
    fn ctx(&self) -> Ctx {
        Ctx {
            scale: self.window.scale_factor() as f32,
            cell: [self.cell_width, self.cell_height],
            speed: self.animation.1,
        }
    }

    /// Starts `plan` (at speed 1) over `from`.
    fn start(&mut self, plan: api::Plan, from: animation::From) {
        self.transition = Some(Transition {
            start: self.now(),
            duration: plan.duration / self.animation.1,
            ease: plan.ease,
            from,
            data: plan.data,
        });
    }

    /// Reveals the first frame over a solid colour (the window opening), the
    /// way `variant`'s theme opens.
    pub fn start_intro(&mut self, from: [u8; 3], variant: &Variant) {
        if !self.animation.0 {
            return;
        }
        let color = from.map(|c| c as f32 / 255.0);
        let aspect = self.aspect();
        if let Some(plan) = variant.theme.reveal(variant, &mut self.rng, aspect) {
            self.start(plan, animation::From::Color(color));
        }
    }

    /// Reveals the next frames over the last one shown, the way `variant`'s
    /// theme takes over (from its other variant, or from another theme).
    pub fn start_transition(&mut self, variant: &Variant) {
        if !self.animation.0 {
            return;
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(
            self.targets.scene.as_image_copy(),
            self.targets.previous.as_image_copy(),
            wgpu::Extent3d { width: self.config.width, height: self.config.height, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));
        let aspect = self.aspect();
        if let Some(plan) = variant.theme.switch(variant, &mut self.rng, aspect) {
            self.start(plan, animation::From::PreviousFrame);
        }
    }

    /// Whether something is still moving, so the app keeps redrawing.
    pub fn is_animating(&self) -> bool {
        let now = self.now();
        let write_time = WRITE_TIME / self.animation.1;
        // Ink is wet for a few write times; it may keep settling for longer.
        let settle_time = self.settle_time / self.animation.1;
        self.transition.as_ref().is_some_and(|t| !t.done(now))
            || now < self.strikes_until
            || (self.animation.0 && now - self.animator.latest_birth() < (write_time * 4.0).max(settle_time))
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        configure_surface(&self.surface, &self.device, &self.config);
        let grid = self.targets.grid;
        self.rebuild_targets(grid);
        // A resized window cannot reveal over a frame of the old size.
        if self.transition.as_ref().is_some_and(|t| t.from == animation::From::PreviousFrame) {
            self.transition = None;
        }
    }

    fn family_info(&mut self, family: &str) -> Option<FamilyInfo> {
        if let Some(info) = self.families.get(family) {
            return info.clone();
        }
        let faces: Vec<_> = self
            .font_system
            .db()
            .faces()
            .filter(|face| face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family)))
            .map(|face| (face.weight.0, face.style == fontdb::Style::Normal, face.monospaced))
            .collect();
        let info = (!faces.is_empty()).then(|| {
            let upright: Vec<u16> = faces.iter().filter(|f| f.1).map(|f| f.0).collect();
            let mut weights = if upright.is_empty() { faces.iter().map(|f| f.0).collect() } else { upright };
            weights.sort_unstable();
            weights.dedup();
            FamilyInfo { weights, monospaced: faces.iter().any(|f| f.2) }
        });
        self.families.insert(family.to_string(), info.clone());
        info
    }

    /// Whether the theme's own font has `c`. Characters it lacks come from
    /// fallback fonts whose advances differ from the cell width.
    fn covers(&mut self, family: &str, c: char) -> bool {
        if self.coverage.0 != family {
            self.coverage = (family.to_string(), HashMap::new());
        }
        if let Some(&known) = self.coverage.1.get(&c) {
            return known;
        }
        let face = self
            .font_system
            .db()
            .faces()
            .find(|face| face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family)))
            .map(|face| (face.id, face.weight));
        let has = face
            .and_then(|(id, weight)| self.font_system.get_font(id, weight))
            .is_some_and(|font| font.as_swash().charmap().map(c) != 0);
        self.coverage.1.insert(c, has);
        has
    }

    /// The weight to ask the text engine for. It only uses a font when the
    /// font has exactly the requested weight; otherwise it silently falls back
    /// to some other (often proportional) font and the grid stops lining up.
    /// So snap to a weight the family really has.
    fn weight_for(&mut self, family: &str, wanted: u16) -> u16 {
        match self.family_info(family) {
            Some(info) => snap_weight(&info.weights, wanted),
            None => wanted,
        }
    }

    /// Why a theme's font would not render as a proper grid, if it would not.
    pub fn font_problem(&mut self, family: &str) -> Option<String> {
        match self.family_info(family) {
            None => Some(format!("font \"{family}\" is not installed, using a fallback")),
            Some(info) if !info.monospaced => {
                Some(format!("font \"{family}\" is not monospace, columns will not line up"))
            }
            Some(_) => None,
        }
    }

    /// The tabs to show in the title strip; none when there is only one.
    pub fn set_tabs(&mut self, labels: Vec<String>, active: usize, editing: Option<(usize, bool)>) {
        self.tabs = labels;
        self.active_tab = active;
        self.editing_tab = editing;
    }

    /// The tab whose label is at `x` in the title strip, if any.
    pub fn tab_at(&self, x: f64) -> Option<usize> {
        let x = x as f32;
        self.tab_slots.iter().position(|slot| x >= slot.left && x < slot.right)
    }

    /// Forgets what the screen showed, so the next frame appears as it is
    /// instead of being written in (switching tabs).
    pub fn reset_writing(&mut self) {
        self.animator = TextAnimator::default();
    }

    pub fn set_notice(&mut self, notice: Option<Notice>) {
        self.notice = notice;
    }

    /// Changes the paper pattern; `None` keeps the current sheet.
    pub fn set_seed(&mut self, seed: Option<f32>) {
        if let Some(seed) = seed {
            self.seed = seed;
        }
    }

    /// A fresh sheet of paper: a new pattern of fibers, stains and splatter.
    pub fn new_sheet(&mut self) {
        self.seed = self.rng.range(0.0, 1000.0);
    }

    pub fn set_controls_hover(&mut self, hover: bool) -> bool {
        let changed = self.controls_hover != hover;
        self.controls_hover = hover;
        changed
    }

    pub fn cell_height(&self) -> f32 {
        self.cell_height
    }

    /// Maps a window position (physical pixels) to a viewport cell, clamped
    /// to the grid. Also says whether the point is in the cell's right half.
    pub fn cell_at(&self, x: f64, y: f64, size: GridSize) -> (usize, usize, bool) {
        let (pad, top) = self.padding();
        let fx = ((x as f32 - pad) / self.cell_width).max(0.0);
        let fy = ((y as f32 - top) / self.cell_height).max(0.0);
        let col = (fx as usize).min(size.columns - 1);
        let line = (fy as usize).min(size.lines - 1);
        (col, line, fx.fract() >= 0.5)
    }

    pub fn reset_zoom(&mut self) {
        self.zoom = 1.0;
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.5, 3.0);
    }

    /// A full-screen app started or ended; returns true if the grid has to
    /// be recomputed.
    pub fn set_full_screen(&mut self, on: bool) -> bool {
        let changed = self.fit_full_screen != on;
        self.fit_full_screen = on;
        changed
    }

    /// Left/right and top margins of the grid, in physical pixels.
    fn padding(&self) -> (f32, f32) {
        let scale = self.window.scale_factor() as f32;
        (PADDING * scale, (TITLE_STRIP + PADDING * 0.5) * scale)
    }

    fn metrics(&self, variant: &Variant) -> Metrics {
        let scale = self.window.scale_factor() as f32 * self.zoom * self.fit;
        Metrics::new(variant.font_size * scale, (variant.line_height * scale).round())
    }

    /// How many cells fit the window. While a full-screen app runs, the
    /// text shrinks (down to `MIN_FIT`) until the grid is at least
    /// `FULL_SCREEN_MIN`.
    pub fn grid_size(&mut self, variant: &Variant) -> GridSize {
        self.fit = 1.0;
        let mut size = self.measure(variant);
        let (columns, lines) = FULL_SCREEN_MIN;
        // Cell size scales with the font, apart from rounding: aim for the
        // ratio, then step down a little until it fits.
        for _ in 0..12 {
            if !self.fit_full_screen || (size.columns >= columns && size.lines >= lines) || self.fit <= MIN_FIT {
                break;
            }
            let ratio = (size.columns as f32 / columns as f32).min(size.lines as f32 / lines as f32);
            self.fit = (self.fit * ratio.min(0.97)).max(MIN_FIT);
            size = self.measure(variant);
        }
        size
    }

    /// Measures the theme's font at the current size and returns how many
    /// cells fit the window.
    fn measure(&mut self, variant: &Variant) -> GridSize {
        let metrics = self.metrics(variant);
        let mut probe = Buffer::new(&mut self.font_system, metrics);
        {
            let mut probe = probe.borrow_with(&mut self.font_system);
            probe.set_wrap(Wrap::None);
            let attrs = Attrs::new().family(Family::Name(&variant.font_family));
            probe.set_text("MMMMMMMMMM", &attrs, Shaping::Advanced, None);
            probe.shape_until_scroll(false);
        }
        let width = probe
            .layout_runs()
            .next()
            .map(|run| run.line_w / 10.0)
            .unwrap_or(metrics.font_size * 0.6);
        self.cell_width = width;
        self.cell_height = metrics.line_height;

        let (pad, top) = self.padding();
        let usable_w = self.config.width as f32 - 2.0 * pad;
        let usable_h = self.config.height as f32 - top - pad;
        let size = GridSize {
            columns: ((usable_w / self.cell_width) as usize).max(2),
            lines: ((usable_h / self.cell_height) as usize).max(1),
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        };
        let grid = (size.columns as u32, size.lines as u32);
        if grid != self.targets.grid {
            self.rebuild_targets(grid);
        }
        size
    }

    /// Draws a frame; `strikes`, when given, finds names of deleted files on
    /// screen to strike out.
    pub fn render<L: EventListener>(&mut self, variant: &Variant, term: &Term<L>, strikes: Option<&mut Strikes>) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                configure_surface(&self.surface, &self.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                configure_surface(&self.surface, &self.device, &self.config);
                return;
            }
            // Timeout, occluded window, ...: skip this frame.
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        self.draw_previews(term);
        self.layout(variant, term, strikes);
        self.draw(variant, &view);
        self.draw_picker(variant, &view);
        self.queue.present(frame);
        self.atlas.trim();
    }

    /// Opens the theme picker on `cards` (per theme, the variant its card
    /// previews) with `selected` chosen, or closes it (None).
    pub fn set_picker(&mut self, picker: Option<(usize, Vec<Variant>)>) {
        let Some((selected, variants)) = picker else {
            self.picker = None;
            return;
        };
        let same = self.picker.as_ref().is_some_and(|p| {
            p.cards.len() == variants.len() && p.cards.iter().zip(&variants).all(|(c, v)| c.variant.name == v.name)
        });
        if !same {
            let fs = &mut self.font_system;
            let mut buffer = || {
                let mut b = Buffer::new(fs, Metrics::new(12.0, 16.0));
                b.borrow_with(fs).set_wrap(Wrap::None);
                b
            };
            let (title, hint) = (buffer(), buffer());
            let cards = variants
                .into_iter()
                .map(|variant| PickerCard { variant, picture: None, name: buffer(), about: buffer() })
                .collect();
            self.picker = Some(PickerView { selected, cards, title, hint });
        }
        if let Some(p) = self.picker.as_mut() {
            p.selected = selected;
        }
    }

    /// Where the picker's cards are, physical pixels.
    pub fn picker_cards(&self) -> Vec<[f32; 4]> {
        let n = self.picker.as_ref().map_or(0, |p| p.cards.len());
        picker::cards(n, self.config.width as f32, self.config.height as f32, self.window.scale_factor() as f32)
    }

    /// Draws each picker card's picture: the screen as its variant draws it,
    /// with no writing in and no transition.
    fn draw_previews<L: EventListener>(&mut self, term: &Term<L>) {
        let Some(mut picker) = self.picker.take() else { return };
        let transition = self.transition.take();
        let settle_time = self.settle_time;
        self.thumbnail = true;
        let (w, h) = (self.config.width, self.config.height);
        for card in &mut picker.cards {
            let stale = card.picture.as_ref().is_none_or(|(t, _)| t.width() != w || t.height() != h);
            if stale {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("theme preview"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.config.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("theme preview"),
                    layout: &self.picker_layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: self.buffers.frame.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.ink_sampler) },
                    ],
                });
                card.picture = Some((texture, group));
            }
            let Some((texture, _)) = &card.picture else { continue };
            let view = texture.create_view(&Default::default());
            self.layout(&card.variant, term, None);
            self.draw(&card.variant, &view);
        }
        self.thumbnail = false;
        self.transition = transition;
        self.settle_time = settle_time;
        self.picker = Some(picker);
    }

    /// The picker over the window: the current look dimmed, a card per
    /// theme with its name and description, the chosen one outlined, and
    /// the keys at the bottom.
    fn draw_picker(&mut self, variant: &Variant, view: &wgpu::TextureView) {
        let Some(mut picker) = self.picker.take() else { return };
        let scale = self.window.scale_factor() as f32;
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let cards = picker::cards(picker.cards.len(), w, h, scale);
        let quiet = variant.palette[8];

        // The veil, the pictures, then the outlines.
        let mut shapes = vec![Shape::new(0.0, 0.0, w, h, variant.surface_rgb(), 0.95)];
        for &[x, y, cw, ch] in &cards {
            let mut picture = Shape::new(x, y, cw, ch, [0; 3], 1.0);
            picture.params[1] = 6.0 * scale;
            shapes.push(picture);
        }
        for (i, &[x, y, cw, ch]) in cards.iter().enumerate() {
            let chosen = i == picker.selected;
            let (t, gap, rgb, alpha) =
                if chosen { (2.0 * scale, 5.0 * scale, variant.cursor, 1.0) } else { (1.0 * scale, 3.0 * scale, quiet, 0.5) };
            let (x0, y0, x1, y1) = (x - gap, y - gap, x + cw + gap, y + ch + gap);
            shapes.extend([
                Shape::new(x0, y0, x1 - x0, t, rgb, alpha),
                Shape::new(x0, y1 - t, x1 - x0, t, rgb, alpha),
                Shape::new(x0, y0, t, y1 - y0, rgb, alpha),
                Shape::new(x1 - t, y0, t, y1 - y0, rgb, alpha),
            ]);
        }
        let needed = (shapes.len() * std::mem::size_of::<Shape>()) as u64;
        if needed > self.picker_buffer.size() {
            self.picker_buffer = Self::create_rect_buffer(&self.device, shapes.len().next_power_of_two());
        }
        self.queue.write_buffer(&self.picker_buffer, 0, bytemuck::cast_slice(&shapes));

        // Words, in the current look's font.
        let family = variant.font_family.clone();
        let weight = self.weight_for(&family, variant.font_weight);
        let bold = self.weight_for(&family, variant.font_weight.max(700));
        let attrs = |rgb: [u8; 3], weight: u16| {
            let [r, g, b] = rgb;
            Attrs::new().family(Family::Name(&family)).weight(Weight(weight)).color(Color::rgb(r, g, b))
        };
        let set = |buffer: &mut Buffer, fs: &mut FontSystem, text: &str, size: f32, width: f32, wrap: Wrap, attrs: Attrs| {
            let mut b = buffer.borrow_with(fs);
            b.set_metrics(Metrics::new(size * scale, (size * 1.4 * scale).round()));
            b.set_wrap(wrap);
            b.set_size(Some(width), Some(h));
            b.set_text(text, &attrs, Shaping::Advanced, None);
            b.shape_until_scroll(false);
        };
        let fs = &mut self.font_system;
        set(&mut picker.title, fs, "Themes", 18.0, w, Wrap::None, attrs(variant.ink, bold));
        set(
            &mut picker.hint,
            fs,
            "\u{2190} \u{2192}  choose      return  apply      esc  close      \u{21e7}\u{2318}L  light / dark",
            12.5,
            w,
            Wrap::None,
            attrs(quiet, weight),
        );
        let selected = picker.selected;
        for (i, (card, &[_, _, cw, _])) in picker.cards.iter_mut().zip(&cards).enumerate() {
            let name_rgb = if i == selected { variant.cursor } else { variant.ink };
            set(&mut card.name, fs, card.variant.theme.name(), 15.0, cw, Wrap::None, attrs(name_rgb, bold));
            set(&mut card.about, fs, card.variant.theme.about(), 12.0, cw, Wrap::Word, attrs(quiet, weight));
        }
        let width_of = |b: &Buffer| b.layout_runs().map(|r| r.line_w).fold(0.0, f32::max);
        let bounds = TextBounds { left: 0, top: 0, right: w as i32, bottom: h as i32 };
        let area = |buffer, left, top| TextArea {
            buffer,
            left,
            top,
            scale: 1.0,
            bounds,
            default_color: Color::rgb(0, 0, 0),
            custom_glyphs: &[],
        };
        let first = cards.first().copied().unwrap_or([0.0; 4]);
        let mut areas = vec![
            area(&picker.title, first[0], first[1] - 44.0 * scale),
            area(&picker.hint, (w - width_of(&picker.hint)) / 2.0, h - 40.0 * scale),
        ];
        for (card, &[x, y, _, ch]) in picker.cards.iter().zip(&cards) {
            areas.push(area(&card.name, x, y + ch + 12.0 * scale));
            areas.push(area(&card.about, x, y + ch + 36.0 * scale));
        }
        self.picker_text
            .prepare(&self.device, &self.queue, &mut self.font_system, &mut self.atlas, &self.viewport, areas, &mut self.swash_cache)
            .expect("prepare picker text");

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("picker") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("picker"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let stages = &self.stages[variant.theme.id()];
            let n = picker.cards.len() as u32;
            pass.set_vertex_buffer(0, self.picker_buffer.slice(..needed));
            pass.set_pipeline(&stages.shapes);
            pass.set_bind_group(0, &self.frame_group, &[]);
            pass.draw(0..4, 0..1);
            pass.set_pipeline(&self.picker_pipeline);
            for (i, card) in picker.cards.iter().enumerate() {
                if let Some((_, group)) = &card.picture {
                    pass.set_bind_group(0, group, &[]);
                    pass.draw(0..4, 1 + i as u32..2 + i as u32);
                }
            }
            pass.set_pipeline(&stages.shapes);
            pass.set_bind_group(0, &self.frame_group, &[]);
            pass.draw(0..4, 1 + n..shapes.len() as u32);
            self.picker_text.render(&self.atlas, &self.viewport, &mut pass).expect("render picker text");
        }
        self.queue.submit(Some(encoder.finish()));
        self.picker = Some(picker);
    }

    /// Renders `frames` stills of a light / dark switch from `from` to `to`,
    /// evenly through it, as `<prefix>-NN.png`. Used by
    /// `ONEPROMPT_SNAPSHOT_SWITCH` to look at transitions frame by frame.
    pub fn snapshot_switch<L: EventListener>(
        &mut self,
        from: &Variant,
        to: &Variant,
        term: &Term<L>,
        frames: usize,
        prefix: &std::path::Path,
    ) {
        // Draw the old look once, so the transition has a frame to cut into.
        self.snapshot(from, term, &prefix.with_extension("before.png"));
        self.animation.0 = true;
        self.start_transition(to);
        let Some(mut transition) = self.transition.clone() else { return };
        let started = transition.start;
        for k in 0..frames {
            transition.start = started - transition.duration * k as f32 / frames as f32;
            self.transition = Some(transition.clone());
            let name = format!("{}-{k:02}.png", prefix.display());
            self.snapshot(to, term, std::path::Path::new(&name));
        }
        self.transition = None;
    }

    /// Renders one frame off screen and saves it as a PNG. Used by
    /// `ONEPROMPT_SNAPSHOT` to preview themes without screen-capture access.
    pub fn snapshot<L: EventListener>(&mut self, variant: &Variant, term: &Term<L>, path: &std::path::Path) {
        let (w, h) = (self.config.width, self.config.height);
        let format = self.config.format;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("snapshot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.layout(variant, term, None);
        self.draw(variant, &texture.create_view(&Default::default()));

        // Rows in a texture-to-buffer copy must be 256-byte aligned.
        let row = w * 4;
        let padded_row = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("snapshot readback"),
            size: (padded_row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map snapshot"));
        self.device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .expect("wait for snapshot");

        let bgra = matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut pixels = Vec::with_capacity((row * h) as usize);
        {
            let data = readback.get_mapped_range(..).expect("read snapshot");
            for y in 0..h {
                let start = (y * padded_row) as usize;
                for px in data[start..start + row as usize].chunks_exact(4) {
                    if bgra {
                        pixels.extend_from_slice(&[px[2], px[1], px[0], 255]);
                    } else {
                        pixels.extend_from_slice(&[px[0], px[1], px[2], 255]);
                    }
                }
            }
        }
        readback.unmap();

        let file = std::fs::File::create(path).expect("create snapshot file");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&pixels))
            .expect("write snapshot png");
    }

    /// The mentions of deleted paths on screen, each with how far its cut has
    /// got (0..1) and how long since it started, in seconds at speed 1.
    /// Full-screen apps draw their own screens, so none there.
    fn find_struck(
        &mut self,
        variant: &Variant,
        cells: &[Indexed<&Cell>],
        display_offset: usize,
        strikes: Option<&mut Strikes>,
        full_screen: bool,
    ) -> (Vec<(Struck, f32, f32)>, f32) {
        let Some(style) = variant.theme.strike() else { return (Vec::new(), 0.0) };
        let Some(strikes) = strikes.filter(|_| !full_screen) else { return (Vec::new(), 0.0) };
        let (columns, lines) = (self.targets.grid.0 as usize, self.targets.grid.1 as usize);
        let mut chars = vec![' '; columns * lines];
        for indexed in cells {
            let line = indexed.point.line.0 + display_offset as i32;
            let col = indexed.point.column.0;
            if line >= 0 && (line as usize) < lines && col < columns {
                chars[line as usize * columns + col] = cell_char(indexed.cell.c);
            }
        }
        let now = self.now();
        let duration = style.duration / self.animation.1;
        let mut found = strikes.find(&chars, columns, now);
        // Mentions of one deletion are cut one after another, in reading order.
        let mut order: HashMap<u32, usize> = HashMap::new();
        let found = found
            .drain(..)
            .map(|s| {
                let k = order.entry(s.born.to_bits()).or_default();
                let start = s.born + (*k as f32 * STRIKE_STAGGER / self.animation.1).min(0.5);
                *k += 1;
                let linger = style.linger / self.animation.1;
                self.strikes_until = self.strikes_until.max(start + duration + linger);
                let (progress, age) = if self.animation.0 {
                    (((now - start) / duration).clamp(0.0, 1.0), (now - start) * self.animation.1)
                } else {
                    (1.0, f32::INFINITY)
                };
                (s, progress, age)
            })
            .filter(|&(_, progress, _)| progress > 0.0)
            .collect();
        (found, style.fade)
    }

    /// Turns the visible part of the grid into styled text runs plus
    /// rectangles for backgrounds, decorations and the cursor.
    fn layout<L: EventListener>(&mut self, variant: &Variant, term: &Term<L>, strikes: Option<&mut Strikes>) {
        let content = term.renderable_content();
        let (pad, top) = self.padding();
        let (cw, ch) = (self.cell_width, self.cell_height);
        let cell_x = |col: usize| pad + col as f32 * cw;
        let cell_y = |line: usize| top + line as f32 * ch;

        let cursor = content.cursor;
        let cursor_line = cursor.point.line.0 + content.display_offset as i32;
        let cursor_col = cursor.point.column.0;
        // An unfocused window shows an outline instead of a solid block.
        let cursor_shape = match cursor.shape {
            CursorShape::Block if !term.is_focused => CursorShape::HollowBlock,
            shape => shape,
        };
        let block_cursor = cursor_shape == CursorShape::Block && cursor_line >= 0;
        // An empty selection (a click without a drag) still reports one
        // cell; nothing is selected.
        let selection = if term.selection.as_ref().is_some_and(|s| s.is_empty()) { None } else { content.selection };

        // What each cell shows, for the writing animation, and the pen path
        // (atlas slot) to write it with.
        let (columns, lines) = (self.targets.grid.0 as usize, self.targets.grid.1 as usize);
        let mut chars = vec![' '; columns * lines];
        let mut slots = vec![0.0f32; columns * lines];
        if !self.thumbnail {
            self.reset_tracer_if_needed(variant);
        }
        let regular = self.weight_for(&variant.font_family, variant.font_weight);
        // Bold cells use the bold face; a theme that is already bold has no
        // heavier face to go to, and synthesized weight just blots the ink.
        let bold = self.weight_for(&variant.font_family, variant.font_weight.max(700));

        // Collect runs of identically styled text: (line, text, color, flags).
        let mut runs: Vec<(usize, String, [u8; 3], Flags)> = Vec::new();
        // Decorations drawn in ink: underlines, box drawing.
        let mut overlay: Vec<Shape> = Vec::new();
        self.rects.clear();
        self.ink_rects.clear();
        let cells: Vec<_> = content.display_iter.collect();
        self.loose.clear();
        let metrics_key = {
            let m = self.metrics(variant);
            (variant.font_family.clone(), m.font_size.to_bits(), m.line_height.to_bits())
        };
        if metrics_key != self.loose_metrics || self.loose_glyphs.len() > 4096 {
            self.loose_glyphs.clear();
            self.loose_metrics = metrics_key;
        }
        let canvas = if self.harmonize { canvas_color(&cells, columns * lines) } else { None };
        let full_screen = term.mode().contains(TermMode::ALT_SCREEN);
        let (struck, fade) = self.find_struck(variant, &cells, content.display_offset, strikes, full_screen);
        // How far each cell's strike has got (0: not struck), to fade it.
        let mut struck_cells = vec![0.0f32; columns * lines];
        for &(s, progress, _) in &struck {
            for col in s.col..(s.col + s.len).min(columns) {
                struck_cells[s.line * columns + col] = progress;
            }
        }
        for indexed in cells {
            let line = (indexed.point.line.0 + content.display_offset as i32) as usize;
            let col = indexed.point.column.0;
            let flags = indexed.cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            if line < lines && col < columns && !flags.contains(Flags::HIDDEN) {
                let c = cell_char(indexed.cell.c);
                chars[line * columns + col] = c;
                if self.animation.0 && !self.thumbnail && c != ' ' && boxdraw::pieces(c, 1.0, 1.0).is_none() {
                    let key = GlyphKey {
                        c,
                        weight: if flags.contains(Flags::BOLD) { bold } else { regular },
                        italic: flags.contains(Flags::ITALIC),
                    };
                    slots[line * columns + col] = match self.tracer.slot(key) {
                        Slot::Ready(slot) => slot as f32,
                        Slot::New(slot) if self.trace_glyph(variant, key, slot) => slot as f32,
                        Slot::New(_) => {
                            self.tracer.forget(key);
                            0.0
                        }
                        Slot::None => 0.0,
                    };
                }
            }

            let mut fg = resolve(variant, indexed.cell.fg, variant.ink);
            let mut bg = match indexed.cell.bg {
                AnsiColor::Named(NamedColor::Background) => None,
                // The app's own canvas: the sheet is the canvas here.
                color if Some(color) == canvas && !flags.contains(Flags::INVERSE) => None,
                color => Some(resolve(variant, color, variant.surface_rgb())),
            };
            // Colours the program picked itself were chosen for a plain
            // screen; bring them onto the sheet as inks and washes.
            let mut bg_alpha = CELL_BG_ALPHA;
            if self.harmonize {
                if program_chosen(indexed.cell.fg) {
                    fg = color::ink(fg, variant);
                }
                if program_chosen(indexed.cell.bg) && !flags.contains(Flags::INVERSE) {
                    // A translucent tint of the program's hue, inked for the
                    // sheet: the paper stays visible underneath.
                    bg = bg.map(|b| color::ink(b, variant));
                    bg_alpha = if variant.dark { WASH_ALPHA_DARK } else { WASH_ALPHA_LIGHT };
                }
            }
            if flags.contains(Flags::INVERSE) {
                let old_bg = bg.unwrap_or(variant.surface_rgb());
                bg = Some(fg);
                fg = old_bg;
            }
            if flags.contains(Flags::DIM) {
                fg = mix(fg, bg.unwrap_or(variant.surface_rgb()), 0.4);
            }
            if let Some(&progress) = struck_cells.get(line * columns + col)
                && progress > 0.0
            {
                fg = mix(fg, bg.unwrap_or(variant.surface_rgb()), fade * progress);
            }
            // Selected text is reversed out of a block of ink: the letters
            // take the paper's colour.
            let selected = selection.is_some_and(|s| s.contains(indexed.point));
            if selected {
                fg = variant.surface_rgb();
                bg = Some(variant.selection.0);
            }
            let is_cursor = block_cursor && line as i32 == cursor_line && col == cursor_col;
            if is_cursor {
                fg = bg.unwrap_or(variant.surface_rgb());
            }

            let width = if flags.contains(Flags::WIDE_CHAR) { 2.0 } else { 1.0 };
            let (x, y) = (cell_x(col), cell_y(line));
            if let Some(bg) = bg {
                let alpha = if selected { variant.selection.1 } else { bg_alpha };
                self.rects.push(Shape::new(x, y, cw * width, ch, bg, alpha));
            }
            if flags.contains(Flags::UNDERLINE) {
                let y = y + ch * 0.86;
                overlay.push(Shape::new(x, y, cw * width, (ch * 0.06).max(1.0), fg, 1.0));
            }
            if flags.contains(Flags::STRIKEOUT) {
                let y = y + ch * 0.5;
                overlay.push(Shape::new(x, y, cw * width, (ch * 0.06).max(1.0), fg, 1.0));
            }
            let drawn = (!flags.contains(Flags::HIDDEN))
                .then(|| boxdraw::pieces(indexed.cell.c, cw, ch))
                .flatten();

            let mut ch_str = String::new();
            if let Some(pieces) = drawn {
                for (px, py, pw, ph, alpha) in pieces {
                    overlay.push(Shape::new(x + px, y + py, pw, ph, fg, alpha));
                }
                ch_str.push(' ');
            } else if flags.contains(Flags::HIDDEN) {
                ch_str.push(' ');
            } else {
                let c = cell_char(indexed.cell.c);
                let mut glyph = String::from(c);
                if let Some(extra) = indexed.cell.zerowidth() {
                    glyph.extend(extra);
                }
                // In a line of text, a character from a fallback font
                // advances by its own width and pushes the rest of the row
                // off the grid. Such characters are drawn on their own at
                // their cell; the row keeps blanks in their place.
                if width > 1.0 || (!c.is_ascii() && !self.covers(&variant.font_family, c)) {
                    let weight = if flags.contains(Flags::BOLD) { bold } else { regular };
                    self.loose.push((x, y, (glyph, weight, flags.contains(Flags::ITALIC)), fg));
                    ch_str.push_str(if width > 1.0 { "  " } else { " " });
                } else {
                    ch_str.push_str(&glyph);
                }
            }
            let style = flags & (Flags::BOLD | Flags::ITALIC);
            match runs.last_mut() {
                Some((l, text, color, f)) if *l == line && *color == fg && *f == style => {
                    text.push_str(&ch_str)
                }
                _ => {
                    // Close the previous line before starting a new one.
                    let prev_line = runs.last().map_or(line, |run| run.0);
                    for _ in prev_line..line {
                        runs.push((line, "\n".into(), variant.ink, Flags::empty()));
                    }
                    runs.push((line, ch_str, fg, style));
                }
            }
        }

        // The cursor and struck-out names are the theme's to draw, in the
        // ink layer; the cursor goes under the text.
        let ctx = self.ctx();
        if cursor_line >= 0 && cursor_shape != CursorShape::Hidden {
            let kind = match cursor_shape {
                CursorShape::Beam => CursorKind::Beam,
                CursorShape::Underline => CursorKind::Underline,
                CursorShape::HollowBlock => CursorKind::Hollow,
                _ => CursorKind::Block,
            };
            let cursor = Cursor {
                cell: [cell_x(cursor_col), cell_y(cursor_line as usize), cw, ch],
                column: cursor_col,
                line: cursor_line as usize,
                kind,
            };
            variant.theme.cursor(variant, &ctx, &cursor, &mut self.ink_rects);
        }
        // Seeded without the line, so a strike keeps its shape as the text
        // scrolls.
        for (i, &(s, progress, age)) in struck.iter().enumerate() {
            let span = StrikeSpan {
                word: [cell_x(s.col), cell_y(s.line), cw * s.len as f32, ch],
                progress,
                age,
                seed: s.col as f32 * 3.71 + s.len as f32 * 1.93 + s.born,
                extras: self.splatter && i < MAX_STRIKE_EXTRAS,
            };
            variant.theme.draw_strike(variant, &ctx, &span, &mut overlay);
        }
        self.ink_rects.extend(overlay);
        self.layout_controls(variant, term.is_focused);
        self.layout_notice(variant);
        self.layout_tabs(variant);

        let now = self.now();
        let writing = Writing {
            enabled: self.animation.0,
            speed: self.writing_speed,
            max_duration: MAX_WRITE / self.animation.1,
            instant: term.mode().contains(TermMode::ALT_SCREEN),
        };
        // A preview shows the screen as it is: no writing in, and it leaves
        // the live screen's record of what was written alone.
        let births = if self.thumbnail { Vec::new() } else { self.animator.update(columns, &chars, now, writing) };
        if births.len() == columns * lines {
            let cells: Vec<[f32; 2]> = births.iter().zip(&slots).map(|(&b, &s)| [b, s]).collect();
            self.queue.write_texture(
                self.targets.births.as_image_copy(),
                bytemuck::cast_slice(&cells),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(columns as u32 * 8),
                    rows_per_image: Some(lines as u32),
                },
                wgpu::Extent3d { width: columns as u32, height: lines as u32, depth_or_array_layers: 1 },
            );
        }

        let family = Family::Name(&variant.font_family);
        let spans = runs.iter().map(|(_, text, [r, g, b], flags)| {
            let weight = if flags.contains(Flags::BOLD) { bold } else { regular };
            let mut attrs = Attrs::new()
                .family(family)
                .weight(Weight(weight))
                .color(Color::rgb(*r, *g, *b));
            if flags.contains(Flags::ITALIC) {
                attrs = attrs.style(Style::Italic);
            }
            (text.as_str(), attrs)
        });
        let metrics = self.metrics(variant);
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let mut text = self.text.borrow_with(&mut self.font_system);
        text.set_metrics(metrics);
        text.set_size(Some(w), Some(h));
        text.set_rich_text(spans, &Attrs::new().family(family), Shaping::Advanced, None);
        text.shape_until_scroll(false);

        for (_, _, key, _) in &self.loose {
            if self.loose_glyphs.contains_key(key) {
                continue;
            }
            let mut buffer = Buffer::new(&mut self.font_system, metrics);
            let mut attrs = Attrs::new().family(Family::Name(&variant.font_family)).weight(Weight(key.1));
            if key.2 {
                attrs = attrs.style(Style::Italic);
            }
            {
                let mut b = buffer.borrow_with(&mut self.font_system);
                b.set_wrap(Wrap::None);
                b.set_size(Some(self.cell_width * 4.0), Some(self.cell_height * 2.0));
                b.set_text(&key.0, &attrs, Shaping::Advanced, None);
                b.shape_until_scroll(false);
            }
            self.loose_glyphs.insert(key.clone(), buffer);
        }
    }

    /// The notice, set in the theme's font, small, in the bottom margin.
    fn layout_notice(&mut self, variant: &Variant) {
        let Some(notice) = &self.notice else { return };
        let scale = self.window.scale_factor() as f32;
        // Errors in the theme's red, confirmations in a quiet ink.
        let [r, g, b] = if notice.is_error { variant.palette[1] } else { variant.palette[8] };
        let attrs = Attrs::new()
            .family(Family::Name(&variant.font_family))
            .color(Color::rgb(r, g, b));
        let mut buffer = self.notice_text.borrow_with(&mut self.font_system);
        buffer.set_metrics(Metrics::new(12.5 * scale, 16.0 * scale));
        buffer.set_size(Some(self.config.width as f32), Some(20.0 * scale));
        buffer.set_text(&notice.text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(false);
    }

    /// Tab labels, evenly spread over the strip right of the controls, the
    /// active one in ink and underlined with a stroke of the brush, the
    /// others in a quiet ink.
    fn layout_tabs(&mut self, variant: &Variant) {
        self.tab_slots.clear();
        if self.tabs.is_empty() {
            return;
        }
        let scale = self.window.scale_factor() as f32;
        let (x, _, r) = chrome::geometry(Control::Zoom, scale);
        let left = x + r + 14.0 * scale;
        let text_y = self.strip_text_y();
        let right = self.config.width as f32 - 16.0 * scale;
        let n = self.tabs.len();
        let slot_w = ((right - left) / n as f32).min(220.0 * scale);
        if slot_w < 24.0 * scale {
            return;
        }
        let start = left + ((right - left) - slot_w * n as f32) / 2.0;
        while self.tab_text.len() < n {
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(12.0, 16.0));
            buffer.borrow_with(&mut self.font_system).set_wrap(Wrap::None);
            self.tab_text.push(buffer);
        }
        let weight = self.weight_for(&variant.font_family, variant.font_weight);
        for (i, label) in self.tabs.iter().enumerate() {
            let active = i == self.active_tab;
            let all_selected = self.editing_tab == Some((i, true));
            let [r, g, b] = if all_selected {
                variant.surface_rgb()
            } else if active {
                variant.ink
            } else {
                variant.palette[8]
            };
            let attrs =
                Attrs::new().family(Family::Name(&variant.font_family)).weight(Weight(weight)).color(Color::rgb(r, g, b));
            let mut buffer = self.tab_text[i].borrow_with(&mut self.font_system);
            buffer.set_metrics(Metrics::new(12.5 * scale, 16.0 * scale));
            buffer.set_size(Some(slot_w * 4.0), Some(20.0 * scale));
            buffer.set_text(label, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(false);
            let width = buffer.layout_runs().map(|run| run.line_w).fold(0.0, f32::max);
            let slot_left = start + slot_w * i as f32;
            let margin = 8.0 * scale;
            // Centred in its slot; long titles start at the slot's edge and are clipped.
            let text_x = (slot_left + (slot_w - width) / 2.0).max(slot_left + margin);
            let editing = self.editing_tab.is_some_and(|(e, _)| e == i);
            // While a name is typed, it starts at the slot's edge so the end
            // (where the caret is) stays in view, and the caret follows it.
            let text_x = if editing { text_x.min(slot_left + slot_w - margin - width - 4.0 * scale) } else { text_x };
            if all_selected && width > 0.0 {
                let (rgb, alpha) = variant.selection;
                let x0 = text_x.max(slot_left + margin) - 2.0 * scale;
                let w = width.min(slot_w - 2.0 * margin) + 4.0 * scale;
                self.rects.push(Shape::new(x0, text_y - 1.0 * scale, w, 19.0 * scale, rgb, alpha));
            }
            if editing {
                let caret_x = (text_x + width + 1.5 * scale).min(slot_left + slot_w - margin);
                self.ink_rects.push(Shape::new(caret_x, text_y + 1.0 * scale, 1.6 * scale, 15.0 * scale, variant.cursor, 1.0));
            }
            if active || editing {
                let w = width.min(slot_w - 2.0 * margin) + 6.0 * scale;
                let y = text_y + 17.0 * scale;
                self.ink_rects.push(Shape::new(text_x.max(slot_left + margin) - 3.0 * scale, y, w, 1.8 * scale, variant.cursor, 0.9));
            }
            self.tab_slots.push(TabSlot {
                left: slot_left,
                right: slot_left + slot_w,
                text: (text_x, text_y),
                clip: slot_left + slot_w - margin,
            });
        }
    }

    /// Top-left corner of the notice: in the margin below the text, lined
    /// up with it, where it does not cover anything.
    fn notice_origin(&self) -> (f32, f32) {
        let scale = self.window.scale_factor() as f32;
        let (pad, _) = self.padding();
        (pad, self.config.height as f32 - (pad + 16.0 * scale) / 2.0)
    }

    /// Where tab labels sit vertically: centred on the window controls.
    fn strip_text_y(&self) -> f32 {
        let scale = self.window.scale_factor() as f32;
        let (_, cy, _) = chrome::geometry(Control::Zoom, scale);
        cy - 8.0 * scale
    }

    /// The window's close / minimize / zoom controls, in the theme's style.
    fn layout_controls(&mut self, variant: &Variant, focused: bool) {
        let scale = self.window.scale_factor() as f32;
        // Like macOS: controls fade when the window is in the background.
        let alpha = if focused || self.controls_hover { 1.0 } else { 0.45 };
        let surface = variant.surface_rgb();
        for (i, control) in chrome::CONTROLS.into_iter().enumerate() {
            let (cx, cy, r) = chrome::geometry(control, scale);
            let color = variant.controls.colors[i];
            let shape = match variant.controls.style {
                ControlStyle::Dot => Shape::round(DISC, cx, cy, r, color, alpha),
                ControlStyle::Brush => {
                    Shape::round(DISC, cx, cy, r, color, alpha).rough(0.16, i as f32 + 1.0)
                }
                ControlStyle::Ring => Shape::round(RING, cx, cy, r, color, alpha)
                    .stroke(1.6 * scale)
                    .rough(0.08, i as f32 + 1.0),
            };
            self.ink_rects.push(shape);

            if !self.controls_hover {
                continue;
            }
            // Symbols: ring style draws them in the control's colour,
            // filled styles cut them out in the surface colour.
            let mark = if variant.controls.style == ControlStyle::Ring { color } else { surface };
            let t = (1.3 * scale).max(1.0);
            let arm = r * 0.5;
            match control {
                Control::Close => self
                    .ink_rects
                    .push(Shape::round(CROSS, cx, cy, arm, mark, 0.9).stroke(t)),
                Control::Minimize => {
                    self.ink_rects.push(Shape::new(cx - arm, cy - t / 2.0, arm * 2.0, t, mark, 0.9))
                }
                Control::Zoom => self.ink_rects.extend([
                    Shape::new(cx - arm, cy - t / 2.0, arm * 2.0, t, mark, 0.9),
                    Shape::new(cx - t / 2.0, cy - arm, t, arm * 2.0, mark, 0.9),
                ]),
            }
        }
    }

    fn draw(&mut self, variant: &Variant, view: &wgpu::TextureView) {
        let (w, h) = (self.config.width, self.config.height);
        let scale = self.window.scale_factor() as f32;
        let now = self.now();
        if self.transition.as_ref().is_some_and(|t| t.done(now)) {
            self.transition = None;
        }
        let (progress, from_color) = match &self.transition {
            Some(t) => {
                let from = match t.from {
                    animation::From::Color([r, g, b]) => [r, g, b, 1.0],
                    animation::From::PreviousFrame => [0.0; 4],
                };
                (t.progress(now).min(0.999), from)
            }
            None => (1.0, [0.0; 4]),
        };
        let rgb = |c: [u8; 3]| {
            let [r, g, b] = c.map(|v| v as f32 / 255.0);
            [r, g, b, 1.0]
        };
        let (pad, top) = self.padding();
        let frame = Frame {
            resolution: [w as f32, h as f32],
            scale,
            seed: self.seed,
            time: now,
            speed: self.animation.1,
            progress,
            _pad0: 0.0,
            from_color,
            grid_origin: [pad, top],
            cell: [self.cell_width, self.cell_height],
            grid_size: [self.targets.grid.0 as f32, self.targets.grid.1 as f32],
            slot_size: [self.tracer.slot_size.0 as f32, self.tracer.slot_size.1 as f32],
            write_time: if self.animation.0 && !self.thumbnail { WRITE_TIME / self.animation.1 } else { 0.0 },
            atlas_columns: self.tracer.columns() as f32,
            _pad1: [0.0; 2],
            surface: [variant.surface[0], variant.surface[1], variant.surface[2], 1.0],
            ink: rgb(variant.ink),
            accent: rgb(variant.cursor),
        };
        self.queue.write_buffer(&self.buffers.frame, 0, bytemuck::bytes_of(&frame));
        let theme = variant.theme;
        let mut data = theme.data(variant, &self.ctx());
        data.resize(api::DATA_SIZE, 0);
        self.queue.write_buffer(&self.buffers.data, 0, &data);
        if let Some(t) = &self.transition {
            let mut plan = t.data.clone();
            plan.resize(api::PLAN_SIZE, 0);
            self.queue.write_buffer(&self.buffers.plan, 0, &plan);
        }
        self.settle_time = theme.settle_time(variant);
        if !self.stages.contains_key(theme.id()) {
            let stages = Stages::new(&self.device, &self.layouts, self.config.format, theme);
            self.stages.insert(theme.id(), stages);
        }

        // Both rect lists share one buffer: surface rects, then ink rects.
        let surface_count = self.rects.len() as u32;
        let total = self.rects.len() + self.ink_rects.len();
        let needed = (total * std::mem::size_of::<Shape>()) as u64;
        if needed > self.rect_buffer.size() {
            self.rect_buffer = Self::create_rect_buffer(&self.device, total.next_power_of_two());
        }
        if total > 0 {
            let all: Vec<Shape> = self.rects.iter().chain(&self.ink_rects).copied().collect();
            self.queue
                .write_buffer(&self.rect_buffer, 0, bytemuck::cast_slice(&all));
        }

        self.viewport.update(&self.queue, Resolution { width: w, height: h });
        let (pad, top) = self.padding();
        let bounds = TextBounds { left: 0, top: 0, right: w as i32, bottom: h as i32 };
        let [r, g, b] = variant.ink;
        let mut areas = vec![TextArea {
            buffer: &self.text,
            left: pad,
            top,
            scale: 1.0,
            bounds,
            default_color: Color::rgb(r, g, b),
            custom_glyphs: &[],
        }];
        // Characters drawn on their own, each at its cell.
        for (x, y, key, [r, g, b]) in &self.loose {
            if let Some(buffer) = self.loose_glyphs.get(key) {
                areas.push(TextArea {
                    buffer,
                    left: *x,
                    top: *y,
                    scale: 1.0,
                    bounds,
                    default_color: Color::rgb(*r, *g, *b),
                    custom_glyphs: &[],
                });
            }
        }
        let strip = (TITLE_STRIP * self.window.scale_factor() as f32) as i32;
        for (slot, buffer) in self.tab_slots.iter().zip(&self.tab_text) {
            areas.push(TextArea {
                buffer,
                left: slot.text.0,
                top: slot.text.1,
                scale: 1.0,
                bounds: TextBounds { left: slot.left as i32, top: 0, right: slot.clip as i32, bottom: strip },
                default_color: Color::rgb(r, g, b),
                custom_glyphs: &[],
            });
        }
        if self.notice.is_some() {
            let (left, top) = self.notice_origin();
            // Long messages are cut at the right margin.
            let right = (w as f32 - left) as i32;
            areas.push(TextArea {
                buffer: &self.notice_text,
                left,
                top,
                scale: 1.0,
                bounds: TextBounds { right, ..bounds },
                default_color: Color::rgb(r, g, b),
                custom_glyphs: &[],
            });
        }
        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash_cache,
            )
            .expect("prepare text");

        let stages = &self.stages[theme.id()];
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        let attachment = |view, load| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ink pass"),
                color_attachments: &[attachment(
                    &self.targets.ink_view,
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                )],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !self.ink_rects.is_empty() {
                pass.set_pipeline(&stages.shapes);
                pass.set_bind_group(0, &self.frame_group, &[]);
                pass.set_vertex_buffer(0, self.rect_buffer.slice(..needed));
                pass.draw(0..4, surface_count..total as u32);
            }
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("render text");
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[attachment(
                    &self.targets.scene_view,
                    wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                )],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&stages.surface);
            pass.set_bind_group(0, &self.frame_group, &[]);
            pass.draw(0..3, 0..1);

            if surface_count > 0 {
                pass.set_pipeline(&stages.shapes);
                pass.set_bind_group(0, &self.frame_group, &[]);
                pass.set_vertex_buffer(0, self.rect_buffer.slice(..needed));
                pass.draw(0..4, 0..surface_count);
            }

            pass.set_pipeline(&stages.ink);
            pass.set_bind_group(0, &self.targets.ink_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present pass"),
                color_attachments: &[attachment(view, wgpu::LoadOp::Clear(wgpu::Color::BLACK))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&stages.transition);
            pass.set_bind_group(0, &self.targets.present_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
    }
}

/// Maps a terminal colour onto the theme. The 16 ANSI colours come from the
/// theme's palette; 256-colour and true-colour values are used as-is.
fn resolve(variant: &Variant, color: AnsiColor, default: [u8; 3]) -> [u8; 3] {
    match color {
        AnsiColor::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
        AnsiColor::Indexed(i) => indexed(variant, i),
        AnsiColor::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground => variant.ink,
            NamedColor::DimForeground => mix(variant.ink, variant.surface_rgb(), 0.4),
            NamedColor::Background => variant.surface_rgb(),
            NamedColor::Cursor => variant.cursor,
            NamedColor::DimBlack => variant.palette[0],
            NamedColor::DimRed => variant.palette[1],
            NamedColor::DimGreen => variant.palette[2],
            NamedColor::DimYellow => variant.palette[3],
            NamedColor::DimBlue => variant.palette[4],
            NamedColor::DimMagenta => variant.palette[5],
            NamedColor::DimCyan => variant.palette[6],
            NamedColor::DimWhite => variant.palette[7],
            other => match (other as usize) < 16 {
                true => variant.palette[other as usize],
                false => default,
            },
        },
    }
}

/// Configures the surface and pins its colour space to sRGB.
///
/// wgpu's `SurfaceColorSpace::Srgb` leaves the Metal layer's colorspace unset
/// (nil), i.e. untagged. On wide-gamut displays macOS then colour-matches some
/// frames and passes others through untouched, so the whole window shifts
/// tint/brightness from frame to frame (seen while typing, when frames are
/// presented in quick succession). Tagging the layer as sRGB makes every
/// frame go through the same conversion. Configuring resets the tag, so this
/// runs after every configure.
fn configure_surface(
    surface: &wgpu::Surface<'static>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) {
    surface.configure(device, config);
    #[cfg(target_os = "macos")]
    // SAFETY: the hal surface is only used to set a layer property, on the
    // thread that owns the window, while no frame is acquired.
    unsafe {
        use objc2_core_graphics::{CGColorSpace, kCGColorSpaceSRGB};
        if let Some(hal) = surface.as_hal::<wgpu::hal::api::Metal>() {
            let srgb = CGColorSpace::with_name(Some(kCGColorSpaceSRGB));
            hal.render_layer().lock().setColorspace(srgb.as_deref());
        }
    }
}

/// A new seed for every launch, so each session gets a different sheet of
/// paper. `ONEPROMPT_SEED` pins it (handy for comparing screenshots).
fn session_seed() -> f32 {
    if let Some(seed) = std::env::var("ONEPROMPT_SEED").ok().and_then(|s| s.parse().ok()) {
        return seed;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    (nanos % 100_000) as f32 / 100.0
}

/// The character to lay out for a cell. Each cell must advance exactly one
/// column, but the grid keeps a literal `\t` where a tab started (so copying
/// preserves it), and the text engine would expand that to its own tab width
/// and push the rest of the line out of the grid. Control characters are
/// therefore drawn as blanks.
fn cell_char(c: char) -> char {
    if c.is_control() { ' ' } else { c }
}

/// Where a tab's label sits in the title strip, in physical pixels.
struct TabSlot {
    left: f32,
    right: f32,
    /// Top-left of its text, and where the text is clipped.
    text: (f32, f32),
    clip: f32,
}

/// A seed that differs every launch.
fn random_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    hasher.finish()
}

/// A character drawn on its own: its text, weight and italic.
type LooseKey = (String, u16, bool);

/// Share of the screen one background colour must cover to count as the
/// program's canvas.
const CANVAS_SHARE: f32 = 0.5;

/// The background a full-screen app paints everywhere (btop, themed TUIs):
/// it stands for "the screen", so it is drawn as the bare sheet rather than
/// tinting the whole window. Only colours set explicitly count.
fn canvas_color(cells: &[Indexed<&Cell>], screen: usize) -> Option<AnsiColor> {
    let mut counts: Vec<(AnsiColor, usize)> = Vec::new();
    for cell in cells {
        let bg = cell.cell.bg;
        if bg == AnsiColor::Named(NamedColor::Background) || cell.cell.flags.contains(Flags::INVERSE) {
            continue;
        }
        if let Some((_, n)) = counts.iter_mut().find(|(c, _)| *c == bg) {
            *n += 1;
        } else if counts.len() < 64 {
            counts.push((bg, 1));
        }
    }
    let (color, n) = counts.into_iter().max_by_key(|&(_, n)| n)?;
    (n as f32 >= screen as f32 * CANVAS_SHARE).then_some(color)
}

/// True-colour and 256-colour values: picked by the program, not the theme.
fn program_chosen(color: AnsiColor) -> bool {
    matches!(color, AnsiColor::Spec(_) | AnsiColor::Indexed(16..))
}

/// The theme's answer for a colour query (OSC 4 / 10 / 11 / 12): palette
/// entries 0..=255, then foreground, background and cursor.
pub fn query_color(variant: &Variant, index: usize) -> [u8; 3] {
    match index {
        0..=255 => indexed(variant, index as u8),
        256 => variant.ink,
        257 => variant.surface_rgb(),
        258 => variant.cursor,
        _ => variant.ink,
    }
}

/// xterm's 256-colour table: 16 theme colours, a 6x6x6 cube, then greys.
fn indexed(variant: &Variant, i: u8) -> [u8; 3] {
    match i {
        0..=15 => variant.palette[i as usize],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [level(i / 36), level((i / 6) % 6), level(i % 6)]
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            [v, v, v]
        }
    }
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    [0, 1, 2].map(|i| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every stage of every theme compiles and validates, so a shader
    /// mistake fails here instead of when the theme is first drawn.
    #[test]
    fn every_theme_stage_compiles() {
        use wgpu::naga;
        for theme in api::all() {
            for (stage, source) in ["surface", "shapes", "ink", "transition"].iter().zip(stage_sources(theme)) {
                let module = naga::front::wgsl::parse_str(&source)
                    .unwrap_or_else(|e| panic!("{} {stage}: {}", theme.id(), e.emit_to_string(&source)));
                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
                    .validate(&module)
                    .unwrap_or_else(|e| panic!("{} {stage}: {}", theme.id(), e.emit_to_string(&source)));
            }
        }
    }

    #[test]
    fn control_characters_take_one_blank_cell() {
        assert_eq!(cell_char('\t'), ' ');
        assert_eq!(cell_char('\u{7f}'), ' ');
        assert_eq!(cell_char('a'), 'a');
        assert_eq!(cell_char('─'), '─');
    }

    #[test]
    fn weights_snap_to_installed_faces() {
        let courier = [400, 700];
        assert_eq!(snap_weight(&courier, 100), 400);
        assert_eq!(snap_weight(&courier, 450), 400);
        assert_eq!(snap_weight(&courier, 550), 700); // tie above 500: heavier
        assert_eq!(snap_weight(&courier, 900), 700);
        assert_eq!(snap_weight(&[300, 500], 400), 300); // tie below 500: lighter
        assert_eq!(snap_weight(&[], 550), 550);
    }
}
