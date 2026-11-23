//! The tray and app icon: a round dial. A dark disc with a gauge arc open at the
//! bottom, its level and a hub in the state's colour. Deliberately not the
//! rounded tile with pill marks the other vino apps use, so vinoWatch is not
//! mistaken for vinoWhisper in a panel. Drawn in code so the tray needs no image
//! decoder, and the checked-in SVGs in `data/` are generated from the same
//! numbers. Menu rows keep meter bars (they carry the numbers), on the same disc.

use std::f32::consts::PI;
use std::io;
use std::path::{Path, PathBuf};

use crate::raster::{Canvas, Rect, Rgba, arc_coverage, disc_coverage};

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

    fn rect(self, unit: f32) -> Rect {
        Rect {
            x0: self.x * unit,
            y0: self.y * unit,
            x1: (self.x + self.w) * unit,
            y1: (self.y + self.h) * unit,
        }
    }
}

/// What the icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No OpenVINO processes.
    Idle,
    /// At least one OpenVINO process is running.
    Active,
    /// Available memory is under the `--warn-below` share, or the driver is stuck.
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// The empty length of a meter or the dial's arc.
    Track,
    /// The filled length.
    Level,
}

/// Its contrast is pinned by `tests::the_pixmap_dial_stands_out_on_any_panel`,
/// `tests::the_symbolic_icons_show_on_light_and_dark_panels`.
const TILE: Rgba = Rgba(6, 10, 20, 255);
const EDGE: Rgba = Rgba(255, 255, 255, 46);
const WHITE: Rgba = Rgba(250, 250, 250, 255);
const TRACK: Rgba = Rgba(165, 175, 200, 132);
const GREEN: Rgba = Rgba(86, 204, 120, 255);
const AMBER: Rgba = Rgba(245, 184, 66, 255);

const PANEL_GREEN: Rgba = Rgba(40, 150, 80, 255);
const PANEL_AMBER: Rgba = Rgba(184, 115, 0, 255);

fn paint(state: State, part: Part) -> Rgba {
    match (state, part) {
        (_, Part::Track) => TRACK,
        (State::Idle, Part::Level) => WHITE,
        (State::Active, Part::Level) => GREEN,
        (State::Warning, Part::Level) => AMBER,
    }
}

/// A dial's geometry on a grid of `size` units. Angles run clockwise from
/// east, in degrees, as SVG's y axis does.
#[derive(Debug, Clone, Copy)]
struct Dial {
    size: f32,
    /// The disc behind the gauge; 0 for the symbolic icons, which have none.
    disc: f32,
    edge: f32,
    arc_radius: f32,
    arc_width: f32,
    hub: f32,
}

impl Dial {
    fn centre(self) -> f32 {
        self.size / 2.0
    }

    /// The point on the arc's centreline at `degrees`.
    fn on_arc(self, degrees: f32) -> (f32, f32) {
        let radians = degrees * PI / 180.0;
        (
            self.centre() + self.arc_radius * radians.cos(),
            self.centre() + self.arc_radius * radians.sin(),
        )
    }

    /// An SVG arc along the centreline from `from` for `sweep` degrees.
    fn svg_arc(self, from: f32, sweep: f32) -> String {
        let (x0, y0) = self.on_arc(from);
        let (x1, y1) = self.on_arc(from + sweep);
        let (r, large) = (self.arc_radius, u8::from(sweep > 180.0));
        format!("M{x0:.2} {y0:.2}A{r} {r} 0 {large} 1 {x1:.2} {y1:.2}")
    }
}

/// 64-unit design, drawn at every tray size and as the app icon.
const DIAL: Dial = Dial {
    size: 64.0,
    disc: 28.0,
    edge: 2.0,
    arc_radius: 17.0,
    arc_width: 7.0,
    hub: 4.0,
};

/// 16-unit design for the symbolic tray icon: no disc, so the panel shows
/// through and the theme can recolour it.
const SYMBOLIC: Dial = Dial {
    size: 16.0,
    disc: 0.0,
    edge: 0.0,
    arc_radius: 5.5,
    arc_width: 1.5,
    hub: 1.25,
};

