//! Tsushima's brushwork for window reveals: a few strokes and drops of ink
//! (the window opening) or a flurry of sword cuts (switching to Tsushima),
//! made up at random every time, and their description for transition.wgsl.

use crate::theme::Rng;

/// Most marks one brush can be made of (the shader's array size).
pub const MAX_MARKS: usize = 8;

/// What a mark is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A brush stroke: the tip travels, ink soaks out from its sides.
    Stroke,
    /// A drop of ink, spreading in every direction.
    Drop,
    /// A sword cut: a thin, tapered line drawn across the window in an
    /// instant, flinging droplets off to one side. It stays a cut for a
    /// moment (`hold`), then soaks outward like the rest.
    Slash,
}

/// One mark of a brush, in window space with x scaled by the aspect ratio
/// (so a circle is round) and y running 0..1 down the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    /// Where the brush touches down (a cut: where the blade enters).
    pub at: [f32; 2],
    /// Stroke direction, radians (a drop ignores it).
    pub angle: f32,
    /// Half the stroke's width (0 for a drop; a cut's widest, mid-way).
    pub width: f32,
    /// When it touches down, and how fast it travels (strokes) or spreads
    /// (drops), in arbitrary units: `Brush::norm` turns them into 0..1.
    pub delay: f32,
    pub speed: f32,
    /// How much the stroke curves away from a straight line.
    pub bend: f32,
    pub kind: Kind,
    /// Cuts only: how long the blade's path is, how long the cut stays thin
    /// before soaking outward, which side the droplets fly to (+1 / -1),
    /// and how much they fly (0..1).
    pub length: f32,
    pub hold: f32,
    pub side: f32,
    pub splatter: f32,
}

/// A reveal's brushwork: a few strokes and drops, the ink soaking outward
/// from them until the window is covered, and how ragged its edge is.
#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub marks: [Mark; MAX_MARKS],
    pub count: usize,
    /// How fast ink soaks outward past a stroke's edge.
    pub soak: f32,
    /// Edge raggedness: big lobes, torn chunks, bristle streaks, and the
    /// size of the lobes.
    pub lobes: f32,
    pub chunks: f32,
    pub bristles: f32,
    pub lobe_scale: f32,
    /// Seeds the edge noise.
    pub shape: f32,
    /// 1 / the latest any point is reached, so arrival times span 0..1.
    pub norm: f32,
    /// How the reveal's front runs through arrival times: front =
    /// progress × front[0] + front[1]. Ragged brushes start early, so their
    /// lobes that reach in ahead of time still show; cuts start crisp.
    pub front: [f32; 2],
    /// How hard progress eases out: 3 rushes in and lingers (brushwork),
    /// 1 runs evenly (cuts, so the cuts and the opening both show).
    pub ease: f32,
}

/// Once cuts open, the new look runs out of them like watercolour on damp
/// paper, slowing as the water spreads thinner: the time to run a distance
/// x is x + WASH_SPREAD × x² (the shader does the same).
pub const WASH_SPREAD: f32 = 0.6;

/// How long the wash takes to run `distance` (already divided by the soak).
fn wash(distance: f32) -> f32 {
    distance * (1.0 + WASH_SPREAD * distance)
}

impl Mark {
    const NONE: Mark = Mark {
        at: [0.0; 2],
        angle: 0.0,
        width: 0.0,
        delay: 0.0,
        speed: 1.0,
        bend: 0.0,
        kind: Kind::Drop,
        length: 1.0,
        hold: 0.0,
        side: 1.0,
        splatter: 0.0,
    };

    /// When this mark's ink reaches `p` (before normalising).
    pub fn arrival(&self, p: [f32; 2], soak: f32) -> f32 {
        let q = [p[0] - self.at[0], p[1] - self.at[1]];
        if self.kind == Kind::Drop {
            return self.delay + q[0].hypot(q[1]) / self.speed;
        }
        let (sin, cos) = self.angle.sin_cos();
        let along = q[0] * cos + q[1] * sin;
        let across = -q[0] * sin + q[1] * cos - self.bend * along * along;
        if self.kind == Kind::Slash {
            // The blade passes in an instant; the cut is widest mid-way and
            // tapers to nothing at both ends. Everything else waits for the
            // hold, then soaks out from the cut (the shader adds the
            // droplets, which only ever arrive earlier).
            let s = (along / self.length).clamp(0.0, 1.0);
            let width = self.width * (std::f32::consts::PI * s).sin();
            let pass = self.delay + along.clamp(0.0, self.length) / self.speed;
            let off = (across.abs() - width).max(0.0) + (along - self.length).max(0.0) + (-along).max(0.0);
            return if off <= 0.0 { pass } else { self.delay + self.length / self.speed + self.hold + wash(off / soak) };
        }
        // The tip passes `along` at its speed; beyond the stroke's sides or
        // behind where it started, ink only gets there by soaking.
        self.delay + along.max(0.0) / self.speed + ((across.abs() - self.width).max(0.0) + (-along).max(0.0)) / soak
    }
}

