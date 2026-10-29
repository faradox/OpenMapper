//! The shader library: ready-made generator presets, grouped by mood. Each
//! one is an ordinary `Generator`, so after adding it to a project every
//! setting stays editable in the Sources panel.

use crate::project::{Generator, Palette, Pattern};

pub struct Preset {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub generator: Generator,
}

pub const CATEGORIES: [&str; 5] = ["Psychedelic", "Geometric", "Organic", "Cosmic", "Basics"];

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

#[allow(clippy::too_many_arguments)]
fn preset(
    name: &'static str,
    category: &'static str,
    description: &'static str,
    pattern: Pattern,
    palette: Palette,
    scale: f32,
    speed: f32,
    detail: f32,
) -> Preset {
    let generator =
        Generator { pattern, palette, scale, speed, detail, color_a: BLACK, color_b: WHITE, angle: 0.0 };
    Preset { name, category, description, generator }
}

/// Every preset, in display order.
pub fn presets() -> Vec<Preset> {
    use Palette as C;
    use Pattern as P;
    let mut list = vec![
        preset(
            "Acid Tunnel",
            "Psychedelic",
            "Fly down an endless twisting checker tunnel.",
            P::Tunnel,
            C::Acid,
            8.0,
            0.5,
            0.65,
        ),
        preset(
            "Kaleidoscope",
            "Psychedelic",
            "Mirrored, slowly turning liquid color.",
            P::Kaleidoscope,
            C::Rainbow,
            2.5,
            0.3,
            0.4,
        ),
        preset(
            "Hypno Spiral",
            "Psychedelic",
            "Black and white spiral that pulls you in.",
            P::Spiral,
            C::Mono,
            6.0,
            0.6,
            0.5,
        ),
        preset(
            "Liquid Dream",
            "Psychedelic",
            "Warped noise flowing like oil on water.",
            P::Liquid,
            C::Sunset,
            3.0,
            0.3,
            0.5,
        ),
        preset(
            "Vortex",
            "Psychedelic",
            "Rainbow whirlpool, tighter toward the center.",
            P::Vortex,
            C::Rainbow,
            6.0,
            0.4,
            0.5,
        ),
        preset(
            "Julia Bloom",
            "Psychedelic",
            "An endlessly morphing fractal.",
            P::Julia,
            C::Neon,
            4.0,
            0.3,
            0.5,
        ),
        preset(
            "Sunburst",
            "Geometric",
            "Rotating rays from the center.",
            P::Sunburst,
            C::Sunset,
            5.0,
            0.2,
            0.3,
        ),
        preset(
            "Hex Pulse",
            "Geometric",
            "A honeycomb with waves of light running through it.",
            P::Hexagons,
            C::Neon,
            6.0,
            0.5,
            0.4,
        ),
        preset(
            "Truchet Flow",
            "Geometric",
            "Interlocking arcs that keep flipping into new paths.",
            P::Truchet,
            C::Ocean,
            6.0,
            0.5,
            0.45,
        ),
        preset(
            "Op Art",
            "Geometric",
            "Wavy stripes in the style of 1960s op art.",
            P::OpArt,
            C::Mono,
            4.0,
            0.4,
            0.5,
        ),
        preset(
            "Moiré",
            "Geometric",
            "Two ring patterns drifting over each other.",
            P::Moire,
            C::Mono,
            4.0,
            0.3,
            0.5,
        ),
        preset("Lava Lamp", "Organic", "Soft blobs that merge and split.", P::Lava, C::Fire, 4.0, 0.4, 0.5),
        preset("Cells", "Organic", "Living cells drifting around.", P::Cells, C::Candy, 5.0, 0.5, 0.3),
        preset("Fire", "Organic", "Flames rising from the bottom edge.", P::Fire, C::Fire, 4.0, 0.6, 0.5),
        preset(
            "Aurora",
            "Organic",
            "Northern lights: soft ribbons of color.",
            P::Aurora,
            C::Aurora,
            3.0,
            0.4,
            0.5,
        ),
        preset(
            "Ripples",
            "Organic",
            "Waves from several drops interfering.",
            P::Ripples,
            C::Ice,
            4.0,
            0.5,
            0.4,
        ),
        preset("Warp Speed", "Cosmic", "Stars rushing toward you.", P::Starfield, C::Ice, 4.0, 0.5, 0.4),
        preset(
            "Digital Rain",
            "Cosmic",
            "Falling streams of glowing blocks.",
            P::Rain,
            C::Duotone,
            4.0,
            0.5,
            0.5,
        ),
        preset(
            "Neon Plasma",
            "Cosmic",
            "Classic demo-scene plasma in neon.",
            P::Plasma,
            C::Neon,
            3.0,
            0.4,
            0.5,
        ),
        preset("Nebula", "Cosmic", "Slow cosmic clouds.", P::Liquid, C::Neon, 1.5, 0.1, 0.9),
        preset(
            "Gradient",
            "Basics",
            "Two colors sliding back and forth.",
            P::Gradient,
            C::Duotone,
            4.0,
            0.25,
            0.5,
        ),
        preset("Checker", "Basics", "Scrolling checkerboard.", P::Checker, C::Duotone, 6.0, 0.25, 0.5),
        preset("Stripes", "Basics", "Moving stripes.", P::Stripes, C::Duotone, 6.0, 0.5, 0.5),
        preset("Rings", "Basics", "Rings growing from the center.", P::Rings, C::Duotone, 5.0, 0.5, 0.5),
        preset("Noise", "Basics", "Soft, cloudy noise.", P::Noise, C::Duotone, 4.0, 0.25, 0.5),
        preset(
            "Test Grid",
            "Basics",
            "Calibration grid for lining up surfaces.",
            P::TestGrid,
            C::Duotone,
            8.0,
            0.0,
            0.5,
        ),
    ];
    // Duotone presets get colors that suit them.
    for p in &mut list {
        let g = &mut p.generator;
        match (g.pattern, g.palette) {
            (Pattern::Rain, _) => (g.color_a, g.color_b) = (BLACK, [0.2, 1.0, 0.45, 1.0]),
            (Pattern::TestGrid, _) => (g.color_a, g.color_b) = (BLACK, WHITE),
            (_, Palette::Duotone) => (g.color_a, g.color_b) = ([0.05, 0.1, 0.9, 1.0], [1.0, 0.2, 0.6, 1.0]),
            _ => {}
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_valid() {
        let list = presets();
        for (i, p) in list.iter().enumerate() {
            assert!(CATEGORIES.contains(&p.category), "{}", p.name);
            assert!(!list[..i].iter().any(|q| q.name == p.name), "duplicate {}", p.name);
            assert!((0.0..=1.0).contains(&p.generator.detail));
        }
        // Every new pattern is in the library.
        for pattern in Pattern::ALL {
            assert!(list.iter().any(|p| p.generator.pattern == pattern), "{pattern:?} missing");
        }
    }
}
