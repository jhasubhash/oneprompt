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
//!
//! With splits, passes 1 and 2 run once per pane, each with its own grid
//! (origin, size, cell births): the first pane lays the surface for all of
//! them, so the sheet runs on under the dividers, and the last adds the
//! window's own marks before pass 3 runs once for the window.

use std::collections::{HashMap, HashSet};
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
use crate::settings::{self, Control as Setting, Hit, Item, Part};
use crate::marks::{self, Finished};
use crate::strike::{Strikes, Struck};
use crate::terminal::GridSize;
use crate::theme::{
    self as api, CommandSpan, ControlStyle, Ctx, Cursor, CursorKind, Shape, StrikeSpan, Theme, Variant, CROSS, DISC,
    RING,
};

/// Margin around the grid, in logical pixels.
const PADDING: f32 = 24.0;
/// The grid full-screen apps (btop, vim, ...) get at least: the classic
/// terminal size most of them are designed for. While one runs in a pane
/// (or a window) too small for it at the chosen font size, that pane's text
/// shrinks to fit.
const FULL_SCREEN_MIN: (usize, usize) = (80, 24);
/// Text shrinks to no less than this share of its size.
const MIN_FIT: f32 = 0.5;
/// No theme's settings panel takes longer than this to come in, seconds.
const SETTINGS_ENTRANCE_MAX: f32 = 1.0;
/// Progress shown eases towards what is reported over about this long, and
/// fades out over PROGRESS_FADE once the program is done (seconds).
const PROGRESS_EASE: f32 = 0.25;
const PROGRESS_FADE: f32 = 0.8;
/// Output this soon after a paste is the pasted text (seconds at speed 1).
const PASTE_LANDS: f32 = 0.4;
/// A jumping cursor's trail fades over this long; a still one starts
/// beating after PULSE_AFTER, once every PULSE_PERIOD (seconds at speed 1).
const TRAIL_TIME: f32 = 0.5;
const PULSE_AFTER: f32 = 2.0;
const PULSE_PERIOD: f32 = 1.4;
/// The idle scene is swept away this fast, at speed 1.
const SWEEP_TIME: f32 = 0.9;
/// Commands remembered per pane, for their marks.
const MAX_COMMANDS: usize = 4000;
/// A command's mark keeps moving this long after it is drawn in (drips,
/// dust), at speed 1.
const MARK_LINGER: f32 = 1.5;
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
    closing: f32,
    from_color: [f32; 4],
    grid_origin: [f32; 2],
    cell: [f32; 2],
    grid_size: [f32; 2],
    slot_size: [f32; 2],
    write_time: f32,
    atlas_columns: f32,
    dry_time: f32,
    aging: f32,
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

/// Everything sized by the window, rebuilt together.
struct Targets {
    /// Ink is drawn here, a pane at a time, before it is composited onto
    /// the surface.
    ink_view: wgpu::TextureView,
    /// The finished frame, before the present pass copies it to the window.
    scene: wgpu::Texture,
    scene_view: wgpu::TextureView,
    /// The frame a light/dark transition reveals the new look over.
    previous: wgpu::Texture,
    present_bind_group: wgpu::BindGroup,
}

/// Each pane's own: its grid, when each cell was written and with which
/// pen path (Rg32Float, for the ink stage), and what its screen showed, for
/// the writing animation.
struct PaneState {
    grid: (u32, u32),
    births: wgpu::Texture,
    ink_bind_group: wgpu::BindGroup,
    animator: TextAnimator,
    /// How far back into the scrollback it was shown last frame: scrolling
    /// through what is already written shows it as it is, not written in.
    offset: usize,
}

/// A divider between panes: which (stable while it exists), the line
/// between them (x, y, w, h; thin across), and whether it runs up and down.
pub struct Divider {
    pub id: u64,
    pub line: [f32; 4],
    pub upright: bool,
}

/// How wide the box a divider is drawn in is, logical pixels.
const DIVIDER_WIDTH: f32 = 14.0;

/// A pane to draw: its terminal, where it is in the window (physical
/// pixels), and whether it has the keyboard.
pub struct PaneView<'a, L: EventListener> {
    pub id: u64,
    pub term: &'a Term<L>,
    pub strikes: Option<&'a mut Strikes>,
    pub rect: [f32; 4],
    pub focused: bool,
}

impl Targets {
    fn new(device: &wgpu::Device, layouts: &Layouts, buffers: &Buffers, config: &wgpu::SurfaceConfiguration) -> Self {
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
        let ink = texture("ink layer", U::RENDER_ATTACHMENT | U::TEXTURE_BINDING);
        let scene = texture("scene", U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC);
        let previous = texture("previous frame", U::TEXTURE_BINDING | U::COPY_DST);
        let ink_view = ink.create_view(&Default::default());
        let scene_view = scene.create_view(&Default::default());
        let previous_view = previous.create_view(&Default::default());
        let view = wgpu::BindingResource::TextureView;
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
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
        Self { ink_view, scene, scene_view, previous, present_bind_group }
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

/// What the settings panel shows (settings.rs builds the rows).
pub struct SettingsPanel {
    pub items: Vec<Item>,
    /// The row the keyboard is on.
    pub focus: usize,
    /// Where the settings file is, as shown.
    pub path: String,
    /// Why the settings file could not be read, if it could not.
    pub error: Option<String>,
}

/// What the find bar shows.
#[derive(Clone)]
pub struct SearchBar {
    pub query: String,
    /// Text being composed in an input method, after the query.
    pub preedit: String,
    /// The current match and how many there are; None with nothing typed.
    pub count: Option<(usize, usize)>,
    /// The query is a regular expression (Cmd+R), and whether it parses.
    pub regex: bool,
    pub invalid: bool,
}

/// The settings panel as drawn: what it shows, how far it is scrolled, its
/// text, and where its controls were in the last frame (for clicks).
struct SettingsView {
    panel: SettingsPanel,
    scroll: f32,
    /// The row last scrolled into view, and the height it was shown in, so
    /// the keyboard's row is brought into view once (again if the panel's
    /// header grows), and scrolling by hand is not undone.
    shown_focus: Option<(usize, u32)>,
    texts: Vec<Buffer>,
    targets: Vec<([f32; 4], Hit)>,
    /// When it opened, for the theme's entrance, and when it began to close
    /// (the entrance played backwards).
    opened: f32,
    closing: Option<f32>,
    /// The panel's box, and the height of its scrolling part and of what
    /// is in it.
    bounds: [f32; 4],
    visible: f32,
    content: f32,
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
    /// How each pane's commands ended, by prompt number, with when that was
    /// heard (for drawing the mark in); and until when a mark is moving.
    commands: HashMap<u64, HashMap<u64, (Finished, f32)>>,
    marks_until: f32,
    mark_failures: bool,
    /// Grey backgrounds programs choose are drawn (as washes) instead of
    /// left out.
    grey_panels: bool,
    /// Commands that ran at least this long get a stamp; None: none do.
    stamp_after: Option<f32>,
    /// Seconds fresh ink looks wet (0: never), and how far old ink fades.
    ink_life: (f32, f32),
    /// The idle scene starts this many seconds after the last key or click
    /// (None: never); when that was; and, while it is swept away, since when
    /// and how far it had got.
    idle_after: Option<f32>,
    last_activity: f32,
    sweep: Option<(f32, f32)>,
    idle_until: f32,
    /// The window is going: its last transition shows nothing after.
    closing: bool,
    /// The theme's effects for closing and for clearing, on or off.
    effects: (bool, bool),
    /// Per pane, where its cursor was and is, and when it moved there; the
    /// cursor's trail and heartbeat, on or off; until when a trail fades.
    cursor_moves: HashMap<u64, ([f32; 4], [f32; 4], f32)>,
    cursor_life: (bool, bool),
    trail_until: f32,
    /// The focused pane's cursor is beating (the app wakes for it).
    pulsing: bool,
    /// The last paste: the pane, the cursor's cell then, how long the text
    /// was, and when; whether pastes splash at all.
    paste: Option<(u64, [f32; 4], f32, f32)>,
    splash: bool,
    splash_until: f32,
    /// Each pane's progress as last reported: the state, the share it is
    /// heading for, the share shown (eased towards it), and when it was
    /// cleared (it fades out); whether progress is shown at all.
    progress: HashMap<u64, (api::ProgressState, f32, f32, Option<f32>)>,
    show_progress: bool,
    progress_until: f32,
    /// What happened in the frames drawn since the app last asked (strikes,
    /// marks), for its sounds; and the newest strike and mark heard of.
    events: Vec<api::SoundEvent>,
    newest_strike: f32,
    newest_mark: f32,
    /// How worn the sheet on show is, 0..1, and the hours it takes to wear
    /// out (None: it never does).
    wear: f32,
    wear_hours: Option<f32>,
    /// Whether struck names fling blood.
    splatter: bool,
    /// Every pane's grid and writing state, by pane id, and the one being
    /// measured or drawn: its id, its box in the window, whether it has the
    /// keyboard.
    panes: HashMap<u64, PaneState>,
    pane: u64,
    pane_rect: [f32; 4],
    pane_focused: bool,
    /// The pane with the keyboard, as last drawn.
    focused_pane: u64,
    /// Drawn over every pane's ink with the last one: a veil over the
    /// panes without the keyboard.
    after_ink: Vec<Shape>,
    /// Drawn into the last pane's ink: the dividers.
    chrome_ink: Vec<Shape>,
    /// When each divider was first drawn, and until when one is still
    /// being drawn in.
    divider_births: HashMap<u64, f32>,
    dividers_until: f32,
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
    /// The settings panel, while it is open.
    /// How long the current theme's panel takes to come in, seconds at speed 1.
    settings_duration: f32,
    settings: Option<SettingsView>,
    settings_buffer: wgpu::Buffer,
    /// Installed monospace font families, sorted, once asked for.
    monospace: Option<Vec<String>>,
    /// Text being composed in an input method, and its selected range.
    preedit: Option<(String, Option<(usize, usize)>)>,
    preedit_text: Buffer,
    /// The cursor's cell in the last frame, physical pixels.
    cursor_cell: [f32; 4],
    /// The link under the pointer while Cmd is held: per row, its screen
    /// line, first column and length.
    link: Vec<(usize, usize, usize)>,
    /// The find bar while it is open, its text, and the matches in view.
    search: Option<SearchBar>,
    search_text: [Buffer; 2],
    search_marks: Vec<(usize, usize, usize, bool)>,

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
    /// The panes running a full-screen app, and each pane's shrink for it
    /// with the cell size that gives (fit, cell width, cell height). `fit`
    /// and the cell size are the pane being measured or drawn's.
    full_screen: HashSet<u64>,
    pane_metrics: HashMap<u64, (f32, f32, f32)>,
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
        let targets = Targets::new(&device, &layouts, &buffers, &config);
        let config_size = (config.width as f32, config.height as f32);

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
        let settings_buffer = Self::create_rect_buffer(&device, 256);
        let mut text = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
        text.borrow_with(&mut font_system).set_wrap(Wrap::None);
        let preedit_text = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
        let search_text = [Buffer::new(&mut font_system, Metrics::new(12.0, 16.0)), Buffer::new(&mut font_system, Metrics::new(12.0, 16.0))];
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
            commands: HashMap::new(),
            marks_until: f32::NEG_INFINITY,
            mark_failures: true,
            grey_panels: false,
            stamp_after: Some(10.0),
            ink_life: (4.0, 0.3),
            idle_after: Some(60.0),
            last_activity: 0.0,
            sweep: None,
            idle_until: f32::NEG_INFINITY,
            closing: false,
            effects: (true, true),
            cursor_moves: HashMap::new(),
            cursor_life: (true, true),
            trail_until: f32::NEG_INFINITY,
            pulsing: false,
            paste: None,
            splash: true,
            splash_until: f32::NEG_INFINITY,
            progress: HashMap::new(),
            show_progress: true,
            progress_until: f32::NEG_INFINITY,
            events: Vec::new(),
            newest_strike: f32::NEG_INFINITY,
            newest_mark: f32::NEG_INFINITY,
            wear: 0.0,
            wear_hours: Some(8.0),
            splatter: true,
            panes: HashMap::new(),
            pane: 0,
            pane_rect: [0.0, 0.0, config_size.0, config_size.1],
            pane_focused: true,
            focused_pane: 0,
            after_ink: Vec::new(),
            chrome_ink: Vec::new(),
            divider_births: HashMap::new(),
            dividers_until: f32::NEG_INFINITY,
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
            settings_duration: 0.25,
            settings: None,
            settings_buffer,
            monospace: None,
            preedit: None,
            preedit_text,
            cursor_cell: [0.0; 4],
            link: Vec::new(),
            search: None,
            search_text,
            search_marks: Vec::new(),
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
            full_screen: HashSet::new(),
            pane_metrics: HashMap::new(),
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

    /// Rebuilds the window-sized targets, and with them every pane's view
    /// of the ink layer.
    fn rebuild_targets(&mut self) {
        self.targets = Targets::new(&self.device, &self.layouts, &self.buffers, &self.config);
        let ids: Vec<u64> = self.panes.keys().copied().collect();
        for id in ids {
            let births = self.panes[&id].births.clone();
            let group = self.ink_bind_group(&births);
            if let Some(pane) = self.panes.get_mut(&id) {
                pane.ink_bind_group = group;
            }
        }
    }

    /// What the ink stage reads for a pane: the ink layer, and `births`.
    fn ink_bind_group(&self, births: &wgpu::Texture) -> wgpu::BindGroup {
        let births_view = births.create_view(&Default::default());
        let view = wgpu::BindingResource::TextureView;
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ink stage"),
            layout: &self.layouts.ink,
            entries: &[
                entry(0, self.buffers.frame.as_entire_binding()),
                entry(1, self.buffers.data.as_entire_binding()),
                entry(2, view(&self.targets.ink_view)),
                entry(3, wgpu::BindingResource::Sampler(&self.ink_sampler)),
                entry(4, view(&births_view)),
                entry(5, view(&self.trace_atlas_view)),
            ],
        })
    }