/// The gauge opens at the bottom: it starts at the lower left and sweeps
/// clockwise over the top to the lower right.
const ARC_START: f32 = 135.0;
const ARC_SWEEP: f32 = 270.0;

/// How far round the level is drawn. The tray icon is a still image, so the
/// state is its colour; this only gives the arc a filled part.
const LEVEL: f32 = 0.65;

/// Degrees left clear between the symbolic icon's level and its track, so the
/// two round caps don't overlap (they are drawn at half opacity).
const SYMBOLIC_GAP: f32 = 28.0;

fn hex(Rgba(r, g, b, _): Rgba) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn opacity(Rgba(_, _, _, a): Rgba) -> String {
    format!("{:.2}", f32::from(a) / 255.0)
}

/// A paint attribute pair (`fill` or `stroke`), with an opacity only where the
/// colour needs it.
fn svg_paint(attr: &str, color: Rgba) -> String {
    if color.3 == u8::MAX {
        format!(r#"{attr}="{}""#, hex(color))
    } else {
        format!(
            r#"{attr}="{}" {attr}-opacity="{}""#,
            hex(color),
            opacity(color)
        )
    }
}

/// The installed app icon (`vinowatch.svg`): the active state, with the
/// viewBox cropped to a 2-unit margin so the disc isn't visibly smaller than
/// other apps' icons in a task manager; the tray pixmaps keep the 4-unit
/// margin, since they're drawn at their own size.
pub fn app_svg() -> String {
    let d = DIAL;
    let (c, edge) = (d.centre(), d.edge);
    let mut svg =
        String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"2 2 60 60\">\n");
    let mut line = |element: String| {
        svg.push_str("  ");
        svg.push_str(&element);
        svg.push('\n');
    };
    line(format!(
        r#"<circle cx="{c}" cy="{c}" r="{}" fill="{}"/>"#,
        d.disc,
        hex(TILE)
    ));
    line(format!(
        r#"<circle cx="{c}" cy="{c}" r="{}" fill="none" stroke="{}" stroke-opacity="{}" stroke-width="{edge}"/>"#,
        d.disc - edge / 2.0,
        hex(EDGE),
        opacity(EDGE)
    ));
    let cap = r#"fill="none" stroke-linecap="round""#;
    let width = d.arc_width;
    line(format!(
        r#"<path d="{}" {cap} stroke-width="{width}" {}/>"#,
        d.svg_arc(ARC_START, ARC_SWEEP),
        svg_paint("stroke", paint(State::Active, Part::Track))
    ));
    line(format!(
        r#"<path d="{}" {cap} stroke-width="{width}" {}/>"#,
        d.svg_arc(ARC_START, ARC_SWEEP * LEVEL),
        svg_paint("stroke", paint(State::Active, Part::Level))
    ));
    line(format!(
        r#"<circle cx="{c}" cy="{c}" r="{}" {}/>"#,
        d.hub,
        svg_paint("fill", paint(State::Active, Part::Level))
    ));
    svg.push_str("</svg>\n");
    svg
}

impl State {
    pub const ALL: [State; 3] = [State::Idle, State::Active, State::Warning];

    /// The themed icon name the tray asks for, as vinoWhisper's tray does:
    /// Plasma recolours a `-symbolic` icon to the panel's text colour, and the
    /// pixmaps below are only the fallback when the theme has none.
    pub fn icon_name(self) -> &'static str {
        match self {
            State::Idle => "vinowatch-symbolic",
            State::Active => "vinowatch-active-symbolic",
            State::Warning => "vinowatch-low-memory-symbolic",
        }
    }
}

