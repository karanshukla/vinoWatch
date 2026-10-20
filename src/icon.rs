//! The tray and app icon, in vinoWhisper's style: a dark rounded tile with a
//! faint edge and pill-shaped marks. Here the marks are three meters (CPU, RAM,
//! NPU). Drawn in code so the tray needs no image decoder, and the checked-in
//! SVGs in `data/` are generated from the same shapes.

use std::io;
use std::path::{Path, PathBuf};

use crate::raster::{Canvas, Rect, Rgba};

#[derive(Debug, Clone, Copy)]
struct Shape {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
}

impl Shape {
    const fn new(x: f32, y: f32, w: f32, h: f32, r: f32) -> Shape {
        Shape { x, y, w, h, r }
    }

    fn inset(self, by: f32) -> Shape {
        Shape::new(
            self.x + by,
            self.y + by,
            self.w - by * 2.0,
            self.h - by * 2.0,
            self.r - by,
        )
    }

    fn rect(self, unit: f32) -> Rect {
        Rect {
            x0: self.x * unit,
            y0: self.y * unit,
            x1: (self.x + self.w) * unit,
            y1: (self.y + self.h) * unit,
        }
    }

    fn svg_path(self) -> String {
        let Shape { x, y, w, h, .. } = self;
        format!("M{x} {y}h{w}v{h}h-{w}z")
    }

    fn svg_rect(self, paint: &str) -> String {
        let Shape { x, y, w, h, r } = self;
        format!(r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" {paint}/>"#)
    }
}

/// What the icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No OpenVINO processes.
    Idle,
    /// At least one OpenVINO process is running.
    Active,
    /// Available memory is under the `--warn-below` share.
    LowMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// The empty length of a meter.
    Track,
    /// The filled length.
    Level,
}

// vinoWhisper's palette (gui/src/paint.rs), as in vinoAuthFace.
const TILE: Rgba = Rgba(14, 14, 16, 255);
const EDGE: Rgba = Rgba(255, 255, 255, 46);
const WHITE: Rgba = Rgba(250, 250, 250, 255);
const TRACK: Rgba = Rgba(160, 164, 172, 100);
const GREEN: Rgba = Rgba(86, 204, 120, 255);
const AMBER: Rgba = Rgba(245, 184, 66, 255);

// The symbolic icon sits straight on the panel, light or dark, so its accents
// are darker shades of the two above that keep 3:1 against both.
const TRAY_GREEN: Rgba = Rgba(40, 150, 80, 255);
const TRAY_AMBER: Rgba = Rgba(184, 115, 0, 255);

fn paint(state: State, part: Part) -> Rgba {
    match (state, part) {
        (_, Part::Track) => TRACK,
        (State::Idle, Part::Level) => WHITE,
        (State::Active, Part::Level) => GREEN,
        (State::LowMemory, Part::Level) => AMBER,
    }
}

struct Grid {
    tile: Shape,
    edge: f32,
    /// Each meter's track, then its level drawn over the start of it.
    marks: &'static [(Shape, Part)],
    size: f32,
}

/// 64-unit design, for the app icon and the larger tray sizes.
const LARGE: Grid = Grid {
    tile: Shape::new(4.0, 4.0, 56.0, 56.0, 12.0),
    edge: 2.0,
    marks: &[
        (Shape::new(14.0, 18.0, 36.0, 6.0, 3.0), Part::Track),
        (Shape::new(14.0, 18.0, 22.0, 6.0, 3.0), Part::Level),
        (Shape::new(14.0, 29.0, 36.0, 6.0, 3.0), Part::Track),
        (Shape::new(14.0, 29.0, 30.0, 6.0, 3.0), Part::Level),
        (Shape::new(14.0, 40.0, 36.0, 6.0, 3.0), Part::Track),
        (Shape::new(14.0, 40.0, 14.0, 6.0, 3.0), Part::Level),
    ],
    size: 64.0,
};