impl Brush {
    /// Makes up a brush for a window with this aspect ratio (width / height).
    pub fn random(rng: &mut Rng, aspect: f32) -> Self {
        let mut marks = [Mark::NONE; MAX_MARKS];
        // Mostly a few strokes; sometimes one sweeping stroke, sometimes a
        // flurry; now and then drops of ink as well, or only drops.
        let strokes = match rng.unit() {
            u if u < 0.12 => 0,
            u if u < 0.35 => 1,
            u if u < 0.6 => 2,
            u if u < 0.8 => 3,
            u if u < 0.92 => 4,
            _ => 5,
        };
        let drops = if strokes == 0 { 2 + (rng.unit() * 3.0) as usize } else { (rng.unit() * 2.4) as usize };
        let count = (strokes + drops).min(MAX_MARKS);
        let mut delay = 0.0;
        for (i, mark) in marks.iter_mut().take(count).enumerate() {
            let drop = i >= strokes;
            // Strokes start near an edge and head roughly across the window;
            // drops land anywhere.
            let at = if drop {
                [rng.range(0.1, 0.9) * aspect, rng.range(0.1, 0.9)]
            } else {
                match (rng.unit() * 4.0) as u32 {
                    0 => [rng.range(-0.05, 0.25) * aspect, rng.unit()],
                    1 => [rng.range(0.75, 1.05) * aspect, rng.unit()],
                    2 => [rng.unit() * aspect, rng.range(-0.05, 0.2)],
                    _ => [rng.unit() * aspect, rng.range(0.8, 1.05)],
                }
            };
            let centre = [0.5 * aspect - at[0], 0.5 - at[1]];
            let angle = centre[1].atan2(centre[0]) + rng.range(-0.9, 0.9);
            *mark = Mark {
                at,
                angle,
                width: if drop { 0.0 } else { rng.range(0.06, 0.4) },
                delay,
                speed: if drop { rng.range(0.5, 1.2) } else { rng.range(0.9, 2.6) },
                bend: if drop { 0.0 } else { rng.range(-0.7, 0.7) },
                kind: if drop { Kind::Drop } else { Kind::Stroke },
                ..Mark::NONE
            };
            // Marks follow one another, sometimes overlapping in time.
            delay += rng.range(0.0, 0.35);
        }
        let brush = Brush {
            marks,
            count,
            soak: rng.range(0.45, 1.4),
            lobes: rng.range(0.3, 0.8),
            chunks: rng.range(0.06, 0.2),
            bristles: rng.range(0.02, 0.1),
            lobe_scale: rng.range(0.0022, 0.006),
            shape: rng.unit() * 1000.0,
            norm: 1.0,
            front: [1.95, -0.45],
            ease: 3.0,
        };
        brush.normalised(aspect)
    }

