//! Pictures in the terminal: the iTerm2 inline image protocol, which
//! `imgcat`, `oneprompt img` and many tools send:
//!
//!   ESC ] 1337 ; File = [key=value;...] : <base64 of the file> BEL
//!
//! with `inline=1` to show it, and optionally `width` and `height` (N cells,
//! Npx, N% of the window, or auto) and `preserveAspectRatio=0`. PNG, JPEG,
//! GIF (its first frame), WebP and BMP are read.
//!
//! The PTY scanner (marks.rs) takes the sequence out of the stream, has the
//! picture decoded here, and puts a block of blank cells in its place, the
//! size the picture takes, each row a hyperlink to `oneprompt-image:<id>:
//! <row>`. Those cells carry the picture wherever they scroll; the renderer
//! draws it over them. The cursor ends below it, as in iTerm2.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;

use crate::terminal::GridSize;

/// The hyperlink address the cells under a picture carry: this, its id, a
/// colon and the row of the picture the cell is on.
pub const IMAGE_URI: &str = "oneprompt-image:";

/// Pictures larger than this on a side are scaled down when read, to keep
/// memory in bounds.
const MAX_SIDE: u32 = 2048;

/// The most bytes one picture's sequence may carry.
pub const MAX_BYTES: usize = 64 << 20;

/// A decoded picture and the cells it covers.
#[derive(Debug)]
pub struct Picture {
    pub id: u64,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub columns: usize,
    pub rows: usize,
}

/// (id, row) of the picture a cell's hyperlink belongs to.
pub fn picture_cell(uri: &str) -> Option<(u64, usize)> {
    let (id, row) = uri.strip_prefix(IMAGE_URI)?.split_once(':')?;
    Some((id.parse().ok()?, row.parse().ok()?))
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// One `File=` sequence's text (after `1337;File=`, before the terminator):
/// the picture to show and the bytes to put in its place, or None if it is
/// not an inline picture or can't be read.
pub fn read(body: &[u8], size: &Arc<Mutex<GridSize>>) -> Option<(Picture, Vec<u8>)> {
    let colon = body.iter().position(|&b| b == b':')?;
    let args = String::from_utf8_lossy(&body[..colon]);
    let arg = |key: &str| args.split(';').find_map(|kv| kv.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.trim().to_string()));
    // Without inline=1 the protocol means "download the file": not shown.
    if arg("inline").as_deref() != Some("1") {
        return None;
    }
    let data: Vec<u8> = body[colon + 1..].iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(&data).ok()?;
    let mut image = image::load_from_memory(&bytes).ok()?;
    if image.width() > MAX_SIDE || image.height() > MAX_SIDE {
        image = image.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle);
    }
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let grid = *size.lock().ok()?;
    let (columns, rows) = cells(&grid, width, height, arg("width").as_deref(), arg("height").as_deref(), arg("preserveAspectRatio").as_deref() != Some("0"));
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let picture = Picture { id, rgba: rgba.into_raw(), width, height, columns, rows };
    Some((picture, placeholder(id, columns, rows)))
}

/// One dimension as the protocol gives it, in pixels: N cells, Npx, N% of
/// `whole`, or None for auto.
fn length(spec: Option<&str>, cell: f32, whole: f32) -> Option<f32> {
    let spec = spec?.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("auto") {
        return None;
    }
    if let Some(px) = spec.strip_suffix("px") {
        return px.trim().parse().ok();
    }
    if let Some(pct) = spec.strip_suffix('%') {
        return pct.trim().parse::<f32>().ok().map(|p| whole * p / 100.0);
    }
    spec.parse::<f32>().ok().map(|n| n * cell)
}