/// 16-unit design on whole pixels, so the 16 px tray icon stays sharp.
const SMALL: Grid = Grid {
    tile: Shape::new(1.0, 1.0, 14.0, 14.0, 3.0),
    edge: 1.0,
    marks: &[
        (Shape::new(3.0, 4.0, 10.0, 2.0, 0.0), Part::Track),
        (Shape::new(3.0, 4.0, 6.0, 2.0, 0.0), Part::Level),
        (Shape::new(3.0, 7.0, 10.0, 2.0, 0.0), Part::Track),
        (Shape::new(3.0, 7.0, 8.0, 2.0, 0.0), Part::Level),
        (Shape::new(3.0, 10.0, 10.0, 2.0, 0.0), Part::Track),
        (Shape::new(3.0, 10.0, 4.0, 2.0, 0.0), Part::Level),
    ],
    size: 16.0,
};

/// 16-unit design for the symbolic tray icon, which has no tile: the meters
/// sit on a 12-unit square with a 2-unit margin, the same footprint as the
/// other icons in the tray.
const SYMBOLIC: Grid = Grid {
    tile: Shape::new(0.0, 0.0, 16.0, 16.0, 0.0),
    edge: 0.0,
    marks: &[
        (Shape::new(9.0, 3.0, 5.0, 2.0, 0.0), Part::Track),
        (Shape::new(2.0, 3.0, 7.0, 2.0, 0.0), Part::Level),
        (Shape::new(12.0, 7.0, 2.0, 2.0, 0.0), Part::Track),
        (Shape::new(2.0, 7.0, 10.0, 2.0, 0.0), Part::Level),
        (Shape::new(7.0, 11.0, 7.0, 2.0, 0.0), Part::Track),
        (Shape::new(2.0, 11.0, 5.0, 2.0, 0.0), Part::Level),
    ],
    size: 16.0,
};

fn hex(Rgba(r, g, b, _): Rgba) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn opacity(Rgba(_, _, _, a): Rgba) -> String {
    format!("{:.2}", f32::from(a) / 255.0)
}

