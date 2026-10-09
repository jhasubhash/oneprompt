//! GPU side. Each frame is three layers in one pass:
//! 1. the procedural background (paper, chalkboard, ...),
//! 2. solid rectangles for cell backgrounds, underlines and the cursor,
//! 3. the terminal grid as text, via glyphon.

use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping,
    Style, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};
use winit::window::Window;

use crate::terminal::GridSize;
use crate::theme::Theme;

/// Margin around the grid, in logical pixels.
const PADDING: f32 = 24.0;
/// Opacity of coloured cell backgrounds, so the texture still shows through.
const CELL_BG_ALPHA: f32 = 0.85;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    surface: [f32; 4],
    surface_shade: [f32; 4],
    resolution: [f32; 2],
    time: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RectGlobals {
    resolution: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Rect {
    rect: [f32; 4],
    color: [f32; 4],
}

impl Rect {
    fn new(x: f32, y: f32, w: f32, h: f32, rgb: [u8; 3], alpha: f32) -> Self {
        let [r, g, b] = rgb.map(|c| c as f32 / 255.0);
        Self { rect: [x, y, w, h], color: [r, g, b, alpha] }
    }
}

pub struct Renderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    background: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    started: Instant,

    rect_pipeline: wgpu::RenderPipeline,
    rect_globals: wgpu::Buffer,
    rect_bind_group: wgpu::BindGroup,
    rect_buffer: wgpu::Buffer,
    rects: Vec<Rect>,

    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    text: Buffer,

    /// Text size multiplier (Cmd + / Cmd -).
    zoom: f32,
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
        surface.configure(&device, &config);

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_buffer = |label, size: usize| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &uniform_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            (buffer, bind_group)
        };
        let (uniforms, bind_group) =
            uniform_buffer("background uniforms", std::mem::size_of::<Uniforms>());
        let (rect_globals, rect_bind_group) =
            uniform_buffer("rect globals", std::mem::size_of::<RectGlobals>());

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline layout"),
            bind_group_layouts: &[Some(&uniform_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders/background.wgsl"));
        let background = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("background"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(format.into())],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let rect_shader = device.create_shader_module(wgpu::include_wgsl!("shaders/rect.wgsl"));
        let rect_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rects"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &rect_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Rect>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &rect_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let rect_buffer = Self::create_rect_buffer(&device, 1024);

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::with_color_mode(&device, &queue, &cache, format, ColorMode::Web);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let mut text = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
        text.borrow_with(&mut font_system).set_wrap(Wrap::None);

        Self {
            window,
            surface,
            device,
            queue,
            config,
            background,
            uniforms,
            bind_group,
            started: Instant::now(),
            rect_pipeline,
            rect_globals,
            rect_bind_group,
            rect_buffer,
            rects: Vec::new(),
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            text,
            zoom: 1.0,
            cell_width: 8.0,
            cell_height: 16.0,
        }
    }

    fn create_rect_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rect instances"),
            size: (capacity * std::mem::size_of::<Rect>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn cell_height(&self) -> f32 {
        self.cell_height
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(0.5, 3.0);
    }

    fn padding(&self) -> f32 {
        PADDING * self.window.scale_factor() as f32
    }

    fn metrics(&self, theme: &Theme) -> Metrics {
        let scale = self.window.scale_factor() as f32 * self.zoom;
        Metrics::new(theme.font_size * scale, (theme.line_height * scale).round())
    }

    /// Measures the theme's font and returns how many cells fit the window.
    pub fn grid_size(&mut self, theme: &Theme) -> GridSize {
        let metrics = self.metrics(theme);
        let mut probe = Buffer::new(&mut self.font_system, metrics);
        {
            let mut probe = probe.borrow_with(&mut self.font_system);
            probe.set_wrap(Wrap::None);
            let attrs = Attrs::new().family(Family::Name(theme.font_family));
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

        let pad = self.padding();
        let usable_w = self.config.width as f32 - 2.0 * pad;
        let usable_h = self.config.height as f32 - 2.0 * pad;
        GridSize {
            columns: ((usable_w / self.cell_width) as usize).max(2),
            lines: ((usable_h / self.cell_height) as usize).max(1),
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }

    pub fn render<L: EventListener>(&mut self, theme: &Theme, term: &Term<L>) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            // Timeout, occluded window, ...: skip this frame.
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());
        self.layout(theme, term);
        self.draw(theme, &view);
        self.queue.present(frame);
        self.atlas.trim();
    }

    /// Renders one frame off screen and saves it as a PNG. Used by
    /// `ONEPROMPT_SNAPSHOT` to preview themes without screen-capture access.
    pub fn snapshot<L: EventListener>(&mut self, theme: &Theme, term: &Term<L>, path: &std::path::Path) {
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
        self.layout(theme, term);
        self.draw(theme, &texture.create_view(&Default::default()));

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

    /// Turns the visible part of the grid into styled text runs plus
    /// rectangles for backgrounds, decorations and the cursor.
    fn layout<L: EventListener>(&mut self, theme: &Theme, term: &Term<L>) {
        let content = term.renderable_content();
        let pad = self.padding();
        let (cw, ch) = (self.cell_width, self.cell_height);
        let cell_x = |col: usize| pad + col as f32 * cw;
        let cell_y = |line: usize| pad + line as f32 * ch;

        let cursor = content.cursor;
        let cursor_line = cursor.point.line.0 + content.display_offset as i32;
        let cursor_col = cursor.point.column.0;
        let block_cursor = cursor.shape == CursorShape::Block && cursor_line >= 0;

        // Collect runs of identically styled text: (line, text, color, flags).
        let mut runs: Vec<(usize, String, [u8; 3], Flags)> = Vec::new();
        self.rects.clear();
        for indexed in content.display_iter {
            let line = (indexed.point.line.0 + content.display_offset as i32) as usize;
            let col = indexed.point.column.0;
            let flags = indexed.cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }

            let mut fg = resolve(theme, indexed.cell.fg, theme.ink);
            let mut bg = match indexed.cell.bg {
                AnsiColor::Named(NamedColor::Background) => None,
                color => Some(resolve(theme, color, theme.surface_rgb())),
            };
            if flags.contains(Flags::INVERSE) {
                let old_bg = bg.unwrap_or(theme.surface_rgb());
                bg = Some(fg);
                fg = old_bg;
            }
            if flags.contains(Flags::DIM) {
                fg = mix(fg, bg.unwrap_or(theme.surface_rgb()), 0.4);
            }
            let is_cursor = block_cursor && line as i32 == cursor_line && col == cursor_col;
            if is_cursor {
                fg = bg.unwrap_or(theme.surface_rgb());
            }

            let width = if flags.contains(Flags::WIDE_CHAR) { 2.0 } else { 1.0 };
            if let Some(bg) = bg {
                self.rects.push(Rect::new(cell_x(col), cell_y(line), cw * width, ch, bg, CELL_BG_ALPHA));
            }
            if flags.contains(Flags::UNDERLINE) {
                let y = cell_y(line) + ch * 0.86;
                self.rects.push(Rect::new(cell_x(col), y, cw * width, (ch * 0.06).max(1.0), fg, 1.0));
            }
            if flags.contains(Flags::STRIKEOUT) {
                let y = cell_y(line) + ch * 0.5;
                self.rects.push(Rect::new(cell_x(col), y, cw * width, (ch * 0.06).max(1.0), fg, 1.0));
            }

            let mut ch_str = String::new();
            if flags.contains(Flags::HIDDEN) {
                ch_str.push(' ');
            } else {
                ch_str.push(indexed.cell.c);
                if let Some(extra) = indexed.cell.zerowidth() {
                    ch_str.extend(extra);
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
                        runs.push((line, "\n".into(), theme.ink, Flags::empty()));
                    }
                    runs.push((line, ch_str, fg, style));
                }
            }
        }

        // Cursor goes under the text so a block cursor can invert its cell.
        if cursor_line >= 0 && cursor.shape != CursorShape::Hidden {
            let (x, y) = (cell_x(cursor_col), cell_y(cursor_line as usize));
            let rect = match cursor.shape {
                CursorShape::Beam => Rect::new(x, y, (cw * 0.12).max(2.0), ch, theme.cursor, 1.0),
                CursorShape::Underline => {
                    Rect::new(x, y + ch * 0.88, cw, (ch * 0.1).max(2.0), theme.cursor, 1.0)
                }
                CursorShape::HollowBlock => Rect::new(x, y, cw, ch, theme.cursor, 0.35),
                _ => Rect::new(x, y, cw, ch, theme.cursor, 1.0),
            };
            self.rects.push(rect);
        }

        let family = Family::Name(theme.font_family);
        let spans = runs.iter().map(|(_, text, [r, g, b], flags)| {
            let mut attrs = Attrs::new().family(family).color(Color::rgb(*r, *g, *b));
            if flags.contains(Flags::BOLD) {
                attrs = attrs.weight(Weight::BOLD);
            }
            if flags.contains(Flags::ITALIC) {
                attrs = attrs.style(Style::Italic);
            }
            (text.as_str(), attrs)
        });
        let metrics = self.metrics(theme);
        let (w, h) = (self.config.width as f32, self.config.height as f32);
        let mut text = self.text.borrow_with(&mut self.font_system);
        text.set_metrics(metrics);
        text.set_size(Some(w), Some(h));
        text.set_rich_text(spans, &Attrs::new().family(family), Shaping::Advanced, None);
        text.shape_until_scroll(false);
    }

    fn draw(&mut self, theme: &Theme, view: &wgpu::TextureView) {
        let (w, h) = (self.config.width, self.config.height);
        let s = theme.surface;
        let d = theme.surface_shade;
        let uniforms = Uniforms {
            surface: [s[0], s[1], s[2], theme.texture as u32 as f32],
            surface_shade: [d[0], d[1], d[2], theme.texture_strength],
            resolution: [w as f32, h as f32],
            time: self.started.elapsed().as_secs_f32(),
            _pad: 0.0,
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let globals = RectGlobals { resolution: [w as f32, h as f32], _pad: [0.0; 2] };
        self.queue
            .write_buffer(&self.rect_globals, 0, bytemuck::bytes_of(&globals));

        let needed = (self.rects.len() * std::mem::size_of::<Rect>()) as u64;
        if needed > self.rect_buffer.size() {
            self.rect_buffer = Self::create_rect_buffer(&self.device, self.rects.len().next_power_of_two());
        }
        if !self.rects.is_empty() {
            self.queue
                .write_buffer(&self.rect_buffer, 0, bytemuck::cast_slice(&self.rects));
        }

        self.viewport.update(&self.queue, Resolution { width: w, height: h });
        let pad = self.padding();
        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [TextArea {
                    buffer: &self.text,
                    left: pad,
                    top: pad,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: w as i32,
                        bottom: h as i32,
                    },
                    default_color: {
                        let [r, g, b] = theme.ink;
                        Color::rgb(r, g, b)
                    },
                    custom_glyphs: &[],
                }],
                &mut self.swash_cache,
            )
            .expect("prepare text");

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.background);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);

            if !self.rects.is_empty() {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_bind_group(0, &self.rect_bind_group, &[]);
                pass.set_vertex_buffer(0, self.rect_buffer.slice(..needed));
                pass.draw(0..4, 0..self.rects.len() as u32);
            }

            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("render text");
        }
        self.queue.submit(Some(encoder.finish()));
    }
}