    /// Makes up a flurry of sword cuts for a window with this aspect ratio:
    /// one to five slashes in quick succession, each flinging droplets, then
    /// every cut and droplet soaking outward until the window is covered.
    pub fn sword(rng: &mut Rng, aspect: f32) -> Self {
        let mut marks = [Mark::NONE; MAX_MARKS];
        let diagonal = aspect.hypot(1.0);
        // Different swordplay each time: one sweeping cut, a cross, a claw of
        // near-parallel cuts, a zigzag of alternating cuts, or a flurry.
        let style = rng.unit();
        let count = match style {
            u if u < 0.18 => 1,
            u if u < 0.4 => 2,
            u if u < 0.6 => 2 + (rng.unit() * 2.0) as usize,
            u if u < 0.8 => 3 + (rng.unit() * 2.0) as usize,
            _ => 3 + (rng.unit() * 3.0) as usize,
        };
        let base = rng.range(0.25, 0.85) * if rng.unit() < 0.5 { 1.0 } else { -1.0 };
        let mut delay = 0.0;
        let mut last_end: f32 = 0.0;
        for (i, mark) in marks.iter_mut().take(count).enumerate() {
            let (centre, angle) = match style {
                // One cut, or a cross: through the middle.
                u if u < 0.4 => {
                    let angle = base + i as f32 * rng.range(1.2, 1.9);
                    ([rng.range(0.35, 0.65) * aspect, rng.range(0.35, 0.65)], angle)
                }
                // Claw: side by side, nearly parallel.
                u if u < 0.6 => {
                    let offset = (i as f32 - (count - 1) as f32 / 2.0) * rng.range(0.1, 0.2);
                    let normal = [-base.sin(), base.cos()];
                    let centre = [0.5 * aspect + normal[0] * offset, 0.5 + normal[1] * offset];
                    (centre, base + rng.range(-0.12, 0.12))
                }
                // Zigzag: back and forth down the window.
                u if u < 0.8 => {
                    let y = (i as f32 + 0.5) / count as f32;
                    let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
                    ([rng.range(0.35, 0.65) * aspect, y + rng.range(-0.08, 0.08)], base.abs() * sign + rng.range(-0.2, 0.2))
                }
                // Flurry: anywhere, any way.
                _ => ([rng.range(0.2, 0.8) * aspect, rng.range(0.2, 0.8)], rng.unit() * std::f32::consts::TAU),
            };
            // Most cuts run right across the window, their tapered ends out
            // of sight; some end inside it, showing the taper.
            let length = diagonal * rng.range(0.6, 1.4);
            let way = [angle.cos(), angle.sin()];
            let at = [centre[0] - way[0] * length / 2.0, centre[1] - way[1] * length / 2.0];
            let speed = length / rng.range(0.05, 0.09);
            *mark = Mark {
                at,
                angle,
                width: rng.range(0.006, 0.02),
                delay,
                speed,
                bend: rng.range(-0.12, 0.12),
                kind: Kind::Slash,
                length,
                hold: 0.0,
                side: if rng.unit() < 0.5 { 1.0 } else { -1.0 },
                splatter: rng.range(0.4, 1.0),
            };
            last_end = last_end.max(delay + length / speed);
            // The claw's cuts land almost together; the others follow one
            // another, the next often starting before the last has finished.
            delay += if (0.4..0.6).contains(&style) {
                rng.range(0.01, 0.04)
            } else {
                length / speed * rng.range(0.5, 1.3)
            };
        }
        // A breath after the last cut, every cut opens up at about once.
        let open = last_end + rng.range(0.05, 0.14);
        for mark in marks.iter_mut().take(count) {
            mark.hold = open + rng.range(0.0, 0.05) - (mark.delay + mark.length / mark.speed);
        }
        let brush = Brush {
            marks,
            count,
            soak: rng.range(0.8, 1.5),
            lobes: rng.range(0.12, 0.3),
            chunks: rng.range(0.04, 0.1),
            bristles: rng.range(0.03, 0.09),
            lobe_scale: rng.range(0.0025, 0.006),
            shape: rng.unit() * 1000.0,
            norm: 1.0,
            // The wash creeps into the last corners slowly; it reaches most
            // of the way, and the rest fades in at the end (present.wgsl).
            front: [0.97, -0.02],
            ease: 1.0,
        };
        brush.normalised(aspect)
    }

    /// Sets the time scale from the latest point to be reached (sampled over
    /// the window), so the reveal always finishes.
    fn normalised(mut self, aspect: f32) -> Self {
        let mut latest: f32 = 1e-3;
        for j in 0..=24 {
            for i in 0..=32 {
                let p = [i as f32 / 32.0 * aspect, j as f32 / 24.0];
                latest = latest.max(self.arrival(p));
            }
        }
        self.norm = 1.0 / latest;
        self
    }

    /// When the brush reaches `p`: the earliest of its marks.
    pub fn arrival(&self, p: [f32; 2]) -> f32 {
        self.marks[..self.count].iter().map(|m| m.arrival(p, self.soak)).fold(f32::INFINITY, f32::min)
    }
}

/// What transition.wgsl reads (its `Plan` struct): edge raggedness (lobes,
/// chunks, bristles, lobe size); marks, 1 / latest arrival, noise seed, soak
/// speed; the colour of cut edges and how strong; how the front runs
/// (progress x x + y) and how the wash slows; three vec4s per mark.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PlanData {
    edge: [f32; 4],
    info: [f32; 4],
    cut: [f32; 4],
    front: [f32; 4],
    marks: [[f32; 4]; 3 * MAX_MARKS],
}