/// The installed app icon (`vinowatch.svg`): the active state, with the
/// viewBox cropped to a 2-unit margin. The 64-unit grid's 4-unit margin left
/// the tile visibly smaller than other apps' icons in a task manager; the tray
/// pixmaps keep it, since they're drawn at their own size.
pub fn app_svg() -> String {
    let mut svg =
        String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"2 2 60 60\">\n");
    let mut line = |element: String| {
        svg.push_str("  ");
        svg.push_str(&element);
        svg.push('\n');
    };
    line(LARGE.tile.svg_rect(&format!(r#"fill="{}""#, hex(TILE))));
    // SVG strokes straddle the path; inset by half to match stroke_rounded.
    line(LARGE.tile.inset(LARGE.edge / 2.0).svg_rect(&format!(
        r#"fill="none" stroke="{}" stroke-opacity="{}" stroke-width="{}""#,
        hex(EDGE),
        opacity(EDGE),
        LARGE.edge
    )));
    for &(shape, part) in LARGE.marks {
        let color = paint(State::Active, part);
        let paint = if color.3 == 255 {
            format!(r#"fill="{}""#, hex(color))
        } else {
            format!(r#"fill="{}" fill-opacity="{}""#, hex(color), opacity(color))
        };
        line(shape.svg_rect(&paint));
    }
    svg.push_str("</svg>\n");
    svg
}

impl State {
    pub const ALL: [State; 3] = [State::Idle, State::Active, State::LowMemory];

    /// The themed icon name the tray asks for, as vinoWhisper's tray does:
    /// Plasma recolours a `-symbolic` icon to the panel's text colour, and the
    /// pixmaps below are only the fallback when the theme has none.
    pub fn icon_name(self) -> &'static str {
        match self {
            State::Idle => "vinowatch-symbolic",
            State::Active => "vinowatch-active-symbolic",
            State::LowMemory => "vinowatch-low-memory-symbolic",
        }
    }
}

/// A monochrome tray icon for one state, in the KDE symbolic format: the
/// `ColorScheme-Text` class is what Plasma recolours. The tracks and an idle
/// level follow the theme; the green and amber levels stay fixed.
pub fn symbolic_svg(state: State) -> String {
    let (mut text, mut dim, mut accent) = (String::new(), String::new(), String::new());
    for &(shape, part) in SYMBOLIC.marks {
        let bucket = match (state, part) {
            (_, Part::Track) => &mut dim,
            (State::Idle, Part::Level) => &mut text,
            (State::Active | State::LowMemory, Part::Level) => &mut accent,
        };
        bucket.push_str(&shape.svg_path());
    }
    let accent_fill = hex(if state == State::LowMemory {
        TRAY_AMBER
    } else {
        TRAY_GREEN
    });
    let mut svg = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 16 16\">\n  \
         <style id=\"current-color-scheme\" type=\"text/css\">.ColorScheme-Text { color: #232629; }</style>\n",
    );
    for (path, style) in [
        (text, "fill:currentColor".to_string()),
        (dim, "fill:currentColor;fill-opacity:0.5".to_string()),
        (accent, format!("fill:{accent_fill}")),
    ] {
        if !path.is_empty() {
            let class = if style.contains("currentColor") {
                r#" class="ColorScheme-Text""#
            } else {
                ""
            };
            svg.push_str(&format!(
                "  <path{class} style=\"{style}\" d=\"{path}\"/>\n"
            ));
        }
    }
    svg.push_str("</svg>\n");
    svg
}

/// Every icon file, as its path under a hicolor theme directory and its SVG.
fn files() -> Vec<(String, String)> {
    let mut files = vec![("scalable/apps/vinowatch.svg".to_string(), app_svg())];
    files.extend(State::ALL.map(|s| {
        (
            format!("symbolic/apps/{}.svg", s.icon_name()),
            symbolic_svg(s),
        )
    }));
    files
}

/// Writes the icons under `data/icons/hicolor` (`data` is usually
/// `~/.local/share`), so the tray's icon names resolve in the theme.
pub fn install(data: &Path) -> io::Result<Vec<PathBuf>> {
    let hicolor = data.join("icons/hicolor");
    files()
        .into_iter()
        .map(|(name, svg)| {
            let path = hicolor.join(name);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, svg)?;
            Ok(path)
        })
        .collect()
}

/// Every size a tray might ask for, for one state.
pub fn tray_icons(state: State) -> Vec<ksni::Icon> {
    [16, 22, 24, 32, 48, 64]
        .into_iter()
        .map(|size| tray_icon(state, size))
        .collect()
}

fn tray_icon(state: State, size: u32) -> ksni::Icon {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, size, size);
    let grid = if size < 32 { &SMALL } else { &LARGE };
    draw(&mut canvas, grid, state, size as f32);
    ksni::Icon {
        width: size as i32,
        height: size as i32,
        data: canvas.to_argb32_be(),
    }
}

