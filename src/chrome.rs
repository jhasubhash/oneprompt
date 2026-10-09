//! Window chrome drawn by the theme: the close / minimize / zoom controls in
//! the top strip. The native title bar is hidden, so these replace it.

/// One of the window controls, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Close,
    Minimize,
    Zoom,
}

pub const CONTROLS: [Control; 3] = [Control::Close, Control::Minimize, Control::Zoom];

/// Layout in logical pixels, matching the spacing of macOS's own buttons.
const RADIUS: f32 = 6.5;
const FIRST_X: f32 = 20.0;
const SPACING: f32 = 21.0;
const CENTER_Y: f32 = 16.0;

/// Centre and radius of a control, in physical pixels.
pub fn geometry(control: Control, scale: f32) -> (f32, f32, f32) {
    let index = CONTROLS.iter().position(|&c| c == control).unwrap_or(0) as f32;
    ((FIRST_X + index * SPACING) * scale, CENTER_Y * scale, RADIUS * scale)
}

/// The control under a point (physical pixels), with a little slack.
pub fn hit(x: f64, y: f64, scale: f32) -> Option<Control> {
    CONTROLS.into_iter().find(|&c| {
        let (cx, cy, r) = geometry(c, scale);
        let (dx, dy) = (x as f32 - cx, y as f32 - cy);
        dx * dx + dy * dy <= (r + 2.0 * scale).powi(2)
    })
}

/// Whether a point is over the group, which reveals the symbols as on macOS.
pub fn over_group(x: f64, y: f64, scale: f32) -> bool {
    let (x0, cy, r) = geometry(Control::Close, scale);
    let (x1, _, _) = geometry(Control::Zoom, scale);
    let (x, y) = (x as f32, y as f32);
    x >= x0 - r * 1.5 && x <= x1 + r * 1.5 && (y - cy).abs() <= r * 1.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_each_control_at_its_centre() {
        for scale in [1.0, 2.0] {
            for control in CONTROLS {
                let (cx, cy, _) = geometry(control, scale);
                assert_eq!(hit(cx as f64, cy as f64, scale), Some(control));
            }
        }
    }

    #[test]
    fn misses_between_and_below_controls() {
        let (x0, cy, _) = geometry(Control::Close, 1.0);
        let (x1, _, _) = geometry(Control::Minimize, 1.0);
        assert_eq!(hit(((x0 + x1) / 2.0) as f64, cy as f64, 1.0), None);
        assert_eq!(hit(x0 as f64, (cy + 20.0) as f64, 1.0), None);
        assert!(over_group(((x0 + x1) / 2.0) as f64, cy as f64, 1.0));
    }
}