/// Maps a terminal colour onto the theme. The 16 ANSI colours come from the
/// theme's palette; 256-colour and true-colour values are used as-is.
fn resolve(theme: &Theme, color: AnsiColor, default: [u8; 3]) -> [u8; 3] {
    match color {
        AnsiColor::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
        AnsiColor::Indexed(i) => indexed(theme, i),
        AnsiColor::Named(named) => match named {
            NamedColor::Foreground | NamedColor::BrightForeground => theme.ink,
            NamedColor::DimForeground => mix(theme.ink, theme.surface_rgb(), 0.4),
            NamedColor::Background => theme.surface_rgb(),
            NamedColor::Cursor => theme.cursor,
            NamedColor::DimBlack => theme.palette[0],
            NamedColor::DimRed => theme.palette[1],
            NamedColor::DimGreen => theme.palette[2],
            NamedColor::DimYellow => theme.palette[3],
            NamedColor::DimBlue => theme.palette[4],
            NamedColor::DimMagenta => theme.palette[5],
            NamedColor::DimCyan => theme.palette[6],
            NamedColor::DimWhite => theme.palette[7],
            other => match (other as usize) < 16 {
                true => theme.palette[other as usize],
                false => default,
            },
        },
    }
}

/// xterm's 256-colour table: 16 theme colours, a 6x6x6 cube, then greys.
fn indexed(theme: &Theme, i: u8) -> [u8; 3] {
    match i {
        0..=15 => theme.palette[i as usize],
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