fn draw(canvas: &mut Canvas, grid: &Grid, state: State, size: f32) {
    let unit = size / grid.size;
    let tile = grid.tile.rect(unit);
    canvas.fill_rounded(tile, grid.tile.r * unit, TILE);
    canvas.stroke_rounded(tile, grid.tile.r * unit, grid.edge * unit, EDGE);
    for &(shape, part) in grid.marks {
        canvas.fill_rounded(shape.rect(unit), shape.r * unit, paint(state, part));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn every_size_is_a_complete_argb_image() {
        for state in State::ALL {
            for icon in tray_icons(state) {
                assert_eq!(icon.data.len(), (icon.width * icon.height * 4) as usize);
            }
        }
    }

    #[test]
    fn the_states_look_different_at_every_size() {
        for size in [16, 22, 64] {
            let [a, b, c] = State::ALL.map(|s| tray_icon(s, size).data);
            assert!(a != b && b != c && a != c, "{size}px states collide");
        }
        let [a, b, c] = State::ALL.map(symbolic_svg);
        assert!(a != b && b != c && a != c, "symbolic states collide");
    }

    #[test]
    fn the_corners_are_clear() {
        for size in [16, 64] {
            assert_eq!(
                tray_icon(State::Active, size).data[0],
                0,
                "{size}px corner is opaque"
            );
        }
    }

    #[test]
    fn the_marks_sit_inside_the_tile() {
        for grid in [&LARGE, &SMALL, &SYMBOLIC] {
            let inner = grid.tile.inset(grid.edge);
            for (m, _) in grid.marks {
                assert!(
                    m.x > inner.x
                        && m.y > inner.y
                        && m.x + m.w < inner.x + inner.w
                        && m.y + m.h < inner.y + inner.h,
                    "{m:?} reaches the edge"
                );
                assert!(m.r * 2.0 <= m.w.min(m.h), "{m:?} is rounded past a pill");
            }
        }
    }

    #[test]
    fn the_meters_are_centred_on_the_tile() {
        for grid in [&LARGE, &SMALL, &SYMBOLIC] {
            let top = grid.marks.iter().map(|(m, _)| m.y).fold(f32::MAX, f32::min);
            let bottom = grid
                .marks
                .iter()
                .map(|(m, _)| m.y + m.h)
                .fold(0.0, f32::max);
            let left = grid.marks.iter().map(|(m, _)| m.x).fold(f32::MAX, f32::min);
            let right = grid
                .marks
                .iter()
                .map(|(m, _)| m.x + m.w)
                .fold(0.0, f32::max);
            let mid = grid.size / 2.0;
            assert_eq!((top + bottom) / 2.0, mid, "off centre vertically");
            assert_eq!((left + right) / 2.0, mid, "off centre horizontally");
        }
    }

    #[test]
    fn the_small_icons_land_on_whole_pixels() {
        let shapes = std::iter::once(SMALL.tile)
            .chain(SMALL.marks.iter().chain(SYMBOLIC.marks).map(|(s, _)| *s));
        for s in shapes {
            for edge in [s.x, s.y, s.w, s.h, SMALL.edge] {
                assert_eq!(edge.fract(), 0.0, "{s:?} is off the pixel grid");
            }
        }
    }

    #[test]
    fn symbolic_tracks_and_levels_do_not_overlap() {
        // Overlapping half-opacity paths would darken where they meet.
        for (i, (a, _)) in SYMBOLIC.marks.iter().enumerate() {
            for (b, _) in &SYMBOLIC.marks[i + 1..] {
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    fn luminance(Rgba(r, g, b, _): Rgba) -> f32 {
        let channel = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    #[test]
    fn the_symbolic_accents_show_on_light_and_dark_panels() {
        // Breeze Light and Breeze Dark panel backgrounds.
        for panel in [Rgba(239, 240, 241, 255), Rgba(32, 35, 38, 255)] {
            for accent in [TRAY_GREEN, TRAY_AMBER] {
                let (a, b) = (luminance(accent) + 0.05, luminance(panel) + 0.05);
                let contrast = a.max(b) / a.min(b);
                assert!(
                    contrast >= 3.0,
                    "{accent:?} on {panel:?} is only {contrast:.2}:1"
                );
            }
        }
    }

    #[test]
    fn the_checked_in_icons_are_the_ones_drawn_here() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let bless = std::env::var_os("VINOWATCH_BLESS_ICONS").is_some();
        for (name, svg) in files() {
            let path = data.join(Path::new(&name).file_name().unwrap());
            if bless {
                std::fs::write(&path, &svg).unwrap();
            }
            let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                on_disk == svg,
                "{} is stale; regenerate it with VINOWATCH_BLESS_ICONS=1 cargo test",
                path.display()
            );
        }
    }

    #[test]
    fn install_writes_a_hicolor_tree() {
        let dir = std::env::temp_dir().join(format!("vinowatch-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = install(&dir).unwrap();
        assert_eq!(written.len(), 4);
        for name in [
            "scalable/apps/vinowatch.svg",
            "symbolic/apps/vinowatch-symbolic.svg",
            "symbolic/apps/vinowatch-active-symbolic.svg",
            "symbolic/apps/vinowatch-low-memory-symbolic.svg",
        ] {
            assert!(dir.join("icons/hicolor").join(name).is_file(), "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