/// The cells a `width` x `height` picture takes on `grid`, as asked: by
/// default its own size, shrunk to fit the grid's width (and its height,
/// so it can be seen whole), keeping its shape unless told not to.
fn cells(grid: &GridSize, width: u32, height: u32, w: Option<&str>, h: Option<&str>, keep_shape: bool) -> (usize, usize) {
    let (cw, ch) = (grid.cell_width.max(1.0), grid.cell_height.max(1.0));
    let (max_w, max_h) = (grid.columns as f32 * cw, grid.lines.saturating_sub(1).max(1) as f32 * ch);
    let (iw, ih) = (width as f32, height as f32);
    let (mut dw, mut dh) = match (length(w, cw, max_w), length(h, ch, max_h)) {
        (Some(dw), Some(dh)) if !keep_shape => (dw, dh),
        (Some(dw), Some(dh)) => {
            let s = (dw / iw).min(dh / ih);
            (iw * s, ih * s)
        }
        (Some(dw), None) => (dw, ih * dw / iw),
        (None, Some(dh)) => (iw * dh / ih, dh),
        (None, None) => (iw, ih),
    };
    let fit = (max_w / dw).min(max_h / dh).min(1.0);
    if keep_shape {
        dw *= fit;
        dh *= fit;
    } else {
        dw = dw.min(max_w);
        dh = dh.min(max_h);
    }
    (((dw / cw).ceil() as usize).clamp(1, grid.columns.max(1)), ((dh / ch).ceil() as usize).max(1))
}

/// Blank cells for the picture to sit on, a row at a time, each row's cells
/// linked to the picture and its row; the cursor ends at the start of the
/// line below.
fn placeholder(id: u64, columns: usize, rows: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for row in 0..rows {
        out.extend_from_slice(format!("\x1b]8;id=oneprompt-image-{id};{IMAGE_URI}{id}:{row}\x1b\\").as_bytes());
        out.extend(std::iter::repeat_n(b' ', columns));
        out.extend_from_slice(b"\x1b]8;;\x1b\\\r\n");
    }
    out
}

/// The escape sequence that shows `bytes` (an image file) inline, for
/// `oneprompt img` and the Markdown viewer; `width` in cells, if given.
pub fn sequence(bytes: &[u8], name: &str, width: Option<usize>) -> String {
    let engine = base64::engine::general_purpose::STANDARD;
    let width = width.map(|w| format!(";width={w}")).unwrap_or_default();
    format!("\x1b]1337;File=name={};size={};inline=1{width}:{}\x07", engine.encode(name), bytes.len(), engine.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> GridSize {
        GridSize { columns: 80, lines: 24, cell_width: 10.0, cell_height: 20.0 }
    }

    #[test]
    fn pictures_take_their_size_in_cells_and_fit_the_grid() {
        let g = grid();
        // 200x100 px: 20 columns, 5 rows.
        assert_eq!(cells(&g, 200, 100, None, None, true), (20, 5));
        // Too wide: shrunk to the grid's 800 px, keeping its shape.
        assert_eq!(cells(&g, 1600, 400, None, None, true), (80, 10));
        // Asked for 40 cells wide.
        assert_eq!(cells(&g, 200, 100, Some("40"), None, true), (40, 10));
        assert_eq!(cells(&g, 200, 100, Some("50%"), None, true), (40, 10));
        assert_eq!(cells(&g, 200, 100, Some("100px"), Some("auto"), true), (10, 3));
        assert_eq!(cells(&g, 200, 100, Some("10"), Some("10"), false), (10, 10));
    }

    #[test]
    fn a_picture_is_read_and_leaves_linked_cells() {
        // A 4x2 red PNG.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 2, image::Rgba([200, 30, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let seq = sequence(&png, "red.png", None);
        let body = seq.strip_prefix("\x1b]1337;File=").unwrap().strip_suffix('\x07').unwrap();
        let size = Arc::new(Mutex::new(grid()));
        let (picture, cells) = read(body.as_bytes(), &size).unwrap();
        assert_eq!((picture.width, picture.height, picture.columns, picture.rows), (4, 2, 1, 1));
        assert_eq!(picture.rgba.len(), 4 * 2 * 4);
        let text = String::from_utf8(cells).unwrap();
        assert!(text.contains(&format!("{IMAGE_URI}{}:0", picture.id)));
        assert_eq!(picture_cell(&format!("{IMAGE_URI}7:3")), Some((7, 3)));
        // Not inline: a download, not shown.
        assert!(read(b"name=eA==:AAAA", &size).is_none());
    }
}