impl PlanData {
    /// A sword cut through the whole sheet, for clearing it: along the line
    /// through `at` (x across 0..aspect, y down 0..1) at `angle`, the halves
    /// then sliding apart along it and falling (`front.w` = 2).
    pub fn slash(at: [f32; 2], angle: f32, seed: f32) -> Self {
        let mut data = Self { edge: [0.0; 4], info: [0.0, 0.0, seed, 0.0], cut: [0.0; 4], front: [0.0, 0.0, 0.0, 2.0], marks: [[0.0; 4]; 3 * MAX_MARKS] };
        data.marks[0] = [at[0], at[1], angle, 0.0];
        data
    }

    /// A fire, for closing a sheet: lit at `at` (x across 0..aspect, y down
    /// 0..1), its front `reach` away when it is done (`front.w` = 1 tells
    /// the shader it is a burn).
    pub fn burn(at: [f32; 2], reach: f32, seed: f32) -> Self {
        let mut data = Self { edge: [0.0; 4], info: [0.0, 0.0, seed, 0.0], cut: [0.0; 4], front: [0.0, 0.0, 0.0, 1.0], marks: [[0.0; 4]; 3 * MAX_MARKS] };
        data.marks[0] = [at[0], at[1], reach, 0.0];
        data
    }

    /// The brush, with cut edges drawn in `cut`.
    pub fn new(b: &Brush, cut: [u8; 3]) -> Self {
        let [r, g, bl] = cut.map(|c| c as f32 / 255.0);
        let mut data = Self {
            edge: [b.lobes, b.chunks, b.bristles, b.lobe_scale],
            info: [b.count as f32, b.norm, b.shape, b.soak],
            cut: [r, g, bl, 0.9],
            front: [b.front[0], b.front[1], WASH_SPREAD, 0.0],
            marks: [[0.0; 4]; 3 * MAX_MARKS],
        };
        for (i, m) in b.marks[..b.count].iter().enumerate() {
            let kind = match m.kind {
                Kind::Stroke => 0.0,
                Kind::Drop => 1.0,
                Kind::Slash => 2.0,
            };
            data.marks[3 * i] = [m.at[0], m.at[1], m.angle, m.width];
            data.marks[3 * i + 1] = [m.delay, m.speed, m.bend, kind];
            data.marks[3 * i + 2] = [m.length, m.hold, m.side, m.splatter];
        }
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_brush_covers_the_window_and_differs() {
        let mut rng = Rng::new(42);
        let mut counts = std::collections::HashSet::new();
        let mut first_marks = Vec::new();
        for _ in 0..200 {
            let aspect = rng.range(0.5, 2.5);
            let brush = Brush::random(&mut rng, aspect);
            assert!(brush.count >= 1 && brush.count <= MAX_MARKS);
            // Normalised, every point is reached by the end (the corners too).
            for p in [[0.0, 0.0], [aspect, 0.0], [0.0, 1.0], [aspect, 1.0], [aspect / 2.0, 0.5]] {
                let t = brush.arrival(p) * brush.norm;
                assert!((0.0..=1.0001).contains(&t), "{t}");
            }
            counts.insert(brush.count);
            first_marks.push(brush.marks[0]);
        }
        // Many different compositions, never the same one twice.
        assert!(counts.len() >= 5, "{counts:?}");
        for (i, a) in first_marks.iter().enumerate() {
            assert!(first_marks[i + 1..].iter().all(|b| b != a));
        }
    }

    #[test]
    fn sword_cuts_come_first_then_open_together() {
        let mut rng = Rng::new(9);
        let mut counts = std::collections::HashSet::new();
        for _ in 0..200 {
            let aspect = rng.range(0.5, 2.5);
            let brush = Brush::sword(&mut rng, aspect);
            let cuts = &brush.marks[..brush.count];
            assert!(cuts.iter().all(|m| m.kind == Kind::Slash));
            for p in [[0.0, 0.0], [aspect, 0.0], [0.0, 1.0], [aspect, 1.0], [aspect / 2.0, 0.5]] {
                let t = brush.arrival(p) * brush.norm;
                assert!((0.0..=1.0001).contains(&t), "{t}");
            }
            // Every cut is made before any of them opens up.
            let made = cuts.iter().map(|m| m.delay + m.length / m.speed).fold(0.0, f32::max);
            let opens = cuts.iter().map(|m| m.delay + m.length / m.speed + m.hold).fold(f32::INFINITY, f32::min);
            assert!(made < opens, "{made} {opens}");
            counts.insert(brush.count);
        }
        assert!(counts.len() >= 4, "{counts:?}");
    }
}