    /// Makes `id` the pane being measured, laid out and drawn, at `rect`,
    /// with a grid of `grid` cells (None: as it was). A new grid starts its
    /// writing over.
    fn set_pane(&mut self, id: u64, rect: [f32; 4], grid: Option<(u32, u32)>) {
        self.pane = id;
        self.pane_rect = rect;
        let current = self.panes.get(&id).map(|p| p.grid);
        let grid = match (grid, current) {
            (Some(grid), _) => grid,
            (None, Some(grid)) => grid,
            (None, None) => (1, 1),
        };
        if current == Some(grid) {
            return;
        }
        let births = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cell births"),
            size: wgpu::Extent3d { width: grid.0.max(1), height: grid.1.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let ink_bind_group = self.ink_bind_group(&births);
        self.panes.insert(id, PaneState { grid, births, ink_bind_group, animator: TextAnimator::default(), offset: 0 });
    }

    /// The pane being drawn's grid, columns and lines.
    fn grid(&self) -> (u32, u32) {
        self.panes.get(&self.pane).map_or((1, 1), |p| p.grid)
    }

    /// Lets go of a closed pane's state.
    pub fn forget_pane(&mut self, id: u64) {
        self.panes.remove(&id);
        self.commands.remove(&id);
        self.cursor_moves.remove(&id);
        self.progress.remove(&id);
        self.full_screen.remove(&id);
        self.pane_metrics.remove(&id);
        self.divider_births.remove(&id);
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

    /// Seconds without a key or a click before the idle scene starts; None:
    /// never.
    pub fn set_idle(&mut self, after: Option<f32>) {
        self.idle_after = after;
    }

    /// A key or a click: the idle scene, if it is showing, is swept away,
    /// and the wait starts over. Returns true if there is a sweep to draw.
    pub fn activity(&mut self) -> bool {
        let now = self.now();
        let elapsed = self.idle_elapsed(now);
        self.last_activity = now;
        match elapsed {
            Some(elapsed) if self.sweep.is_none() && self.animation.0 => {
                self.sweep = Some((now, elapsed));
                true
            }
            _ => false,
        }
    }

    /// Seconds the idle scene has been going, if it is.
    fn idle_elapsed(&self, now: f32) -> Option<f32> {
        let after = self.idle_after?;
        let since = now - self.last_activity - after;
        (since >= 0.0).then_some(since * self.animation.1)
    }

    /// Seconds until the idle scene is due (the app sleeps until then), if
    /// it is not already showing.
    pub fn idle_due_in(&self) -> Option<f32> {
        let after = self.idle_after?;
        let left = self.last_activity + after - self.now();
        (left > 0.0 && self.sweep.is_none()).then_some(left)
    }

    /// The idle scene's shapes for this frame (chrome ink), as the theme
    /// draws it: going, or being swept away.
    fn idle_shapes(&mut self, variant: &Variant, out: &mut Vec<Shape>) {
        let time = variant.theme.idle_time();
        if time <= 0.0 || self.thumbnail || !self.animation.0 {
            self.sweep = None;
            return;
        }
        let now = self.now();
        let (elapsed, sweep) = match self.sweep {
            Some((start, at)) => {
                let progress = (now - start) * self.animation.1 / SWEEP_TIME;
                if progress >= 1.0 {
                    self.sweep = None;
                    return;
                }
                self.idle_until = self.idle_until.max(start + SWEEP_TIME / self.animation.1);
                (at, progress)
            }
            None => match self.idle_elapsed(now) {
                Some(elapsed) => {
                    self.idle_until = self.idle_until.max(now + (time - elapsed).max(0.0) / self.animation.1);
                    (elapsed, 0.0)
                }
                None => return,
            },
        };
        let span = api::IdleSpan {
            area: self.window_rect(),
            elapsed,
            sweep,
            seed: (self.last_activity * 13.37).fract() * 97.0 + 1.0,
        };
        variant.theme.draw_idle(variant, &self.ctx(), &span, out);
    }

    /// How long fresh ink looks wet (seconds, 0: never) and how far ink an
    /// hour old fades (0..1).
    pub fn set_ink_life(&mut self, dry_time: f32, aging: f32) {
        self.ink_life = (dry_time, aging);
    }

    /// Which marks commands get at their prompts: failures beside them,
    /// and a stamp after the ones that ran at least `stamp_after` seconds.
    pub fn set_command_marks(&mut self, failures: bool, stamp_after: Option<f32>) {
        self.mark_failures = failures;
        self.stamp_after = stamp_after;
    }

    /// Command `n` in pane `pane` ended (shell integration).
    pub fn finish_command(&mut self, pane: u64, n: u64, finished: Finished) {
        let now = self.now();
        let commands = self.commands.entry(pane).or_default();
        commands.insert(n, (finished, now));
        // Old commands have scrolled far out of reach.
        if commands.len() > MAX_COMMANDS {
            commands.retain(|&k, _| k + MAX_COMMANDS as u64 / 2 > n);
        }
    }

    /// Whether grey backgrounds programs choose are drawn.
    pub fn set_grey_panels(&mut self, on: bool) {
        self.grey_panels = on;
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
            wear: self.wear,
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

    /// Starts the way `variant`'s theme closes a sheet, over the last frame
    /// shown: a tab (the next one is drawn beneath) or, with `window`, the
    /// window. Returns false if there is nothing to show (no animation, or
    /// the theme has none).
    pub fn start_close(&mut self, variant: &Variant, window: bool) -> bool {
        if !self.animation.0 || !self.effects.0 {
            return false;
        }
        let aspect = self.aspect();
        let Some(plan) = variant.theme.close(variant, &mut self.rng, aspect) else { return false };
        self.closing = window;
        self.start_over_last(plan);
        self.events.push(api::SoundEvent::Close);
        true
    }

    /// Starts the way `variant`'s theme clears the screen (Cmd+K), over the
    /// last frame shown.
    /// `pane` (at `rect`, physical pixels) is cleared: its old screen goes the
    /// theme's way, from where its cursor was. A clear while one is already
    /// going (Cmd+K makes the shell clear too) adds nothing.
    pub fn start_clear(&mut self, variant: &Variant, pane: u64, rect: [f32; 4]) {
        if !self.animation.0 || !self.effects.1 || !self.transition_done() {
            return;
        }
        let cursor = self.cursor_moves.get(&pane).map_or(self.cursor_cell, |m| m.1);
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let at = [(cursor[0] + cursor[2] / 2.0) / w, (cursor[1] + cursor[3] / 2.0) / h];
        let (aspect, area) = (self.aspect(), self.uv_rect(rect));
        if let Some(plan) = variant.theme.clear(variant, &mut self.rng, aspect, area, at) {
            self.start_over_last(plan);
            self.events.push(api::SoundEvent::Clear);
        }
    }

    /// The pane at `rect` (physical pixels) closes, the theme's way.
    pub fn start_pane_close(&mut self, variant: &Variant, rect: [f32; 4]) {
        if !self.animation.0 || !self.effects.0 {
            return;
        }
        let (aspect, area) = (self.aspect(), self.uv_rect(rect));
        if let Some(plan) = variant.theme.close_pane(variant, &mut self.rng, aspect, area) {
            self.start_over_last(plan);
            self.events.push(api::SoundEvent::Close);
        }
    }

    /// `rect` (physical pixels) as shares of the window.
    fn uv_rect(&self, [x, y, w, h]: [f32; 4]) -> [f32; 4] {
        let (ww, wh) = (self.config.width as f32, self.config.height as f32);
        [x / ww, y / wh, w / ww, h / wh]
    }

    /// Which of the theme's effects play: closing a tab or the window, and
    /// clearing the screen.
    pub fn set_effects(&mut self, close: bool, clear: bool) {
        self.effects = (close, clear);
    }

    /// Whether the cursor leaves a trail when it jumps, and beats while it
    /// is still.
    pub fn set_cursor_life(&mut self, trail: bool, pulse: bool) {
        self.cursor_life = (trail, pulse);
    }

    /// Whether programs' progress is drawn along the title strip.
    pub fn set_show_progress(&mut self, on: bool) {
        self.show_progress = on;
    }

    /// Pane `pane`'s program reported its progress.
    pub fn set_progress(&mut self, pane: u64, report: crate::marks::Progress) {
        use crate::marks::Progress;
        let now = self.now();
        let shown = self.progress.get(&pane).map_or(0.0, |p| p.2);
        let (state, share) = match report {
            Progress::Clear => {
                if let Some(p) = self.progress.get_mut(&pane) {
                    p.3.get_or_insert(now);
                    self.progress_until = self.progress_until.max(now + PROGRESS_FADE);
                }
                return;
            }
            Progress::Running(s) => (api::ProgressState::Running, s),
            Progress::Error(s) => (api::ProgressState::Error, s),
            Progress::Busy => (api::ProgressState::Busy, shown),
            Progress::Paused(s) => (api::ProgressState::Paused, s),
        };
        self.progress.insert(pane, (state, share, shown, None));
        self.progress_until = self.progress_until.max(now + PROGRESS_EASE * 4.0);
    }

    /// A program is busy with no idea how far along: the app wakes now and
    /// then to move its stroke.
    pub fn is_busy(&self) -> bool {
        self.show_progress && self.animation.0 && self.progress.values().any(|p| p.0 == api::ProgressState::Busy && p.3.is_none())
    }

    /// The focused pane's progress, as the theme draws it along the title
    /// strip (chrome ink).
    fn progress_shapes(&mut self, variant: &Variant, out: &mut Vec<Shape>) {
        if !self.show_progress || self.thumbnail {
            return;
        }
        let now = self.now();
        let pane = self.focused_pane;
        let Some(p) = self.progress.get_mut(&pane) else { return };
        // Ease the share shown towards the reported one.
        let k = if self.animation.0 { (1.0 / (PROGRESS_EASE * 60.0)).min(1.0) } else { 1.0 };
        p.2 += (p.1 - p.2) * k;
        let fade = p.3.map_or(1.0, |at| 1.0 - ((now - at) / PROGRESS_FADE).clamp(0.0, 1.0));
        if fade <= 0.0 {
            self.progress.remove(&pane);
            return;
        }
        let (state, share) = (p.0, p.2);
        let scale = self.window.scale_factor() as f32;
        let span = api::ProgressSpan {
            strip: [0.0, 0.0, self.config.width as f32, TITLE_STRIP * scale],
            share,
            state,
            time: now * self.animation.1,
            fade,
        };
        let ctx = self.ctx();
        variant.theme.draw_progress(variant, &ctx, &span, out);
    }

    /// What has happened since the last call, for sounds.
    pub fn take_events(&mut self) -> Vec<api::SoundEvent> {
        std::mem::take(&mut self.events)
    }

    /// Hours of use for a sheet to wear out; None: sheets never age.
    pub fn set_wear_hours(&mut self, hours: Option<f32>) {
        self.wear_hours = hours;
    }

    /// The tab on show has been open `seconds`: how worn its sheet is.
    pub fn set_sheet_age(&mut self, seconds: f32) {
        self.wear = self.wear_hours.map_or(0.0, |h| (seconds / (h * 3600.0)).clamp(0.0, 1.0));
    }

    /// Whether pasted text lands as a splash of ink.
    pub fn set_splash(&mut self, on: bool) {
        self.splash = on;
    }

    /// `chars` characters were pasted into the focused pane: they land at
    /// once, with the theme's splash.
    pub fn pasted(&mut self, chars: usize) {
        if !self.splash || !self.animation.0 {
            return;
        }
        self.paste = Some((self.focused_pane, self.cursor_cell, chars as f32, self.now()));
    }

    /// The cursor is beating: the app should wake now and then to draw it.
    pub fn is_pulsing(&self) -> bool {
        self.pulsing && self.window.has_focus()
    }

    /// Starts `plan` over the frame last shown.
    fn start_over_last(&mut self, plan: api::Plan) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(
            self.targets.scene.as_image_copy(),
            self.targets.previous.as_image_copy(),
            wgpu::Extent3d { width: self.config.width, height: self.config.height, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));
        self.start(plan, animation::From::PreviousFrame);
    }

    /// No transition is running (a closing window may go).
    pub fn transition_done(&self) -> bool {
        self.transition.as_ref().is_none_or(|t| t.done(self.now()))
    }

    /// Whether something is still moving, so the app keeps redrawing.
    pub fn is_animating(&self) -> bool {
        let now = self.now();
        let write_time = WRITE_TIME / self.animation.1;
        // Ink is wet for a few write times; it may keep settling for longer.
        let settle_time = self.settle_time.max(self.ink_life.0) / self.animation.1;
        self.transition.as_ref().is_some_and(|t| !t.done(now))
            || now < self.strikes_until
            || now < self.dividers_until
            || now < self.marks_until
            || now < self.idle_until
            || now < self.trail_until
            || now < self.splash_until
            || now < self.progress_until
            || self.settings.as_ref().is_some_and(|v| v.closing.is_some() || now < v.opened + SETTINGS_ENTRANCE_MAX / self.animation.1)
            || (self.animation.0
                && self.panes.values().any(|p| now - p.animator.latest_birth() < (write_time * 4.0).max(settle_time)))
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        configure_surface(&self.surface, &self.device, &self.config);
        self.rebuild_targets();
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

    /// The installed monospace font families, sorted, without the system's
    /// hidden ones.
    pub fn monospace_families(&mut self) -> Vec<String> {
        if let Some(families) = &self.monospace {
            return families.clone();
        }
        let mut families: Vec<String> = self
            .font_system
            .db()
            .faces()
            .filter(|face| face.monospaced)
            .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .filter(|name| !name.starts_with('.'))
            .collect();
        families.sort_by_key(|name| name.to_lowercase());
        families.dedup();
        self.monospace = Some(families.clone());
        families
    }

    /// The weights of `family`'s upright faces, lightest first.
    pub fn font_weights(&mut self, family: &str) -> Vec<u16> {
        let mut weights = self.family_info(family).map(|info| info.weights).unwrap_or_default();
        weights.sort_unstable();
        weights.dedup();
        weights
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
        for pane in self.panes.values_mut() {
            pane.animator = TextAnimator::default();
        }
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
    pub fn cell_at(&self, rect: [f32; 4], x: f64, y: f64, size: GridSize) -> (usize, usize, bool) {
        // The pane's own cells: a shrunk one's are smaller.
        let (pad, top) = self.origin(rect);
        let fx = ((x as f32 - pad) / size.cell_width).max(0.0);
        let fy = ((y as f32 - top) / size.cell_height).max(0.0);
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

    /// A full-screen app started or ended in pane `id`; returns true if
    /// the grid has to be recomputed.
    pub fn set_full_screen(&mut self, id: u64, on: bool) -> bool {
        if on { self.full_screen.insert(id) } else { self.full_screen.remove(&id) }
    }

    /// Makes pane `id`'s text size and cells the ones drawn with.
    fn use_pane_metrics(&mut self, id: u64) {
        if let Some(&(fit, width, height)) = self.pane_metrics.get(&id) {
            (self.fit, self.cell_width, self.cell_height) = (fit, width, height);
        }
    }

    /// Where the grid of the pane being drawn starts, physical pixels.
    fn padding(&self) -> (f32, f32) {
        self.origin(self.pane_rect)
    }

    /// Where the grid of a pane at `rect` starts.
    fn origin(&self, rect: [f32; 4]) -> (f32, f32) {
        let [left, top, _, _] = self.insets(rect);
        (rect[0] + left, rect[1] + top)
    }

    /// A pane's margins around its grid (left, top, right, bottom): the
    /// window's own at the window's edges (the title strip at the top), half
    /// as much beside another pane.
    fn insets(&self, rect: [f32; 4]) -> [f32; 4] {
        let scale = self.window.scale_factor() as f32;
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let (outer, inner) = (PADDING * scale, PADDING * 0.5 * scale);
        let at = |a: f32, b: f32| (a - b).abs() < 1.0;
        [
            if at(rect[0], 0.0) { outer } else { inner },
            if at(rect[1], 0.0) { (TITLE_STRIP + PADDING * 0.5) * scale } else { inner },
            if at(rect[0] + rect[2], w) { outer } else { inner },
            if at(rect[1] + rect[3], h) { outer } else { inner },
        ]
    }

    fn metrics(&self, variant: &Variant) -> Metrics {
        let scale = self.window.scale_factor() as f32 * self.zoom * self.fit;
        Metrics::new(variant.font_size * scale, (variant.line_height * scale).round())
    }

    /// How many cells fit pane `id` at `rect` (physical pixels). While a
    /// full-screen app runs in it, the pane's text shrinks (down to
    /// `MIN_FIT`) until its grid is at least `FULL_SCREEN_MIN`; the rest of
    /// the window keeps its size.
    pub fn grid_size(&mut self, variant: &Variant, id: u64, rect: [f32; 4]) -> GridSize {
        self.set_pane(id, rect, None);
        self.fit = 1.0;
        let fits = self.full_screen.contains(&id);
        let mut size = self.measure(variant);
        let (columns, lines) = FULL_SCREEN_MIN;
        // Cell size scales with the font, apart from rounding: aim for the
        // ratio, then step down a little until it fits.
        for _ in 0..12 {
            if !fits || (size.columns >= columns && size.lines >= lines) || self.fit <= MIN_FIT {
                break;
            }
            let ratio = (size.columns as f32 / columns as f32).min(size.lines as f32 / lines as f32);
            self.fit = (self.fit * ratio.min(0.97)).max(MIN_FIT);
            size = self.measure(variant);
        }
        self.pane_metrics.insert(id, (self.fit, self.cell_width, self.cell_height));
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

        let [left, top, right, bottom] = self.insets(self.pane_rect);
        let usable_w = self.pane_rect[2] - left - right;
        let usable_h = self.pane_rect[3] - top - bottom;
        let size = GridSize {
            columns: ((usable_w / self.cell_width) as usize).max(2),
            lines: ((usable_h / self.cell_height) as usize).max(1),
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        };
        let grid = (size.columns as u32, size.lines as u32);
        self.set_pane(self.pane, self.pane_rect, Some(grid));
        size
    }

    /// The whole window, physical pixels: the box of a lone pane.
    pub fn window_rect(&self) -> [f32; 4] {
        [0.0, 0.0, self.config.width as f32, self.config.height as f32]
    }

    /// Draws a frame: every pane, with `dividers` between them. A pane's
    /// `strikes`, when given, finds names of deleted files on its screen to
    /// strike out.
    pub fn render<L: EventListener>(&mut self, variant: &Variant, panes: &mut [PaneView<L>], dividers: &[Divider]) {
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
        self.draw_previews(panes, dividers);
        self.draw_panes(variant, panes, dividers, &view, true);
        self.draw_picker(variant, &view);
        self.draw_settings(variant, &view);
        self.draw_search(variant, &view);
        self.draw_preedit(variant, &view);
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

    /// Draws the panes into `view`, one after another onto the same surface,
    /// the window's own marks (controls, tabs, notice) and the dividers with
    /// the last; `strikes`: with names of deleted files struck out.
    fn draw_panes<L: EventListener>(
        &mut self,
        variant: &Variant,
        panes: &mut [PaneView<L>],
        dividers: &[Divider],
        view: &wgpu::TextureView,
        strikes: bool,
    ) {
        let n = panes.len();
        let scale = self.window.scale_factor() as f32;
        let reach = DIVIDER_WIDTH * scale / 2.0;
        // The panes without the keyboard are veiled a little, up to the
        // dividers (which stay as drawn).
        let mut after = Vec::new();
        if n > 1 {
            let (w, h) = (self.config.width as f32, self.config.height as f32);
            for pane in panes.iter().filter(|p| !p.focused) {
                let [x, y, pw, ph] = pane.rect;
                let inset = |inner: bool| if inner { reach } else { 0.0 };
                let (l, t) = (inset(x > 0.5), inset(y > 0.5));
                let (r, b) = (inset(x + pw < w - 0.5), inset(y + ph < h - 0.5));
                after.push(Shape::new(x + l, y + t, pw - l - r, ph - t - b, variant.surface_rgb(), 0.3));
            }
        }
        // The dividers, drawn by the theme into the last pane's ink, a new
        // one drawn in over the theme's time for it.
        let now = self.now();
        let ctx = self.ctx();
        let time = variant.theme.divider_time().max(1e-3) / self.animation.1;
        self.chrome_ink.clear();
        for divider in dividers {
            let born = *self.divider_births.entry(divider.id).or_insert(now);
            let progress = if self.animation.0 && !self.thumbnail { ((now - born) / time).clamp(0.0, 1.0) } else { 1.0 };
            self.dividers_until = self.dividers_until.max(born + time);
            let [x, y, w, h] = divider.line;
            let rect = if divider.upright {
                [x + w / 2.0 - reach, y, reach * 2.0, h]
            } else {
                [x, y + h / 2.0 - reach, w, reach * 2.0]
            };
            let span = api::DividerSpan { rect, upright: divider.upright, progress, seed: divider.id as f32 * 7.13 % 97.0 };
            let mut shapes = Vec::new();
            variant.theme.draw_divider(variant, &ctx, &span, &mut shapes);
            self.chrome_ink.extend(shapes);
        }
        let mut idle = Vec::new();
        self.idle_shapes(variant, &mut idle);
        self.progress_shapes(variant, &mut idle);
        self.chrome_ink.extend(idle);
        for (i, pane) in panes.iter_mut().enumerate() {
            let last = i + 1 == n;
            self.set_pane(pane.id, pane.rect, None);
            self.use_pane_metrics(pane.id);
            self.pane_focused = pane.focused;
            self.after_ink = if last { std::mem::take(&mut after) } else { Vec::new() };
            let struck = if strikes { pane.strikes.as_deref_mut() } else { None };
            self.layout(variant, pane.term, struck, last);
            self.draw(variant, view, i == 0, last);
        }
        self.after_ink.clear();
        // What is drawn after the panes (the input method's text) sits in
        // the focused one.
        if let Some(pane) = panes.iter().find(|p| p.focused).or(panes.first()) {
            self.set_pane(pane.id, pane.rect, None);
            self.use_pane_metrics(pane.id);
        }
    }

    /// Draws each picker card's picture: the screen as its variant draws it,
    /// with no writing in and no transition.
    fn draw_previews<L: EventListener>(&mut self, panes: &mut [PaneView<L>], dividers: &[Divider]) {
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
            self.draw_panes(&card.variant, panes, dividers, &view, false);
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

    /// Text being composed in an input method (Chinese, Japanese, accents
    /// typed as two keys, ...), with its selected range in bytes; None when
    /// there is none.
    pub fn set_preedit(&mut self, preedit: Option<(String, Option<(usize, usize)>)>) {
        self.preedit = preedit;
    }

    /// Shows the find bar (None closes it).
    pub fn set_search(&mut self, bar: Option<SearchBar>) {
        self.search = bar;
    }

    /// Highlights matches: per row, screen line, column, length, and
    /// whether it is the current one.
    pub fn set_search_marks(&mut self, marks: Vec<(usize, usize, usize, bool)>) {
        self.search_marks = marks;
    }

    /// The find bar, at the top right under the title strip: a patch of
    /// fresh surface with what is being looked for, a caret, and how many
    /// matches there are.
    fn draw_search(&mut self, variant: &Variant, view: &wgpu::TextureView) {
        let Some(bar) = self.search.clone() else { return };
        if self.picker.is_some() || self.settings_shown() {
            return;
        }
        let s = self.window.scale_factor() as f32;
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let bar_w = (340.0 * s).min(w - 32.0 * s).max(1.0);
        let bar_h = 32.0 * s;
        let (x, y) = (w - bar_w - 16.0 * s, TITLE_STRIP * s + 2.0 * s);
        let quiet = variant.palette[8];
        let family = variant.font_family.clone();
        let weight = self.weight_for(&family, variant.font_weight.min(500));
        let attrs = |rgb: [u8; 3]| {
            let [r, g, b] = rgb;
            Attrs::new().family(Family::Name(&family)).weight(Weight(weight)).color(Color::rgb(r, g, b))
        };
        let size = 12.5 * s;
        let line = (size * 1.35).round();
        let set = |buffer: &mut Buffer, fs: &mut FontSystem, spans: &[(&str, Attrs)]| {
            let mut b = buffer.borrow_with(fs);
            b.set_metrics(Metrics::new(size, line));
            b.set_wrap(Wrap::None);
            b.set_size(Some(w), Some(line * 2.0));
            b.set_rich_text(spans.iter().map(|(t, a)| (*t, a.clone())), &spans[0].1, Shaping::Advanced, None);
            b.shape_until_scroll(false);
            b.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
        };
        let fs = &mut self.font_system;
        let (count, count_rgb) = match bar.count {
            None if bar.regex => ("\u{21a9} older   \u{2318}R text".to_string(), quiet),
            None => ("\u{21a9} older   \u{2318}R regex".to_string(), quiet),
            Some(_) if bar.invalid => ("bad pattern".to_string(), variant.palette[1]),
            Some((_, 0)) => ("no matches".to_string(), variant.palette[1]),
            Some((i, n)) if n >= crate::search::MAX_MATCHES => (format!("{i} of {n}+"), quiet),
            Some((i, n)) => (format!("{i} of {n}"), quiet),
        };
        // A regular expression is marked like an editor's regex toggle.
        let count = if bar.regex && bar.count.is_some() { format!(".*  {count}") } else { count };
        let count_w = set(&mut self.search_text[1], fs, &[(&count, attrs(count_rgb))]);
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        if bar.query.is_empty() && bar.preedit.is_empty() {
            spans.push((if bar.regex { "Find .*" } else { "Find" }, attrs(quiet)));
        } else {
            spans.push((&bar.query, attrs(variant.ink)));
            if !bar.preedit.is_empty() {
                spans.push((&bar.preedit, attrs(quiet)));
            }
        }
        let typed = !(bar.query.is_empty() && bar.preedit.is_empty());
        let query_w = set(&mut self.search_text[0], fs, &spans);
        // A long query scrolls left so its end and the caret stay in view.
        let room = bar_w - count_w - 36.0 * s;
        let text_x = x + 12.0 * s - (query_w - room).max(0.0) * typed as u8 as f32;
        let text_y = y + (bar_h - line) / 2.0;
        let caret_x = if typed { text_x + query_w } else { text_x };

        let shapes = [
            Shape::new(x, y, bar_w, bar_h, variant.surface_rgb(), 0.3),
            Shape::rounded([x, y, bar_w, bar_h], 7.0 * s, 1.0 * s, quiet, 0.8),
            Shape::new(caret_x + 1.0 * s, text_y + 2.0 * s, (1.5 * s).max(1.0), line - 4.0 * s, variant.cursor, 1.0),
        ];
        let needed = (shapes.len() * std::mem::size_of::<Shape>()) as u64;
        self.queue.write_buffer(&self.settings_buffer, 0, bytemuck::cast_slice(&shapes));
        let clip = TextBounds {
            left: (x + 10.0 * s) as i32,
            top: y as i32,
            right: (x + 12.0 * s + room) as i32,
            bottom: (y + bar_h) as i32,
        };
        let areas = [
            TextArea { buffer: &self.search_text[0], left: text_x, top: text_y, scale: 1.0, bounds: clip, default_color: Color::rgb(0, 0, 0), custom_glyphs: &[] },
            TextArea {
                buffer: &self.search_text[1],
                left: x + bar_w - count_w - 12.0 * s,
                top: text_y,
                scale: 1.0,
                bounds: TextBounds { left: 0, top: 0, right: w as i32, bottom: h as i32 },
                default_color: Color::rgb(0, 0, 0),
                custom_glyphs: &[],
            },
        ];
        self.picker_text
            .prepare(&self.device, &self.queue, &mut self.font_system, &mut self.atlas, &self.viewport, areas, &mut self.swash_cache)
            .expect("prepare search text");
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("search") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("search"),
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
            let (px, py) = (x.max(0.0) as u32, y.max(0.0) as u32);
            let pw = (bar_w as u32).min(self.config.width.saturating_sub(px));
            let ph = (bar_h as u32).min(self.config.height.saturating_sub(py));
            pass.set_bind_group(0, &self.frame_group, &[]);
            if pw > 0 && ph > 0 {
                pass.set_scissor_rect(px, py, pw, ph);
                pass.set_pipeline(&stages.surface);
                pass.draw(0..3, 0..1);
                pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
            }
            pass.set_vertex_buffer(0, self.settings_buffer.slice(..needed));
            pass.set_pipeline(&stages.shapes);
            pass.draw(0..4, 0..shapes.len() as u32);
            self.picker_text.render(&self.atlas, &self.viewport, &mut pass).expect("render search text");
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// Underlines a link (runs of screen line, column, length), or none.
    pub fn set_link(&mut self, link: Vec<(usize, usize, usize)>) {
        self.link = link;
    }

    /// Where the input method's candidate window should go: the cursor's
    /// cell, physical pixels.
    pub fn ime_area(&self) -> [f32; 4] {
        self.cursor_cell
    }

    /// The text being composed, drawn at the cursor on a patch of fresh
    /// surface so it covers what is under it: underlined as macOS marks
    /// composing text, the part being converted more heavily, and a caret.
    fn draw_preedit(&mut self, variant: &Variant, view: &wgpu::TextureView) {
        let Some((text, selected)) = self.preedit.clone() else { return };
        if self.picker.is_some() || self.settings_shown() {
            return;
        }
        let s = self.window.scale_factor() as f32;
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let [cx, y, cw, ch] = self.cursor_cell;
        let metrics = self.metrics(variant);
        let weight = self.weight_for(&variant.font_family, variant.font_weight);
        let [r, g, b] = variant.ink;
        let attrs = Attrs::new().family(Family::Name(&variant.font_family)).weight(Weight(weight)).color(Color::rgb(r, g, b));
        let mut buffer = self.preedit_text.borrow_with(&mut self.font_system);
        buffer.set_metrics(metrics);
        buffer.set_wrap(Wrap::None);
        buffer.set_size(Some(w), Some(ch * 2.0));
        buffer.set_text(&text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(false);
        // Where a byte offset into the text falls, in pixels from its start.
        let mut text_w = 0.0f32;
        let mut edges: Vec<(usize, f32)> = Vec::new();
        for run in buffer.layout_runs() {
            text_w = text_w.max(run.line_w);
            edges.extend(run.glyphs.iter().map(|g| (g.start, g.x)));
        }
        let at = |byte: usize| edges.iter().find(|(start, _)| *start >= byte).map_or(text_w, |e| e.1);
        // Kept inside the window: long compositions move left.
        let pad = PADDING * s;
        let box_w = text_w.max(cw);
        let x = cx.min(w - pad - box_w).max(pad.min(cx));
        let line = (ch * 0.05).max(1.0 * s);
        let mut shapes = vec![Shape::new(x, y + ch * 0.9, text_w, line, variant.ink, 0.6)];
        match selected {
            Some((a, b)) if b > a => {
                let (a, b) = (at(a), at(b));
                shapes.push(Shape::new(x + a, y + ch * 0.9 - line, b - a, line * 2.5, variant.ink, 0.95));
            }
            Some((a, _)) => shapes.push(Shape::new(x + at(a), y + ch * 0.08, (1.5 * s).max(1.0), ch * 0.84, variant.cursor, 1.0)),
            None => {}
        }
        let needed = (shapes.len() * std::mem::size_of::<Shape>()) as u64;
        self.queue.write_buffer(&self.settings_buffer, 0, bytemuck::cast_slice(&shapes));
        let area = TextArea {
            buffer: &self.preedit_text,
            left: x,
            top: y,
            scale: 1.0,
            bounds: TextBounds { left: 0, top: 0, right: w as i32, bottom: h as i32 },
            default_color: Color::rgb(r, g, b),
            custom_glyphs: &[],
        };
        self.picker_text
            .prepare(&self.device, &self.queue, &mut self.font_system, &mut self.atlas, &self.viewport, [area], &mut self.swash_cache)
            .expect("prepare preedit text");
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("preedit") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preedit"),
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
            let (px, py) = ((x - 2.0 * s).max(0.0) as u32, y.max(0.0) as u32);
            let pw = ((box_w + 4.0 * s) as u32).min(self.config.width.saturating_sub(px));
            let ph = (ch as u32).min(self.config.height.saturating_sub(py));
            pass.set_bind_group(0, &self.frame_group, &[]);
            if pw > 0 && ph > 0 {
                pass.set_scissor_rect(px, py, pw, ph);
                pass.set_pipeline(&stages.surface);
                pass.draw(0..3, 0..1);
                pass.set_scissor_rect(0, 0, self.config.width, self.config.height);
            }
            pass.set_vertex_buffer(0, self.settings_buffer.slice(..needed));
            pass.set_pipeline(&stages.shapes);
            pass.draw(0..4, 0..shapes.len() as u32);
            self.picker_text.render(&self.atlas, &self.viewport, &mut pass).expect("render preedit text");
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// Opens the settings panel showing `panel`, updates it, or closes it
    /// (None). It stays scrolled where it was.
    pub fn set_settings(&mut self, panel: Option<SettingsPanel>) {
        let now = self.now();
        let Some(panel) = panel else {
            // It goes the way it came, backwards; at once without animation.
            match self.settings.as_mut() {
                Some(view) if self.animation.0 && view.closing.is_none() => view.closing = Some(now),
                Some(_) if self.animation.0 => {}
                _ => self.settings = None,
            }
            return;
        };
        // Opened again while going: it comes back from where it got to.
        let reopened = self.settings.as_ref().and_then(|v| v.closing.map(|c| (v.opened, c))).map(|(opened, closing)| {
            now - self.settings_progress(opened, Some(closing), now) * self.settings_duration_now()
        });
        match self.settings.as_mut() {
            Some(view) => {
                if let Some(opened) = reopened {
                    view.closing = None;
                    view.opened = opened;
                }
                view.panel = panel;
            }
            None => {
                self.settings = Some(SettingsView {
                    panel,
                    scroll: 0.0,
                    shown_focus: None,
                    texts: Vec::new(),
                    targets: Vec::new(),
                    opened: now,
                    closing: None,
                    bounds: [0.0; 4],
                    visible: 0.0,
                    content: 0.0,
                })
            }
        }
    }

    /// The panel is open (not gone, nor going).
    fn settings_shown(&self) -> bool {
        self.settings.as_ref().is_some_and(|v| v.closing.is_none())
    }

    /// Seconds (on the clock) the current theme's panel takes to come in.
    fn settings_duration_now(&self) -> f32 {
        self.settings_duration.max(0.05) / self.animation.1
    }

    /// How far in the panel is at `now`, 0..1 (before easing): coming in
    /// since `opened`, or going out since `closing`.
    fn settings_progress(&self, opened: f32, closing: Option<f32>, now: f32) -> f32 {
        if !self.animation.0 || self.thumbnail {
            return if closing.is_some() { 0.0 } else { 1.0 };
        }
        let d = self.settings_duration_now();
        match closing {
            // Going out from wherever it had got to coming in.
            Some(c) => (((c - opened) / d).clamp(0.0, 1.0) - (now - c) / d).clamp(0.0, 1.0),
            None => ((now - opened) / d).clamp(0.0, 1.0),
        }
    }

    /// What in the settings panel is at (x, y), physical pixels.
    pub fn settings_hit(&self, x: f32, y: f32) -> Option<Hit> {
        let view = self.settings.as_ref().filter(|v| v.closing.is_none())?;
        let inside = |[bx, by, bw, bh]: [f32; 4]| x >= bx && x < bx + bw && y >= by && y < by + bh;
        // Later targets sit on top of earlier ones (a row's control on the row).
        view.targets.iter().rev().find(|(rect, _)| inside(*rect)).map(|(_, hit)| *hit)
    }

    /// Whether (x, y) is on the settings panel.
    pub fn settings_contains(&self, x: f32, y: f32) -> bool {
        self.settings.as_ref().filter(|v| v.closing.is_none()).is_some_and(|v| {
            let [bx, by, bw, bh] = v.bounds;
            x >= bx && x < bx + bw && y >= by && y < by + bh
        })
    }

    /// Scrolls the settings panel by `pixels` (positive: further down).
    pub fn scroll_settings(&mut self, pixels: f32) {
        if let Some(view) = self.settings.as_mut() {
            view.scroll = (view.scroll + pixels).clamp(0.0, (view.content - view.visible).max(0.0));
        }
    }

    /// The settings panel down the right of the window: the current look's
    /// surface, a row per setting in its font and inks, the settings file's
    /// place at the top and the keys at the bottom. The rows scroll between
    /// those two.
    fn draw_settings(&mut self, variant: &Variant, view: &wgpu::TextureView) {
        let Some(mut panel) = self.settings.take() else { return };
        let s = self.window.scale_factor() as f32;
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let panel_w = (w * 0.46).clamp(400.0 * s, 560.0 * s).min(w);
        let left = w - panel_w;
        // The theme's entrance: how far in the panel is, eased.
        let (entrance, duration) = variant.theme.panel_entrance();
        self.settings_duration = duration.clamp(0.05, SETTINGS_ENTRANCE_MAX);
        let raw = self.settings_progress(panel.opened, panel.closing, self.now());
        // Gone back out: it is closed.
        if panel.closing.is_some() && raw <= 0.0 {
            return;
        }
        let eased = 1.0 - (1.0 - raw).powi(3);
        // A sliding panel moves; the others stay put and are uncovered.
        let dx = if entrance == api::Entrance::Slide { panel_w * (1.0 - eased) } else { 0.0 };
        let clip = match entrance {
            api::Entrance::Unroll => [left, 0.0, panel_w, (h * eased).max(1.0)],
            api::Entrance::Turn => [w - panel_w * eased, 0.0, (panel_w * eased).max(1.0), h],
            api::Entrance::Slide => [left + dx, 0.0, (panel_w - dx).max(1.0), h],
        };
        let edge = match entrance {
            api::Entrance::Unroll => h * eased,
            api::Entrance::Turn => w - panel_w * eased,
            api::Entrance::Slide => left + dx,
        };
        let pad = 28.0 * s;
        let (x0, width) = (left + pad, panel_w - 2.0 * pad);
        let (ink, accent, surface, quiet) = (variant.ink, variant.cursor, variant.surface_rgb(), variant.palette[8]);
        let family = variant.font_family.clone();
        let regular = self.weight_for(&family, variant.font_weight.min(500));
        let bold = self.weight_for(&family, 700);
        let attrs = |rgb: [u8; 3], weight: u16| {
            let [r, g, b] = rgb;
            Attrs::new().family(Family::Name(&family)).weight(Weight(weight)).color(Color::rgb(r, g, b))
        };

        // Text is laid out first, as its size decides where things go.
        // `put` shapes one piece into the next free buffer and returns it
        // with its width and height.
        let fs = &mut self.font_system;
        let texts = &mut panel.texts;
        let mut used = 0;
        let mut put = |spans: &[(&str, Attrs)], size: f32, wrap_width: Option<f32>| {
            if used == texts.len() {
                texts.push(Buffer::new(fs, Metrics::new(12.0, 16.0)));
            }
            let line = (size * 1.35 * s).round();
            let mut b = texts[used].borrow_with(fs);
            b.set_metrics(Metrics::new(size * s, line));
            b.set_wrap(if wrap_width.is_some() { Wrap::Word } else { Wrap::None });
            b.set_size(Some(wrap_width.unwrap_or(w)), None);
            b.set_rich_text(spans.iter().map(|(t, a)| (*t, a.clone())), &spans[0].1, Shaping::Advanced, None);
            b.shape_until_scroll(false);
            let (mut text_w, mut lines) = (0.0f32, 0);
            for run in b.layout_runs() {
                text_w = text_w.max(run.line_w);
                lines += 1;
            }
            used += 1;
            (used - 1, text_w, lines.max(1) as f32 * line)
        };
        // Pieces of text placed: buffer, corner, and whether it scrolls.
        let mut placed: Vec<(usize, f32, f32, bool)> = Vec::new();
        // The panel is a fresh piece of the surface (drawn in the pass
        // below), calmed a little so its words read.
        let mut fixed = vec![
            Shape::new(left, 0.0, panel_w, h, surface, 0.45),
            Shape::new(left, 0.0, (1.0 * s).max(1.0), h, quiet, 0.35),
        ];
        let mut targets = Vec::new();

        // The header: the title, the button that opens the file, and where
        // the file is.
        let mut y = TITLE_STRIP * s + 12.0 * s;
        let (title, _, title_h) = put(&[("Settings", attrs(ink, bold))], 20.0, None);
        placed.push((title, x0, y, false));
        let (open, open_w, open_h) = put(&[("Open the file", attrs(ink, regular))], 11.5, None);
        let button = [x0 + width - open_w - 20.0 * s, y + (title_h - open_h) / 2.0 - 4.0 * s, open_w + 20.0 * s, open_h + 8.0 * s];
        fixed.push(Shape::rounded(button, 5.0 * s, 1.0 * s, quiet, 0.8));
        placed.push((open, button[0] + 10.0 * s, button[1] + 4.0 * s, false));
        targets.push((button, Hit::OpenFile));
        y += title_h + 6.0 * s;
        let path = panel.panel.path.clone();
        let (info, _, info_h) = put(
            &[
                ("Saved in ", attrs(quiet, regular)),
                (&path, attrs(ink, regular)),
                (". Edit it directly if you prefer: what you save there shows up here.", attrs(quiet, regular)),
            ],
            11.5,
            Some(width),
        );
        placed.push((info, x0, y, false));
        y += info_h;
        if let Some(error) = panel.panel.error.clone() {
            y += 6.0 * s;
            let red = variant.palette[1];
            let text = format!("The file has a mistake, so it is not applied: {error}");
            let (e, _, e_h) = put(&[(&text, attrs(red, regular))], 11.5, Some(width));
            placed.push((e, x0, y, false));
            y += e_h;
        }
        y += 12.0 * s;
        fixed.push(Shape::new(x0, y - 1.0 * s, width, (1.0 * s).max(1.0), quiet, 0.3));
        let top = y;

        // The footer: the keys.
        let (keys, keys_w, keys_h) = put(
            &[("\u{2191}\u{2193} choose    \u{2190}\u{2192} change    \u{2318}O the file    esc close", attrs(quiet, regular))],
            11.0,
            None,
        );
        let bottom = h - keys_h - 22.0 * s;
        placed.push((keys, left + (panel_w - keys_w) / 2.0, h - keys_h - 14.0 * s, false));
        fixed.push(Shape::new(x0, bottom, width, (1.0 * s).max(1.0), quiet, 0.3));

        // The rows, from y = 0 at the top of the scrolling part.
        let control_w = (width * 0.48).min(230.0 * s);
        let control_h = 26.0 * s;
        let text_w = width - control_w - 18.0 * s;
        let cx = x0 + width - control_w;
        let mut scrolled = Vec::new();
        let mut row_targets: Vec<([f32; 4], Hit)> = Vec::new();
        let mut focus_box = None;
        let mut y = 4.0 * s;
        let focus = panel.panel.focus;
        for (index, item) in panel.panel.items.iter().enumerate() {
            match item {
                Item::Heading { title, note, fold } => {
                    if index > 0 {
                        y += if fold.is_some() { 8.0 } else { 18.0 } * s;
                    }
                    let heading_top = y;
                    let focused = index == focus;
                    // A section that folds has an arrow: pointing right while
                    // closed, down while open.
                    let arrow = match fold {
                        Some(true) => "\u{25be} ",
                        Some(false) => "\u{25b8} ",
                        None => "",
                    };
                    let shown = format!("{arrow}{title}");
                    let (t, t_w, t_h) = put(&[(&shown, attrs(if focused { accent } else { ink }, bold))], 12.5, None);
                    placed.push((t, x0, y, true));
                    y += t_h;
                    if fold.is_some() {
                        let box_ = [x0 - 10.0 * s, heading_top - 5.0 * s, width + 20.0 * s, t_h + 10.0 * s];
                        if focused {
                            scrolled.push(Shape::rounded(box_, 8.0 * s, 0.0, accent, 0.08));
                            focus_box = Some((box_[1], box_[1] + box_[3]));
                        }
                        row_targets.push((box_, Hit::Heading(index)));
                    }
                    // The note beside the heading, or under it if it does not fit.
                    if !note.is_empty() {
                        let (n, n_w, n_h) = put(&[(note, attrs(quiet, regular))], 11.0, Some(width));
                        if t_w + 10.0 * s + n_w <= width && n_h < t_h * 1.2 {
                            placed.push((n, x0 + t_w + 10.0 * s, y - t_h + (t_h - n_h) / 2.0 + 1.0 * s, true));
                        } else {
                            placed.push((n, x0, y + 1.0 * s, true));
                            y += n_h + 1.0 * s;
                        }
                    }
                    y += 4.0 * s;
                }
                Item::Row(row) => {
                    let focused = index == focus;
                    // The label and control on one line, what it does below.
                    let row_top = y;
                    let cy = row_top + 8.0 * s;
                    let (l, _, l_h) = put(&[(&row.label, attrs(if focused { accent } else { ink }, regular))], 13.0, Some(text_w));
                    placed.push((l, x0, cy + (control_h - l_h) / 2.0, true));
                    let mut row_h = control_h + 16.0 * s;
                    if !row.about.is_empty() {
                        let (a, _, a_h) = put(&[(&row.about, attrs(quiet, regular))], 11.0, Some(width));
                        placed.push((a, x0, cy + control_h + 3.0 * s, true));
                        row_h += a_h + 3.0 * s;
                    }
                    let row_box = [x0 - 10.0 * s, row_top, width + 20.0 * s, row_h];
                    if focused {
                        scrolled.push(Shape::rounded(row_box, 8.0 * s, 0.0, accent, 0.08));
                        focus_box = Some((row_top, row_top + row_h));
                    }
                    row_targets.push((row_box, Hit::Row(index, Part::Label)));
                    let line = (11.5 * 1.35 * s).round();
                    let text_y = cy + (control_h - line) / 2.0;
                    let control = [cx, cy, control_w, control_h];
                    // A few options show side by side, if their names fit.
                    let segmented = match &row.control {
                        Setting::Choice { labels, .. } if labels.len() <= settings::SEGMENTS => {
                            let segment = control_w / labels.len() as f32;
                            labels.iter().all(|l| put(&[(l, attrs(ink, regular))], 11.5, None).1 <= segment - 10.0 * s)
                        }
                        _ => false,
                    };
                    match &row.control {
                        Setting::Choice { labels, index: chosen, .. } if segmented => {
                            scrolled.push(Shape::rounded(control, 6.0 * s, 1.0 * s, quiet, 0.7));
                            let segment = control_w / labels.len() as f32;
                            for (i, text) in labels.iter().enumerate() {
                                let on = i == *chosen;
                                let sx = cx + i as f32 * segment;
                                if on {
                                    let b = [sx + 2.5 * s, cy + 2.5 * s, segment - 5.0 * s, control_h - 5.0 * s];
                                    scrolled.push(Shape::rounded(b, 4.5 * s, 0.0, ink, 0.9));
                                }
                                let (t, t_w, _) = put(&[(text, attrs(if on { surface } else { ink }, regular))], 11.5, None);
                                placed.push((t, sx + (segment - t_w) / 2.0, text_y, true));
                                row_targets.push(([sx, cy, segment, control_h], Hit::Row(index, Part::Option(i))));
                            }
                        }
                        Setting::Choice { .. } => {
                            scrolled.push(Shape::rounded(control, 6.0 * s, 1.0 * s, quiet, 0.7));
                            let arrow = 28.0 * s;
                            let (p, p_w, _) = put(&[("\u{2039}", attrs(ink, regular))], 13.0, None);
                            let (n, n_w, _) = put(&[("\u{203a}", attrs(ink, regular))], 13.0, None);
                            let arrow_y = cy + (control_h - (13.0 * 1.35 * s).round()) / 2.0;
                            placed.push((p, cx + (arrow - p_w) / 2.0, arrow_y, true));
                            placed.push((n, cx + control_w - (arrow + n_w) / 2.0, arrow_y, true));
                            let (v, v_w, _) = put(&[(&row.value_text(), attrs(ink, regular))], 11.5, None);
                            placed.push((v, cx + (control_w - v_w.min(control_w - 2.0 * arrow)) / 2.0, text_y, true));
                            row_targets.push((control, Hit::Row(index, Part::Next)));
                            row_targets.push(([cx, cy, arrow, control_h], Hit::Row(index, Part::Previous)));
                        }
                        &Setting::Toggle(on) => {
                            let (tw, th) = (38.0 * s, 22.0 * s);
                            let (tx, ty) = (cx + control_w - tw, cy + (control_h - th) / 2.0);
                            let knob = th / 2.0 - 3.5 * s;
                            if on {
                                scrolled.push(Shape::rounded([tx, ty, tw, th], th / 2.0, 0.0, accent, 1.0));
                                scrolled.push(Shape::round(DISC, tx + tw - th / 2.0, ty + th / 2.0, knob, surface, 1.0));
                            } else {
                                scrolled.push(Shape::rounded([tx, ty, tw, th], th / 2.0, 1.2 * s, quiet, 0.9));
                                scrolled.push(Shape::round(DISC, tx + th / 2.0, ty + th / 2.0, knob, quiet, 0.9));
                            }
                            row_targets.push((control, Hit::Row(index, Part::Toggle)));
                        }
                        &Setting::Slider { value, min, max, .. } => {
                            let value_w = 50.0 * s;
                            let track_w = control_w - value_w - 10.0 * s;
                            let along = ((value - min) / (max - min).max(1e-6)).clamp(0.0, 1.0);
                            let mid = cy + control_h / 2.0;
                            let track_h = 3.0 * s;
                            scrolled.push(Shape::rounded([cx, mid - track_h / 2.0, track_w, track_h], track_h / 2.0, 0.0, quiet, 0.45));
                            scrolled.push(Shape::rounded(
                                [cx, mid - track_h / 2.0, track_w * along, track_h],
                                track_h / 2.0,
                                0.0,
                                accent,
                                0.9,
                            ));
                            scrolled.push(Shape::round(DISC, cx + track_w * along, mid, 7.0 * s, accent, 1.0));
                            let (v, v_w, _) = put(&[(&row.value_text(), attrs(ink, regular))], 11.5, None);
                            placed.push((v, cx + control_w - v_w, text_y, true));
                            row_targets.push((
                                [cx - 10.0 * s, cy, track_w + 20.0 * s, control_h],
                                Hit::Row(index, Part::Slider(cx, track_w)),
                            ));
                        }
                    }
                    y += row_h;
                }
            }
        }
        let content = y + 12.0 * s;

        // Scroll: the keyboard's row into view when it moves, then keep
        // within the rows.
        let visible = bottom - top;
        if panel.shown_focus != Some((focus, visible as u32)) {
            if let Some((a, b)) = focus_box {
                if a < panel.scroll {
                    panel.scroll = a - 8.0 * s;
                } else if b > panel.scroll + visible {
                    panel.scroll = b - visible + 8.0 * s;
                }
            }
            panel.shown_focus = Some((focus, visible as u32));
        }
        panel.scroll = panel.scroll.clamp(0.0, (content - visible).max(0.0));
        let shift = top - panel.scroll;
        for shape in &mut scrolled {
            shape.rect[1] += shift;
        }
        if content > visible {
            let bar_h = (visible * visible / content).max(24.0 * s);
            let bar_y = top + (visible - bar_h) * panel.scroll / (content - visible);
            fixed.push(Shape::rounded([w - 7.0 * s, bar_y, 3.0 * s, bar_h], 1.5 * s, 0.0, quiet, 0.4));
        }
        // Click targets: only what can be seen, cut to the scrolling part.
        panel.targets = targets;
        for ([tx, ty, tw, th], hit) in row_targets {
            let (a, b) = ((ty + shift).max(top), (ty + th + shift).min(bottom));
            if b > a {
                panel.targets.push(([tx, a, tw, b - a], hit));
            }
        }
        panel.bounds = [left, 0.0, panel_w, h];
        (panel.visible, panel.content) = (visible, content);

        // Sliding in: everything moves with the panel. The theme draws its
        // edge (roller, curl, frame) over the rest.
        if dx > 0.0 {
            for shape in fixed.iter_mut().chain(scrolled.iter_mut()) {
                shape.rect[0] += dx;
            }
            for p in &mut placed {
                p.1 += dx;
            }
            for (rect, _) in &mut panel.targets {
                rect[0] += dx;
            }
        }
        let mut edges = Vec::new();
        if raw < 1.0 {
            let span = api::PanelSpan { panel: [left + dx, 0.0, panel_w, h], edge, progress: eased };
            variant.theme.draw_panel_edge(variant, &self.ctx(), &span, &mut edges);
        }

        let n_fixed = fixed.len() as u32;
        let n_body = n_fixed + scrolled.len() as u32;
        let shapes: Vec<Shape> = fixed.into_iter().chain(scrolled).chain(edges).collect();
        let needed = (shapes.len() * std::mem::size_of::<Shape>()) as u64;
        if needed > self.settings_buffer.size() {
            self.settings_buffer = Self::create_rect_buffer(&self.device, shapes.len().next_power_of_two());
        }
        self.queue.write_buffer(&self.settings_buffer, 0, bytemuck::cast_slice(&shapes));

        // Text only where the panel has come in so far.
        let [cx0, cy0, cw0, ch0] = clip;
        let all = TextBounds { left: cx0 as i32, top: cy0 as i32, right: (cx0 + cw0) as i32, bottom: (cy0 + ch0) as i32 };
        let rows = TextBounds { left: cx0 as i32, top: (top.max(cy0)) as i32, right: (cx0 + cw0) as i32, bottom: (bottom.min(cy0 + ch0)) as i32 };
        let areas = placed.iter().map(|&(i, x, y, scrolls)| TextArea {
            buffer: &panel.texts[i],
            left: x,
            top: if scrolls { y + shift } else { y },
            scale: 1.0,
            bounds: if scrolls { rows } else { all },
            default_color: Color::rgb(0, 0, 0),
            custom_glyphs: &[],
        });
        self.picker_text
            .prepare(&self.device, &self.queue, &mut self.font_system, &mut self.atlas, &self.viewport, areas, &mut self.swash_cache)
            .expect("prepare settings text");

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("settings") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("settings"),
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
            let (fw, fh) = (self.config.width, self.config.height);
            // Scissor to `r` within the window (and within the panel as it has
            // come in so far); false if nothing of it is left.
            let scissor = |pass: &mut wgpu::RenderPass, r: [f32; 4]| -> bool {
                let x = r[0].max(clip[0]).max(0.0);
                let y = r[1].max(clip[1]).max(0.0);
                let x1 = (r[0] + r[2]).min(clip[0] + clip[2]).min(fw as f32);
                let y1 = (r[1] + r[3]).min(clip[1] + clip[3]).min(fh as f32);
                if x1 - x < 1.0 || y1 - y < 1.0 {
                    return false;
                }
                pass.set_scissor_rect(x as u32, y as u32, (x1 - x) as u32, (y1 - y) as u32);
                true
            };
            pass.set_bind_group(0, &self.frame_group, &[]);
            if scissor(&mut pass, [left + dx, 0.0, panel_w, h]) {
                pass.set_pipeline(&stages.surface);
                pass.draw(0..3, 0..1);
            }
            pass.set_vertex_buffer(0, self.settings_buffer.slice(..needed));
            pass.set_pipeline(&stages.shapes);
            if scissor(&mut pass, [0.0, 0.0, w, h]) {
                pass.draw(0..4, 0..n_fixed);
            }
            if scissor(&mut pass, [left + dx, top, panel_w, bottom - top]) {
                pass.draw(0..4, n_fixed..n_body);
            }
            pass.set_scissor_rect(0, 0, fw, fh);
            self.picker_text.render(&self.atlas, &self.viewport, &mut pass).expect("render settings text");
            // The edge goes over the text; drawing text changed the pipeline.
            if n_body < shapes.len() as u32 {
                pass.set_bind_group(0, &self.frame_group, &[]);
                pass.set_vertex_buffer(0, self.settings_buffer.slice(..needed));
                pass.set_pipeline(&stages.shapes);
                pass.draw(0..4, n_body..shapes.len() as u32);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        self.settings = Some(panel);
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
        let mut panes = [PaneView { id: self.pane, term, strikes: None, rect: self.pane_rect, focused: true }];
        self.chrome_ink.clear();
        self.draw_panes(variant, &mut panes, &[], &texture.create_view(&Default::default()), false);

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
        let (columns, lines) = (self.grid().0 as usize, self.grid().1 as usize);
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
                if s.born > self.newest_strike && start <= now {
                    self.newest_strike = s.born;
                    self.events.push(api::SoundEvent::Strike);
                }
                (s, progress, age)
            })
            .filter(|&(_, progress, _)| progress > 0.0)
            .collect();
        (found, style.fade)
    }

    /// Turns the visible part of the grid into styled text runs plus
    /// rectangles for backgrounds, decorations and the cursor.
    /// Lays out the pane being drawn; `chrome`: also the window's own
    /// marks (controls, notice, tabs).
    fn layout<L: EventListener>(&mut self, variant: &Variant, term: &Term<L>, strikes: Option<&mut Strikes>, chrome: bool) {
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
        let (columns, lines) = (self.grid().0 as usize, self.grid().1 as usize);
        let mut chars = vec![' '; columns * lines];
        let mut slots = vec![0.0f32; columns * lines];
        // A shrunk pane (a full-screen app squeezed into it) writes with the
        // plain sweep: the pen-path atlas is for the window's own cell size,
        // and such apps redraw too often for the pen to show.
        let shrunk = self.fit < 1.0;
        if !self.thumbnail && !shrunk {
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
        // Prompts on screen (shell integration): each command's number, with
        // the last line of its prompt and the column after the typed text.
        let mut prompts: HashMap<u64, (usize, usize)> = HashMap::new();
        for indexed in cells {
            let line = (indexed.point.line.0 + content.display_offset as i32) as usize;
            let col = indexed.point.column.0;
            let flags = indexed.cell.flags;
            if !full_screen
                && line < lines
                && let Some(n) = indexed.cell.hyperlink().and_then(|h| marks::prompt_number(h.uri()))
            {
                let at = prompts.entry(n).or_insert((line, 0));
                if line > at.0 {
                    *at = (line, 0);
                }
                if line == at.0 && indexed.cell.c != ' ' {
                    at.1 = at.1.max(col + 1);
                }
            }
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            if line < lines && col < columns && !flags.contains(Flags::HIDDEN) {
                let c = cell_char(indexed.cell.c);
                chars[line * columns + col] = c;
                if self.animation.0 && !self.thumbnail && !shrunk && c != ' ' && boxdraw::pieces(c, 1.0, 1.0).is_none() {
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
                // A grey band a program lays behind text (Claude Code's
                // messages) is for a plain screen; here the paper is it.
                if program_chosen(indexed.cell.bg) && !flags.contains(Flags::INVERSE) && !self.grey_panels && bg.is_some_and(color::is_neutral) {
                    bg = None;
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

        // Find's matches, on the surface under the text: the current one in
        // the cursor's colour. Find, links and input look at the pane with
        // the keyboard.
        if !self.thumbnail && self.pane_focused {
            for &(line, col, len, current) in &self.search_marks {
                let (rgb, alpha) = if current { (variant.cursor, 0.45) } else { (variant.selection.0, 0.18) };
                self.rects.push(Shape::new(cell_x(col), cell_y(line), cw * len as f32, ch, rgb, alpha));
            }
        }

        // The cursor and struck-out names are the theme's to draw, in the
        // ink layer; the cursor goes under the text.
        let ctx = self.ctx();
        if cursor_line >= 0 && !self.thumbnail && self.pane_focused {
            self.cursor_cell = [cell_x(cursor_col), cell_y(cursor_line as usize), cw, ch];
            self.focused_pane = self.pane;
        }
        if cursor_line >= 0 && cursor_shape != CursorShape::Hidden {
            let kind = match cursor_shape {
                CursorShape::Beam => CursorKind::Beam,
                CursorShape::Underline => CursorKind::Underline,
                CursorShape::HollowBlock => CursorKind::Hollow,
                _ => CursorKind::Block,
            };
            let cell = [cell_x(cursor_col), cell_y(cursor_line as usize), cw, ch];
            let now = self.now();
            let moved = self.cursor_moves.entry(self.pane).or_insert((cell, cell, f32::NEG_INFINITY));
            if moved.1 != cell {
                *moved = (moved.1, cell, now);
            }
            let (from, _, at) = *moved;
            let since = (now - at) * self.animation.1;
            // A step (typing, an arrow key) leaves no trail; a jump does.
            let jumped = (from[1] - cell[1]).abs() > ch * 0.5 || (from[0] - cell[0]).abs() > cw * 2.5;
            let trail = (self.cursor_life.0 && self.animation.0 && !self.thumbnail && jumped && since < TRAIL_TIME)
                .then(|| (from, since / TRAIL_TIME));
            if trail.is_some() {
                self.trail_until = self.trail_until.max(at + TRAIL_TIME / self.animation.1);
            }
            let beats = self.cursor_life.1 && self.animation.0 && !self.thumbnail && self.pane_focused && since > PULSE_AFTER;
            if self.pane_focused {
                self.pulsing = beats;
            }
            let pulse = if beats {
                // A heartbeat: two quick beats, then rest.
                let t = ((since - PULSE_AFTER) / PULSE_PERIOD).fract();
                let beat = |c: f32| (-((t - c) * 22.0).powi(2)).exp();
                (beat(0.12) + 0.6 * beat(0.3)).min(1.0)
            } else {
                0.0
            };
            let cursor = Cursor { cell, column: cursor_col, line: cursor_line as usize, kind, trail, pulse };
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
        // The link Cmd+click would open, underlined in the cursor's colour.
        if !self.thumbnail && self.pane_focused {
            for &(line, col, len) in &self.link {
                let y = cell_y(line) + ch * 0.88;
                overlay.push(Shape::new(cell_x(col), y, cw * len as f32, (ch * 0.07).max(1.0), variant.cursor, 1.0));
            }
        }
        // A paste's splash, where it landed.
        if let Some((pane, cell, chars, at)) = self.paste
            && pane == self.pane
        {
            let time = variant.theme.splash_time();
            let age = (self.now() - at) * self.animation.1;
            if age < time {
                self.splash_until = self.splash_until.max(at + time / self.animation.1);
                let span = api::SplashSpan { cell, reach: chars.min(columns as f32 * 3.0), age, seed: (at * 7.31).fract() * 89.0 + 1.0 };
                variant.theme.draw_splash(variant, &ctx, &span, &mut overlay);
            }
        }
        // Commands marked at their prompts, as the theme marks them: a
        // failure beside it, a stamp after a long one.
        if let Some(commands) = self.commands.get(&self.pane) {
            let now = self.now();
            let speed = self.animation.1;
            let margin_x = self.pane_rect[0];
            let mut marks_until = self.marks_until;
            let timing = |heard: f32, time: f32, until: &mut f32| {
                let time = time.max(1e-3) / speed;
                *until = until.max(heard + time + MARK_LINGER / speed);
                if self.animation.0 {
                    (((now - heard) / time).clamp(0.0, 1.0), (now - heard) * speed)
                } else {
                    (1.0, f32::INFINITY)
                }
            };
            for (&n, &(line, end)) in &prompts {
                let Some(&(finished, heard)) = commands.get(&n) else { continue };
                let failed = finished.failed();
                let stamped = self.stamp_after.is_some_and(|after| finished.seconds >= after);
                let mut span = CommandSpan {
                    margin: [margin_x, cell_y(line), (pad - margin_x).max(1.0), ch],
                    after: [cell_x(end.min(columns.saturating_sub(1))), cell_y(line), cw, ch],
                    failed,
                    progress: 1.0,
                    age: f32::INFINITY,
                    seed: (n as f32 * 7.77) % 101.0,
                };
                if heard > self.newest_mark && (failed && self.mark_failures || stamped) {
                    self.events.push(if stamped { api::SoundEvent::Stamp } else { api::SoundEvent::Failure });
                }
                if failed && self.mark_failures {
                    (span.progress, span.age) = timing(heard, variant.theme.mark_time(), &mut marks_until);
                    variant.theme.draw_failure(variant, &ctx, &span, &mut overlay);
                }
                if stamped {
                    (span.progress, span.age) = timing(heard, variant.theme.stamp_time(), &mut marks_until);
                    variant.theme.draw_stamp(variant, &ctx, &span, &mut overlay);
                }
            }
            self.marks_until = marks_until;
            if let Some(newest) = commands.values().map(|&(_, heard)| heard).reduce(f32::max) {
                self.newest_mark = self.newest_mark.max(newest);
            }
        }
        self.ink_rects.extend(overlay);
        if chrome {
            self.ink_rects.extend_from_slice(&self.chrome_ink);
            self.layout_controls(variant, self.window.has_focus());
            self.layout_notice(variant);
            self.layout_tabs(variant);
        }

        let now = self.now();
        let offset = term.grid().display_offset();
        let scrolled = self.panes.get(&self.pane).is_some_and(|p| p.offset != offset);
        if let Some(pane) = self.panes.get_mut(&self.pane) {
            pane.offset = offset;
        }
        let writing = Writing {
            enabled: self.animation.0,
            speed: self.writing_speed,
            max_duration: MAX_WRITE / self.animation.1,
            // Full-screen apps, and text scrolled into view from the
            // scrollback, appear as they are.
            instant: term.mode().contains(TermMode::ALT_SCREEN) || scrolled,
            // Text pasted in the last moment lands together, as one splash.
            together: self.paste.is_some_and(|(pane, _, _, at)| pane == self.pane && (now - at) * self.animation.1 < PASTE_LANDS),
        };
        // A preview shows the screen as it is: no writing in, and it leaves
        // the live screen's record of what was written alone.
        let Some(pane) = self.panes.get_mut(&self.pane) else { return };
        let births = if self.thumbnail { Vec::new() } else { pane.animator.update(columns, &chars, now, writing) };
        if births.len() == columns * lines {
            let cells: Vec<[f32; 2]> = births.iter().zip(&slots).map(|(&b, &s)| [b, s]).collect();
            self.queue.write_texture(
                pane.births.as_image_copy(),
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
        let pad = PADDING * scale;
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

    /// Draws the pane laid out last into the scene: the `first` pane lays
    /// the surface under all of them, the `last` adds the window's marks and
    /// shows the scene in `view`, through any transition.
    fn draw(&mut self, variant: &Variant, view: &wgpu::TextureView, first: bool, last: bool) {
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
            closing: if self.closing { 1.0 } else { 0.0 },
            from_color,
            grid_origin: [pad, top],
            cell: [self.cell_width, self.cell_height],
            grid_size: [self.grid().0 as f32, self.grid().1 as f32],
            slot_size: [self.tracer.slot_size.0 as f32, self.tracer.slot_size.1 as f32],
            write_time: if self.animation.0 && !self.thumbnail { WRITE_TIME / self.animation.1 } else { 0.0 },
            atlas_columns: self.tracer.columns() as f32,
            dry_time: if self.animation.0 && !self.thumbnail { self.ink_life.0 / self.animation.1 } else { 0.0 },
            aging: if self.thumbnail { 0.0 } else { self.ink_life.1 },
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

        // The rect lists share one buffer: surface rects, ink rects, then
        // what goes over the ink.
        let surface_count = self.rects.len() as u32;
        let ink_end = (self.rects.len() + self.ink_rects.len()) as u32;
        let total = self.rects.len() + self.ink_rects.len() + self.after_ink.len();
        let needed = (total * std::mem::size_of::<Shape>()) as u64;
        if needed > self.rect_buffer.size() {
            self.rect_buffer = Self::create_rect_buffer(&self.device, total.next_power_of_two());
        }
        if total > 0 {
            let all: Vec<Shape> = self.rects.iter().chain(&self.ink_rects).chain(&self.after_ink).copied().collect();
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
        for (slot, buffer) in self.tab_slots.iter().zip(&self.tab_text).filter(|_| last) {
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
        if self.notice.is_some() && last {
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
                pass.draw(0..4, surface_count..ink_end);
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
                    if first { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load },
                )],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if first {
                pass.set_pipeline(&stages.surface);
                pass.set_bind_group(0, &self.frame_group, &[]);
                pass.draw(0..3, 0..1);
            }

            if surface_count > 0 {
                pass.set_pipeline(&stages.shapes);
                pass.set_bind_group(0, &self.frame_group, &[]);
                pass.set_vertex_buffer(0, self.rect_buffer.slice(..needed));
                pass.draw(0..4, 0..surface_count);
            }

            // Each pane's ink layer holds only its own ink, so it can be
            // laid over the whole window.
            if let Some(pane) = self.panes.get(&self.pane) {
                pass.set_pipeline(&stages.ink);
                pass.set_bind_group(0, &pane.ink_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            if !self.after_ink.is_empty() {
                pass.set_pipeline(&stages.shapes);
                pass.set_bind_group(0, &self.frame_group, &[]);
                pass.set_vertex_buffer(0, self.rect_buffer.slice(..needed));
                pass.draw(0..4, ink_end..total as u32);
            }
        }
        if !last {
            self.queue.submit(Some(encoder.finish()));
            return;
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
///
/// While the window is being resized, macOS shows the last frame until the
/// next one arrives, stretched to the new size by default: the text would
/// swell and snap back with every step. Each frame is presented in the same
/// Core Animation transaction as the window's change of size (the app draws
/// one as soon as it is resized), so the two arrive together; and a frame
/// shown in a window of another size stays pinned to the top-left corner,
/// where the grid and the window's controls are.
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
            let layer = hal.render_layer().lock();
            layer.setColorspace(srgb.as_deref());
            // The layer's contents are flipped (the view's origin is its top
            // left), so "bottom left" is the top left on screen.
            layer.setContentsGravity(objc2_quartz_core::kCAGravityBottomLeft);
            layer.setPresentsWithTransaction(true);
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
