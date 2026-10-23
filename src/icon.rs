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
// Opaque enough that a meter's empty length shows against the tile (3:1),
// and faint enough that its green level still shows against the track (3:1).
const TRACK: Rgba = Rgba(160, 164, 172, 140);
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

/// The symbolic icon's outline: vinoWhisper's and vinoPop's tray frame, path
/// for path, so the vino icons sit together in the panel.
const FRAME: &str = "M3 2h10a2 2 0 0 1 2 2v8a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2v-8a2 2 0 0 1 2 -2z\
                     M3 3h10a1 1 0 0 1 1 1v8a1 1 0 0 1 -1 1h-10a1 1 0 0 1 -1 -1v-8a1 1 0 0 1 1 -1z";

/// 16-unit design for the symbolic tray icon: inside `FRAME`, three 1-unit
/// meters in vinoWhisper's line style, each level a gap away from its track.
const SYMBOLIC: Grid = Grid {
    tile: Shape::new(0.0, 0.0, 16.0, 16.0, 0.0),
    edge: 0.0,
    marks: &[
        (Shape::new(4.0, 5.0, 4.0, 1.0, 0.0), Part::Level),
        (Shape::new(9.0, 5.0, 3.0, 1.0, 0.0), Part::Track),
        (Shape::new(4.0, 8.0, 6.0, 1.0, 0.0), Part::Level),
        (Shape::new(11.0, 8.0, 1.0, 1.0, 0.0), Part::Track),
        (Shape::new(4.0, 11.0, 2.0, 1.0, 0.0), Part::Level),
        (Shape::new(7.0, 11.0, 5.0, 1.0, 0.0), Part::Track),
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
/// level follow the theme; the green and amber levels stay fixed. The tracks
/// are at 0.6, not 0.5: half of Breeze Light's text colour falls just short
/// of 3:1 on its panel.
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
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 16 16\">\n  \
         <style id=\"current-color-scheme\" type=\"text/css\">.ColorScheme-Text {{ color: #232629; }}</style>\n  \
         <path class=\"ColorScheme-Text\" style=\"fill:currentColor\" fill-rule=\"evenodd\" d=\"{FRAME}\"/>\n",
    );
    for (path, style) in [
        (text, "fill:currentColor".to_string()),
        (dim, "fill:currentColor;fill-opacity:0.6".to_string()),
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
    draw_tile(canvas, grid, unit);
    for &(shape, part) in grid.marks {
        canvas.fill_rounded(shape.rect(unit), shape.r * unit, paint(state, part));
    }
}

fn draw_tile(canvas: &mut Canvas, grid: &Grid, unit: f32) {
    let tile = grid.tile.rect(unit);
    canvas.fill_rounded(tile, grid.tile.r * unit, TILE);
    canvas.stroke_rounded(tile, grid.tile.r * unit, grid.edge * unit, EDGE);
}

/// Menu row icons are drawn at this size and scaled down by the host: a 16 px
/// menu icon is 24 px at 150%, 32 px at 200%.
const ROW_ICON: u32 = 48;

/// The one meter of a single-level row icon: the three meters' span, twice as
/// thick.
const WIDE_METER: Shape = Shape::new(14.0, 27.0, 36.0, 10.0, 5.0);

/// The meters of a row icon on the 64-unit grid: each track, and the level
/// over it when there is one. `levels` are 0 to 1, one meter or three.
fn meters(levels: &[f32]) -> Vec<(Shape, Option<Shape>)> {
    let tracks: Vec<Shape> = if levels.len() == 1 {
        vec![WIDE_METER]
    } else {
        LARGE
            .marks
            .iter()
            .filter(|(_, part)| *part == Part::Track)
            .map(|(shape, _)| *shape)
            .collect()
    };
    tracks
        .into_iter()
        .zip(levels)
        .map(|(track, &level)| {
            // In 2-unit steps, so a level that barely moves doesn't redraw the
            // menu; anything above zero shows at least as a dot.
            let steps = track.w / 2.0;
            let filled = (level.clamp(0.0, 1.0) * steps).round() / steps * track.w;
            let level = (level > 0.0).then(|| Shape {
                w: filled.max(track.h),
                ..track
            });
            (track, level)
        })
        .collect()
}

/// A menu row's icon, as PNG: the tile with one meter per level, filled in
/// `state`'s colour, so a row shows its own numbers at a glance.
pub fn meter_png(state: State, levels: &[f32]) -> Vec<u8> {
    crate::png::encode(ROW_ICON, ROW_ICON, &meter_rgba(state, levels))
}

/// `meter_png`'s pixels, straight RGBA.
fn meter_rgba(state: State, levels: &[f32]) -> Vec<u8> {
    let mut pixels = vec![0u8; (ROW_ICON * ROW_ICON * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, ROW_ICON, ROW_ICON);
    let unit = ROW_ICON as f32 / LARGE.size;
    draw_tile(&mut canvas, &LARGE, unit);
    for (track, level) in meters(levels) {
        canvas.fill_rounded(track.rect(unit), track.r * unit, TRACK);
        if let Some(level) = level {
            let colour = paint(state, Part::Level);
            canvas.fill_rounded(level.rect(unit), level.r * unit, colour);
        }
    }
    canvas.to_rgba()
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
    fn row_meters_follow_their_levels() {
        let levels = |input: &[f32]| -> Vec<Option<f32>> {
            meters(input)
                .into_iter()
                .map(|(_, l)| l.map(|l| l.w))
                .collect()
        };
        assert_eq!(levels(&[0.0, 1.0, 0.5]), [None, Some(36.0), Some(18.0)]);
        // Over-full is full; a sliver is a round dot, not nothing.
        assert_eq!(levels(&[2.0, 0.001, 0.0]), [Some(36.0), Some(6.0), None]);
        assert_eq!(levels(&[0.25]), [Some(10.0)]);
        // A level that barely moves draws the same icon.
        assert_eq!(
            meter_png(State::Active, &[0.5]),
            meter_png(State::Active, &[0.51])
        );
        assert_ne!(
            meter_png(State::Active, &[0.5]),
            meter_png(State::Idle, &[0.5])
        );
    }

    #[test]
    fn the_wide_meter_is_centred_in_the_three() {
        let (top, bottom) = (LARGE.marks[0].0.y, LARGE.marks[5].0.y + LARGE.marks[5].0.h);
        assert_eq!(WIDE_METER.y + WIDE_METER.h / 2.0, (top + bottom) / 2.0);
        assert_eq!(WIDE_METER.r * 2.0, WIDE_METER.h);
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

    /// The inside of `FRAME`.
    const FRAME_INNER: Shape = Shape::new(2.0, 3.0, 12.0, 10.0, 1.0);

    #[test]
    fn the_symbolic_meters_sit_inside_the_frame() {
        let inner = FRAME_INNER.inset(1.0);
        for (m, _) in SYMBOLIC.marks {
            assert!(
                m.x >= inner.x
                    && m.y >= inner.y
                    && m.x + m.w <= inner.x + inner.w
                    && m.y + m.h <= inner.y + inner.h,
                "{m:?} touches the frame"
            );
        }
        assert!(FRAME.contains("M3 2h10a2 2 0 0 1 2 2v8"));
    }

    #[test]
    fn the_meters_are_centred_on_the_tile() {
        for grid in [&LARGE, &SMALL] {
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

    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let (a, b) = (luminance(a) + 0.05, luminance(b) + 0.05);
        a.max(b) / a.min(b)
    }

    /// `top` at `alpha` over `under`, blended in sRGB as SVG renderers do.
    fn over(top: Rgba, alpha: f32, under: Rgba) -> Rgba {
        let mix =
            |t: u8, u: u8| (f32::from(t) * alpha + f32::from(u) * (1.0 - alpha)).round() as u8;
        Rgba(
            mix(top.0, under.0),
            mix(top.1, under.1),
            mix(top.2, under.2),
            255,
        )
    }

    /// Panel backgrounds, each with the text colour the theme recolours a
    /// symbolic icon to: Breeze Light, plain white, Breeze Dark (Plasma 6
    /// and 5), GNOME's black top bar, and waybar's default.
    const PANELS: [(Rgba, Rgba); 6] = [
        (Rgba(239, 240, 241, 255), Rgba(35, 38, 41, 255)),
        (Rgba(255, 255, 255, 255), Rgba(0, 0, 0, 255)),
        (Rgba(32, 35, 38, 255), Rgba(252, 252, 252, 255)),
        (Rgba(49, 54, 59, 255), Rgba(252, 252, 252, 255)),
        (Rgba(0, 0, 0, 255), Rgba(255, 255, 255, 255)),
        (Rgba(43, 48, 59, 255), Rgba(255, 255, 255, 255)),
    ];

    /// The pixel in `area` (in pixels) that stands out most from `against`.
    fn best(pixel: &dyn Fn(u32, u32) -> Rgba, area: Rect, against: Rgba) -> Rgba {
        (area.y0 as u32..area.y1.ceil() as u32)
            .flat_map(|y| (area.x0 as u32..area.x1.ceil() as u32).map(move |x| (x, y)))
            .map(|(x, y)| pixel(x, y))
            .max_by(|a, b| contrast(*a, against).total_cmp(&contrast(*b, against)))
            .unwrap()
    }

    /// One meter, drawn: its level (if any) against the track, and the
    /// track's empty end against the tile, both 3:1 (WCAG 1.4.11). The tile is
    /// opaque and surrounds every meter, so the panel never touches them and
    /// this holds on any panel.
    fn check_meter(
        what: &str,
        pixel: &dyn Fn(u32, u32) -> Rgba,
        track: Shape,
        level: Option<Shape>,
        unit: f32,
    ) {
        // The empty end, clear of the level and its anti-aliased edge.
        let from = level.map_or(track.x, |l| l.x + l.w + 1.0);
        let end = Shape {
            x: from,
            w: track.x + track.w - from,
            ..track
        };
        let empty = best(pixel, end.rect(unit), TILE);
        let on_tile = contrast(empty, TILE);
        assert!(
            on_tile >= 3.0,
            "{what}: track is {on_tile:.2}:1 on the tile"
        );
        let mut marks = vec![("track", empty)];
        if let Some(level) = level {
            let filled = best(pixel, level.rect(unit), empty);
            let c = contrast(filled, empty);
            assert!(c >= 3.0, "{what}: level is {c:.2}:1 on its track");
            marks.push(("level", filled));
        }
        for (part, colour) in marks {
            assert_eq!(colour.3, 255, "{what}: {part} is see-through");
        }
    }

    #[test]
    fn the_pixmap_meters_stand_out_on_any_panel() {
        for state in State::ALL {
            for icon in tray_icons(state) {
                let size = icon.width as u32;
                let grid = if size < 32 { &SMALL } else { &LARGE };
                let pixel = |x: u32, y: u32| {
                    let i = ((y * size + x) * 4) as usize;
                    let [a, r, g, b] = icon.data[i..i + 4] else {
                        unreachable!()
                    };
                    Rgba(r, g, b, a)
                };
                for pair in grid.marks.chunks(2) {
                    let [(track, Part::Track), (level, Part::Level)] = pair else {
                        panic!("{pair:?} is not a track then its level")
                    };
                    let what = format!("{state:?} {size}px");
                    check_meter(&what, &pixel, *track, Some(*level), size as f32 / grid.size);
                }
            }
        }
        for (panel, _) in PANELS.iter().filter(|(p, _)| luminance(*p) > 0.5) {
            assert!(
                contrast(TILE, *panel) >= 3.0,
                "the tile vanishes on {panel:?}"
            );
        }
    }

    #[test]
    fn the_menu_row_meters_stand_out_on_any_panel() {
        for state in State::ALL {
            for levels in [&[0.5][..], &[0.0, 0.3, 1.0], &[0.05, 0.6, 0.0]] {
                let rgba = meter_rgba(state, levels);
                let pixel = |x: u32, y: u32| {
                    let i = ((y * ROW_ICON + x) * 4) as usize;
                    let [r, g, b, a] = rgba[i..i + 4] else {
                        unreachable!()
                    };
                    Rgba(r, g, b, a)
                };
                for (track, level) in meters(levels) {
                    // A full meter has no empty end to check.
                    if level.is_some_and(|l| l.w >= track.w - 1.0) {
                        continue;
                    }
                    let what = format!("{state:?} {levels:?}");
                    check_meter(&what, &pixel, track, level, ROW_ICON as f32 / LARGE.size);
                }
            }
        }
    }

    /// The symbolic icons have no tile: every path, as the theme recolours it,
    /// must clear 3:1 against the panel, including the half-opacity tracks.
    #[test]
    fn the_symbolic_icons_show_on_light_and_dark_panels() {
        for state in State::ALL {
            let svg = symbolic_svg(state);
            let styles: Vec<&str> = svg
                .split("style=\"")
                .skip(1)
                .map(|s| s.split('"').next().unwrap())
                .collect();
            assert!(!styles.is_empty(), "{state:?} draws nothing");
            for (panel, text) in PANELS {
                for style in &styles {
                    let fill = if style.contains("currentColor") {
                        let alpha = style
                            .split("fill-opacity:")
                            .nth(1)
                            .map_or(1.0, |a| a.parse().unwrap());
                        over(text, alpha, panel)
                    } else {
                        let hex = style.strip_prefix("fill:#").unwrap();
                        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
                        Rgba(byte(0), byte(2), byte(4), 255)
                    };
                    let c = contrast(fill, panel);
                    assert!(c >= 3.0, "{state:?} {style} is {c:.2}:1 on {panel:?}");
                }
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