/// A monochrome tray icon for one state, in the KDE symbolic format: the
/// `ColorScheme-Text` class is what Plasma recolours. The track and an idle
/// level follow the theme; the green and amber levels stay fixed. The track is
/// at 0.6, not 0.5: half of Breeze Light's text colour falls just short of 3:1
/// on its panel.
pub fn symbolic_svg(state: State) -> String {
    let d = SYMBOLIC;
    let level_sweep = ARC_SWEEP * LEVEL - SYMBOLIC_GAP / 2.0;
    let track_from = ARC_START + ARC_SWEEP * LEVEL + SYMBOLIC_GAP / 2.0;
    let track_sweep = ARC_START + ARC_SWEEP - track_from;
    let accent = hex(if state == State::Warning {
        PANEL_AMBER
    } else {
        PANEL_GREEN
    });
    let themed = state == State::Idle;
    let (class, level_colour) = if themed {
        (r#" class="ColorScheme-Text""#, "currentColor".to_string())
    } else {
        ("", accent)
    };
    let stroke = format!(
        "fill:none;stroke-width:{};stroke-linecap:round",
        d.arc_width
    );
    let c = d.centre();
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 16 16\">\n  \
         <style id=\"current-color-scheme\" type=\"text/css\">.ColorScheme-Text {{ color: #232629; }}</style>\n  \
         <path class=\"ColorScheme-Text\" style=\"{stroke};stroke:currentColor;stroke-opacity:0.6\" d=\"{track}\"/>\n  \
         <path{class} style=\"{stroke};stroke:{level_colour}\" d=\"{level}\"/>\n  \
         <circle{class} style=\"fill:{level_colour}\" cx=\"{c}\" cy=\"{c}\" r=\"{hub}\"/>\n\
         </svg>\n",
        track = d.svg_arc(track_from, track_sweep),
        level = d.svg_arc(ARC_START, level_sweep),
        hub = d.hub,
    )
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

/// Every size a tray might ask for.
const TRAY_SIZES: [u32; 6] = [16, 22, 24, 32, 48, 64];

pub fn tray_icons(state: State) -> Vec<ksni::Icon> {
    TRAY_SIZES
        .into_iter()
        .map(|size| tray_icon(state, size))
        .collect()
}

fn tray_icon(state: State, size: u32) -> ksni::Icon {
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    let mut canvas = Canvas::new(&mut pixels, size, size);
    draw_dial(&mut canvas, state, size as f32 / DIAL.size);
    ksni::Icon {
        width: size as i32,
        height: size as i32,
        data: canvas.to_argb32_be(),
    }
}

fn draw_dial(canvas: &mut Canvas, state: State, unit: f32) {
    let d = DIAL;
    draw_tile(canvas, unit);
    let (c, radius, width) = (d.centre() * unit, d.arc_radius * unit, d.arc_width * unit);
    let bounds = Rect {
        x0: c - radius - width,
        y0: c - radius - width,
        x1: c + radius + width,
        y1: c + radius + width,
    };
    for (sweep, part) in [(ARC_SWEEP, Part::Track), (ARC_SWEEP * LEVEL, Part::Level)] {
        canvas.cover(bounds, paint(state, part), |x, y| {
            arc_coverage(x, y, c, radius, width, ARC_START, sweep)
        });
    }
    let hub = Rect {
        x0: c - d.hub * unit - 1.0,
        y0: c - d.hub * unit - 1.0,
        x1: c + d.hub * unit + 1.0,
        y1: c + d.hub * unit + 1.0,
    };
    canvas.cover(hub, paint(state, Part::Level), |x, y| {
        disc_coverage(x, y, c, d.hub * unit)
    });
}

/// The dark disc and its faint edge, also behind each menu row's meters.
fn draw_tile(canvas: &mut Canvas, unit: f32) {
    let d = DIAL;
    let (c, radius, edge) = (d.centre() * unit, d.disc * unit, d.edge * unit);
    let bounds = Rect {
        x0: c - radius,
        y0: c - radius,
        x1: c + radius,
        y1: c + radius,
    };
    canvas.cover(bounds, TILE, |x, y| disc_coverage(x, y, c, radius));
    canvas.cover(bounds, EDGE, |x, y| {
        disc_coverage(x, y, c, radius) - disc_coverage(x, y, c, radius - edge)
    });
}

/// Menu row icons are drawn at this size and scaled down by the host: a 16 px
/// menu icon is 24 px at 150%, 32 px at 200%.
const ROW_ICON: u32 = 48;

/// The tracks of a three-meter row icon on the 64-unit grid.
const ROW_TRACKS: [Shape; 3] = [
    Shape::new(14.0, 18.0, 36.0, 6.0, 3.0),
    Shape::new(14.0, 29.0, 36.0, 6.0, 3.0),
    Shape::new(14.0, 40.0, 36.0, 6.0, 3.0),
];

/// The one meter of a single-level row icon: the three meters' span, twice as
/// thick.
const WIDE_METER: Shape = Shape::new(14.0, 27.0, 36.0, 10.0, 5.0);

/// The meters of a row icon on the 64-unit grid: each track, and the level
/// over it when there is one. `levels` are 0 to 1, one meter or three.
fn meters(levels: &[f32]) -> Vec<(Shape, Option<Shape>)> {
    let tracks: Vec<Shape> = if levels.len() == 1 {
        vec![WIDE_METER]
    } else {
        ROW_TRACKS.to_vec()
    };
    tracks
        .into_iter()
        .zip(levels)
        .map(|(track, &level)| (track, filled_part(track, level)))
        .collect()
}

/// A level moves in steps this wide, so one that barely changes draws the
/// same icon and the menu isn't redrawn (`row_meters_follow_their_levels`).
const LEVEL_STEP: f32 = 2.0;

/// The filled start of `track` for a level of 0 to 1: nothing at zero, and
/// otherwise at least a round dot.
fn filled_part(track: Shape, level: f32) -> Option<Shape> {
    let steps = track.w / LEVEL_STEP;
    let filled = (level.clamp(0.0, 1.0) * steps).round() / steps * track.w;
    (level > 0.0).then(|| Shape {
        w: filled.max(track.h),
        ..track
    })
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
    let unit = ROW_ICON as f32 / DIAL.size;
    draw_tile(&mut canvas, unit);
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
        assert_eq!(levels(&[0.25]), [Some(10.0)]);
    }

    #[test]
    fn an_overfull_meter_is_full_and_a_sliver_is_a_round_dot_not_nothing() {
        let levels: Vec<_> = meters(&[2.0, 0.001, 0.0])
            .into_iter()
            .map(|(_, l)| l.map(|l| l.w))
            .collect();
        assert_eq!(levels, [Some(36.0), Some(6.0), None]);
    }

    #[test]
    fn a_level_that_barely_moves_draws_the_same_icon() {
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
        let (top, bottom) = (ROW_TRACKS[0].y, ROW_TRACKS[2].y + ROW_TRACKS[2].h);
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
    fn the_row_meters_sit_inside_the_disc() {
        let inner = DIAL.disc - DIAL.edge;
        for m in ROW_TRACKS.iter().chain([&WIDE_METER]) {
            assert!(m.r * 2.0 <= m.w.min(m.h), "{m:?} is rounded past a pill");
            for (x, y) in [
                (m.x, m.y),
                (m.x + m.w, m.y),
                (m.x, m.y + m.h),
                (m.x + m.w, m.y + m.h),
            ] {
                let from_centre = (x - DIAL.centre()).hypot(y - DIAL.centre());
                assert!(from_centre < inner, "{m:?} reaches the edge");
            }
        }
    }

    #[test]
    fn the_dial_sits_inside_the_disc_and_the_symbolic_one_inside_its_grid() {
        const {
            let reach = DIAL.arc_radius + DIAL.arc_width / 2.0;
            assert!(reach < DIAL.disc - DIAL.edge, "the arc reaches the edge");
            assert!(DIAL.hub < DIAL.arc_radius - DIAL.arc_width / 2.0);
            let reach = SYMBOLIC.arc_radius + SYMBOLIC.arc_width / 2.0;
            assert!(reach < SYMBOLIC.size / 2.0, "the symbolic arc is clipped");
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

    /// The track's end clear of the level and its anti-aliased edge.
    fn empty_end(track: Shape, level: Option<Shape>) -> Shape {
        let from = level.map_or(track.x, |l| l.x + l.w + 1.0);
        Shape {
            x: from,
            w: track.x + track.w - from,
            ..track
        }
    }

    /// A full meter has no empty end to check.
    fn is_full(track: Shape, level: Option<Shape>) -> bool {
        level.is_some_and(|l| l.w >= track.w - 1.0)
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
        let empty = best(pixel, empty_end(track, level).rect(unit), TILE);
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

    /// The dial's pixel at `degrees` round the arc, `size` px wide.
    fn arc_pixel(icon: &ksni::Icon, degrees: f32) -> Rgba {
        let size = icon.width as u32;
        let (x, y) = DIAL.on_arc(degrees);
        let unit = size as f32 / DIAL.size;
        let i = (((y * unit) as u32 * size + (x * unit) as u32) * 4) as usize;
        let [a, r, g, b] = icon.data[i..i + 4] else {
            unreachable!()
        };
        Rgba(r, g, b, a)
    }

    #[test]
    fn the_pixmap_dial_stands_out_on_any_panel() {
        for state in State::ALL {
            for icon in tray_icons(state) {
                let what = format!("{state:?} {}px", icon.width);
                let level = arc_pixel(&icon, ARC_START + ARC_SWEEP * LEVEL / 2.0);
                let empty = arc_pixel(&icon, ARC_START + ARC_SWEEP * (1.0 + LEVEL) / 2.0);
                let on_tile = contrast(empty, TILE);
                assert!(
                    on_tile >= 3.0,
                    "{what}: track is {on_tile:.2}:1 on the tile"
                );
                let on_track = contrast(level, empty);
                assert!(
                    on_track >= 3.0,
                    "{what}: level is {on_track:.2}:1 on its track"
                );
                assert_eq!((level.3, empty.3), (255, 255), "{what}: see-through");
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
                    if is_full(track, level) {
                        continue;
                    }
                    let what = format!("{state:?} {levels:?}");
                    check_meter(&what, &pixel, track, level, ROW_ICON as f32 / DIAL.size);
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
            let value = |style: &str, key: &str| -> Option<String> {
                let rest = style.split(&format!("{key}:")).nth(1)?;
                Some(rest.split(';').next().unwrap().to_string())
            };
            for (panel, text) in PANELS {
                for style in &styles {
                    // Arcs are stroked, the hub is filled.
                    let key = if style.contains("stroke:") {
                        "stroke"
                    } else {
                        "fill"
                    };
                    let colour = value(style, key).unwrap();
                    let fill = if colour == "currentColor" {
                        let alpha = value(style, &format!("{key}-opacity"))
                            .map_or(1.0, |a| a.parse().unwrap());
                        over(text, alpha, panel)
                    } else {
                        let hex = colour.strip_prefix('#').unwrap();
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
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path();
        let written = install(dir).unwrap();
        assert_eq!(written.len(), 4);
        for name in [
            "scalable/apps/vinowatch.svg",
            "symbolic/apps/vinowatch-symbolic.svg",
            "symbolic/apps/vinowatch-active-symbolic.svg",
            "symbolic/apps/vinowatch-low-memory-symbolic.svg",
        ] {
            assert!(dir.join("icons/hicolor").join(name).is_file(), "{name}");
        }
    }

    /// `install.sh --uninstall` lists the icons by hand; a new one added
    /// here and not there would be left behind.
    #[test]
    fn install_sh_uninstalls_every_icon() {
        let script = include_str!("../install.sh");
        let uninstall = &script[script.find("--uninstall\" ]").expect("uninstall branch")..];
        let uninstall = &uninstall[..uninstall.find("exit 0").unwrap()];
        for (name, _) in files() {
            assert!(
                uninstall.contains(&format!("\"$hicolor/{name}\"")),
                "install.sh --uninstall doesn't remove {name}"
            );
        }
    }
}
