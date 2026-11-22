//! Map's look, in one place: every colour and size the design (docs/design.md) takes from the
//! observed Xaero record, and the pure layout functions the drawing code places things with.
//! Nothing here touches the game, so all of it is tested on its own. Sizes are GUI pixels.

/// `[x, y, w, h]`.
pub type Rect = [f32; 4];
/// A point `[x, y]`.
pub type Point = [f32; 2];

// ---- Minimap -----------------------------------------------------------------------------------

/// The map area's default side and its distance from the screen corner.
pub const MAP_SIDE: f32 = 135.0;
pub const MAP_MARGIN: f32 = 5.0;

/// The frame: four 1-GUI bands outside the map, outer first. Light bands sit on the top and
/// left, dark bands on the bottom and right; the circle uses the same colours.
pub const FRAME_BAND: f32 = 1.0;
pub const FRAME_LIGHT: [u32; 4] = [0xFF00_0000, 0xFF93_9393, 0xFF5D_5D5D, 0xFF10_1010];
pub const FRAME_DARK: [u32; 4] = [0xFF00_0000, 0xFF47_4747, 0xFF7E_7E7E, 0xFF2B_2B2B];
/// The whole frame's thickness.
pub const FRAME: f32 = FRAME_BAND * 4.0;

/// Compass letters: half-GUI font, white with a blue shadow, on a translucent blue box that
/// rides the inside of the frame.
pub const COMPASS_TEXT: u32 = 0xFFFF_FFFF;
pub const COMPASS_SHADOW: u32 = 0xFF55_55FF;
pub const COMPASS_BOX: u32 = 0x8F55_55FF;
pub const COMPASS_BOX_SIDE: f32 = 4.5;
pub const COMPASS_INSET: f32 = 2.5;
/// The letters' font scale: one font pixel per screen pixel at GUI scale 2.
pub const COMPASS_FONT: f32 = 0.5;

/// The player on a north-locked map, and its soft dark rim.
pub const ARROW: u32 = 0xFFCC_1919;
pub const ARROW_RIM: u32 = 0x8C00_0000;
/// The arrow's default scale (Xaero "Arrow Scale: 1.5x").
pub const ARROW_SCALE: f32 = 1.5;
/// The crosshair on a rotating map: white arms, a lilac centre.
pub const CROSSHAIR: u32 = 0xFFFF_FFFF;
pub const CROSSHAIR_CENTRE: u32 = 0xFFB3_84C3;
pub const CROSSHAIR_SIDE: f32 = 5.0;

/// Entity heads: one texel per GUI pixel, a one-screen-pixel black outline, fixed size.
pub const HEAD_OUTLINE: u32 = 0xFF00_0000;
/// Beyond this height difference an entity is not drawn.
pub const HEIGHT_LIMIT: f32 = 20.0;
/// A dropped item's sprite side (Xaero draws a diamond 14 × 15 px).
pub const ITEM_SIDE: f32 = 7.5;

/// Waypoint squares on the minimap.
pub const WAYPOINT_SIDE: f32 = 9.0;
pub const WAYPOINT_ALPHA: f32 = 0.9;
/// The initial's colours on a waypoint square or pennant.
pub const INITIAL_TEXT: u32 = 0xFFFF_FFFF;
pub const INITIAL_SHADOW: u32 = 0xFF3F_3F3F;
/// A death point's square.
pub const DEATH_FILL: u32 = 0xFF00_0000;

/// Info lines under the map: white with the vanilla shadow, one fact per line.
pub const INFO_TEXT: u32 = 0xFFFF_FFFF;
pub const INFO_SHADOW: u32 = 0xFF3F_3F3F;
pub const INFO_GAP: f32 = 4.0;
pub const INFO_PITCH: f32 = 10.0;
/// The vanilla font's line: 8 pixels of glyph and one of shadow.
pub const FONT_LINE: f32 = 9.0;

/// Name tags under heads (off by default): white on black at 35%.
pub const NAME_BOX: u32 = 0x5900_0000;
pub const NAME_FONT: f32 = 0.5;

// ---- World map ---------------------------------------------------------------------------------

/// Unexplored ground.
pub const UNEXPLORED: u32 = 0xFF00_0000;
/// Glyph controls: bare icons in 20-GUI cells on the side edges; the gear is 30.
pub const CONTROL_CELL: f32 = 20.0;
pub const GEAR_CELL: f32 = 30.0;
pub const GLYPH: u32 = 0xFFFC_FCFC;
pub const GLYPH_HOVER: u32 = 0xFFE6_E6E6;
pub const GLYPH_SHADOW: u32 = 0xFF3E_3E3E;
pub const GLYPH_OFF: u32 = 0xFF8C_8C8C;
/// Each wheel step multiplies the zoom by this.
pub const ZOOM_STEP: f64 = 1.2;
/// The zoom label's box.
pub const ZOOM_BOX: u32 = 0x8000_0000;
/// Tooltips: a dark translucent box, white text, keys green.
pub const TOOLTIP_BOX: u32 = 0xE810_0010;
pub const TOOLTIP_TEXT: u32 = 0xFFFF_FFFF;
pub const KEY_TEXT: u32 = 0xFF55_FF55;
/// Greyed text (coordinates and distance rows, disabled labels).
pub const GREY_TEXT: u32 = 0xFFA0_A0A0;

/// The right-click menu.
pub const MENU_BORDER: u32 = 0xFFFF_FFFF;
pub const MENU_TITLE: u32 = 0xFF60_6060;
pub const MENU_ROW: u32 = 0xE600_0000;
pub const MENU_HOVER: u32 = 0xE640_4040;
pub const MENU_SEPARATOR: u32 = 0xFF32_3232;
pub const MENU_ROW_HEIGHT: f32 = 10.0;
pub const MENU_WIDTH: f32 = 156.0;
pub const MENU_LINE: f32 = 1.0;

/// World-map waypoints: an opaque pennant with a diagonal drop shadow.
pub const PENNANT: [f32; 2] = [15.0, 15.0];
pub const PENNANT_POINT: f32 = 6.0;
pub const PENNANT_SHADOW: u32 = 0x8000_0000;
pub const PENNANT_SHADOW_OFFSET: f32 = 2.0;
pub const PENNANT_FONT: f32 = 1.5;

// ---- Vanilla screens ---------------------------------------------------------------------------

pub const BUTTON: [f32; 2] = [200.0, 20.0];
pub const SMALL_BUTTON: [f32; 2] = [75.0, 20.0];
pub const FORM_BUTTON: [f32; 2] = [150.0, 20.0];
pub const COLUMN_GAP: f32 = 10.0;
pub const ROW_PITCH: f32 = 24.0;
/// Settings rows on one page.
pub const PAGE_ROWS: usize = 6;
/// What vanilla lays over the blurred world behind an in-game screen
/// (`Screen.extractTransparentBackground`: rgba 16, 16, 16, 160).
pub const SCREEN_DIM: u32 = 0xA010_1010;
/// Vanilla's screen text colours.
pub const SCREEN_TEXT: u32 = 0xFFFF_FFFF;
pub const DISABLED_TEXT: u32 = 0xFFA0_A0A0;
pub const HINT_TEXT: u32 = 0xFF70_7070;
/// The darkened list band behind the waypoint rows, and a selected row.
pub const LIST_BAND: u32 = 0x8000_0000;
pub const LIST_EDGE: u32 = 0xFF00_0000;
pub const SELECTED_ROW: u32 = 0xFF00_0000;
pub const SELECTED_BORDER: u32 = 0xFFFF_FFFF;
pub const WAYPOINT_ROW: [f32; 2] = [220.0, 18.0];
/// The black bar of a vanilla dropdown or a read-only value.
pub const BAR: u32 = 0xFF00_0000;
pub const BAR_EDGE: u32 = 0xFF70_7070;
pub const BAR_HEIGHT: f32 = 11.0;
/// Nine-slice borders of the vanilla widget sprites (their `.mcmeta`), at 200 × 20 texels.
pub const BUTTON_BORDER: f32 = 3.0;
pub const FIELD_BORDER: f32 = 1.0;

/// The kinds an entity on the map can be, in the order they draw: later kinds draw on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Item,
    Passive,
    Hostile,
    Player,
}

/// A world-map control, by what it does; its place comes from `world_controls`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Settings,
    MapSwitching,
    CaveMode,
    Dimension,
    HopTo,
    Camera,
    WaypointMenu,
    Waypoints,
    TrackedPlayers,
    Radar,
    ExportPng,
    Help,
    ZoomOut,
    ZoomIn,
}

/// The mini-menu's toggles, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MiniToggle {
    FullMenu,
    AllSets,
    Backgrounds,
    Deathpoints,
    CloseOnHop,
}
pub const MINI_TOGGLES: [MiniToggle; 5] = [
    MiniToggle::FullMenu,
    MiniToggle::AllSets,
    MiniToggle::Backgrounds,
    MiniToggle::Deathpoints,
    MiniToggle::CloseOnHop,
];

const LEFT: [Control; 5] = [
    Control::MapSwitching,
    Control::CaveMode,
    Control::Dimension,
    Control::HopTo,
    Control::Camera,
];
const RIGHT: [Control; 8] = [
    Control::WaypointMenu,
    Control::Waypoints,
    Control::TrackedPlayers,
    Control::Radar,
    Control::ExportPng,
    Control::Help,
    Control::ZoomOut,
    Control::ZoomIn,
];

/// Each control's glyph, one string per row, `#` lit. Drawn one GUI pixel per cell with
/// `GLYPH_SHADOW` one pixel down and right.
pub fn glyph(control: Control) -> &'static [&'static str] {
    match control {
        Control::Settings => &[
            ".....###.....",
            "..##.###.##..",
            "..#########..",
            "...#######...",
            ".####...####.",
            "####.....####",
            "####.....####",
            "####.....####",
            ".####...####.",
            "...#######...",
            "..#########..",
            "..##.###.##..",
            ".....###.....",
        ],
        Control::MapSwitching => &["#########", ".........", "#########", ".........", "#########"],
        Control::CaveMode => &[
            "...###...",
            "..#####..",
            ".#######.",
            "####.####",
            "###...###",
            "##.....##",
            "##.....##",
        ],
        Control::Dimension => &[
            "#######", "#.....#", "#.###.#", "#.###.#", "#.###.#", "#.....#", "#######",
        ],
        Control::HopTo => &[
            "#...#....",
            ".#...#...",
            "..#...#..",
            "...#...#.",
            "..#...#..",
            ".#...#...",
            "#...#....",
        ],
        Control::Camera => &[
            "....#....",
            "..#####..",
            ".#..#..#.",
            ".#.....#.",
            "####.####",
            ".#.....#.",
            ".#..#..#.",
            "..#####..",
            "....#....",
        ],
        Control::WaypointMenu => &[
            "##.######", "##.######", ".........", "##.######", "##.######", ".........",
            "##.######", "##.######",
        ],
        Control::Waypoints => &[
            "#######", "#######", "#######", "#######", ".#####.", "..###..", "...#...",
        ],
        Control::TrackedPlayers => &[
            "...###...",
            "..#####..",
            "..#####..",
            "...###...",
            ".........",
            ".#######.",
            "#########",
            "#########",
        ],
        Control::Radar => &[
            "#########",
            "#.......#",
            "#.##.##.#",
            "#.##.##.#",
            "#...#...#",
            "#..###..#",
            "#..#.#..#",
            "#.......#",
            "#########",
        ],
        Control::ExportPng => &[
            "....#....",
            "....#....",
            "..#.#.#..",
            "...###...",
            "....#....",
            "#.......#",
            "#########",
        ],
        Control::Help => &[
            "#########",
            "#.......#",
            "#.#.#.#.#",
            "#.......#",
            "#.#####.#",
            "#.......#",
            "#########",
        ],
        Control::ZoomOut => &["#######"],
        Control::ZoomIn => &[
            "...#...", "...#...", "...#...", "#######", "...#...", "...#...", "...#...",
        ],
    }
}

/// A danger ping's sword, drawn on its pennant.
pub const DANGER_GLYPH: &[&str] = &[
    ".......##",
    "......###",
    ".....###.",
    "#...###..",
    ".#.###...",
    "..###....",
    "..##.....",
    ".#..#....",
    "#........",
];

/// The mini-menu toggles' glyphs.
pub fn mini_glyph(toggle: MiniToggle) -> &'static [&'static str] {
    match toggle {
        MiniToggle::FullMenu => glyph(Control::WaypointMenu),
        MiniToggle::AllSets => glyph(Control::MapSwitching),
        MiniToggle::Backgrounds => glyph(Control::Waypoints),
        MiniToggle::Deathpoints => &[
            ".#####.", "#######", "#..#..#", "#######", ".##.##.", ".#####.", ".#.#.#.",
        ],
        MiniToggle::CloseOnHop => &[
            "#.....#", ".#...#.", "..#.#..", "...#...", "..#.#..", ".#...#.", "#.....#",
        ],
    }
}

/// A glyph's size in GUI pixels.
pub fn glyph_size(rows: &[&str]) -> [f32; 2] {
    [rows.iter().map(|row| row.len()).max().unwrap_or(0) as f32, rows.len() as f32]
}

/// The top-left of a glyph centred in its cell (on whole pixels).
pub fn glyph_origin(cell: Rect, rows: &[&str]) -> Point {
    let [w, h] = glyph_size(rows);
    [
        (cell[0] + (cell[2] - w) * 0.5).floor(),
        (cell[1] + (cell[3] - h) * 0.5).floor(),
    ]
}

/// The lit runs of a glyph at `origin`: `(x, y, width)` per run, one pixel tall.
pub fn glyph_runs(rows: &[&str], origin: Point) -> Vec<Rect> {
    let mut runs = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        let bytes = row.as_bytes();
        let mut x = 0;
        while x < bytes.len() {
            if bytes[x] != b'#' {
                x += 1;
                continue;
            }
            let start = x;
            while x < bytes.len() && bytes[x] == b'#' {
                x += 1;
            }
            runs.push([origin[0] + start as f32, origin[1] + y as f32, (x - start) as f32, 1.0]);
        }
    }
    runs
}

// ---- Shared geometry -----------------------------------------------------------------------------

pub fn contains(outer: Rect, inner: Rect) -> bool {
    const E: f32 = 1e-3;
    inner[0] >= outer[0] - E
        && inner[1] >= outer[1] - E
        && inner[0] + inner[2] <= outer[0] + outer[2] + E
        && inner[1] + inner[3] <= outer[1] + outer[3] + E
}

pub fn overlaps(a: Rect, b: Rect) -> bool {
    const E: f32 = 1e-3;
    a[0] + a[2] > b[0] + E && b[0] + b[2] > a[0] + E && a[1] + a[3] > b[1] + E && b[1] + b[3] > a[1] + E
}

pub fn hit(point: Point, rect: Rect) -> bool {
    point[0] >= rect[0] && point[1] >= rect[1] && point[0] < rect[0] + rect[2] && point[1] < rect[1] + rect[3]
}

/// A rectangle reflected across the vertical centre line of a `width`-wide screen.
pub fn mirror(rect: Rect, width: f32) -> Rect {
    [width - rect[0] - rect[2], rect[1], rect[2], rect[3]]
}

fn mirrored(rect: Rect, width: f32, rtl: bool) -> Rect {
    if rtl { mirror(rect, width) } else { rect }
}

fn centre(rect: Rect) -> Point {
    [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5]
}

// ---- Minimap -----------------------------------------------------------------------------------

/// The sixteen frame bands of a square map: four bands on each side, outer first, each with
/// its colour. The map area is `map`; the frame lies outside it. Light bands own the top-left
/// corner and run along the top and the left; dark bands own the bottom-right corner, so the
/// rings tile without overlap.
pub fn frame_bands(map: Rect) -> Vec<(Rect, u32)> {
    let [x, y, w, h] = map;
    let b = FRAME_BAND;
    let mut bands = Vec::with_capacity(16);
    for band in 0..4 {
        let k = (4 - band) as f32 * b;
        let (left, top, right, bottom) = (x - k, y - k, x + w + k, y + h + k);
        bands.push(([left, top, right - left - b, b], FRAME_LIGHT[band]));
        bands.push(([left, top + b, b, bottom - top - b], FRAME_LIGHT[band]));
        bands.push(([left + b, bottom - b, right - left - b, b], FRAME_DARK[band]));
        bands.push(([right - b, top, b, bottom - top - b], FRAME_DARK[band]));
    }
    bands
}

/// The ring of a round map: four bands of `steps` quads each, outer first. A quad facing up
/// and left takes the band's light colour, one facing down and right its dark colour.
pub fn frame_ring(map: Rect, steps: usize) -> Vec<([Point; 4], u32)> {
    let c = centre(map);
    let radius = map[2].min(map[3]) * 0.5;
    let steps = steps.max(8);
    let mut quads = Vec::with_capacity(steps * 4);
    for band in 0..4 {
        let outer = radius + (4 - band) as f32 * FRAME_BAND;
        let inner = outer - FRAME_BAND;
        for step in 0..steps {
            let a0 = std::f32::consts::TAU * step as f32 / steps as f32;
            let a1 = std::f32::consts::TAU * (step + 1) as f32 / steps as f32;
            let mid = (a0 + a1) * 0.5;
            let light = mid.cos() + mid.sin() < 0.0;
            let at = |r: f32, a: f32| [c[0] + a.cos() * r, c[1] + a.sin() * r];
            quads.push((
                [at(outer, a0), at(outer, a1), at(inner, a1), at(inner, a0)],
                if light { FRAME_LIGHT[band] } else { FRAME_DARK[band] },
            ));
        }
    }
    quads
}

/// Where each compass letter's box goes for a map turned by `angle` (radians, the map view's
/// angle; 0 = north up): on the inside of the frame along the line from the centre, so at 45°
/// they sit in the corners. Letters stay upright.
pub fn compass_boxes(map: Rect, angle: f32, round: bool) -> [(char, Rect); 4] {
    let c = centre(map);
    let (s, cos) = angle.sin_cos();
    let reach = map[2].min(map[3]) * 0.5 - COMPASS_INSET - COMPASS_BOX_SIDE * 0.5;
    let place = |letter: char, dx: f32, dz: f32| {
        let sx = dx * cos + dz * s;
        let sy = -dx * s + dz * cos;
        let t = if round { reach } else { reach / sx.abs().max(sy.abs()).max(1e-6) };
        let at = [c[0] + sx * t, c[1] + sy * t];
        let half = COMPASS_BOX_SIDE * 0.5;
        (letter, [at[0] - half, at[1] - half, COMPASS_BOX_SIDE, COMPASS_BOX_SIDE])
    };
    [
        place('N', 0.0, -1.0),
        place('E', 1.0, 0.0),
        place('S', 0.0, 1.0),
        place('W', -1.0, 0.0),
    ]
}

/// How bright a head draws for an entity `dy` blocks above (positive) or below the player:
/// full above and level, darker below (−10 → 58%, −20 → 25%), none beyond `HEIGHT_LIMIT`.
pub fn depth_shade(dy: f32) -> Option<f32> {
    if !dy.is_finite() || dy.abs() > HEIGHT_LIMIT {
        return None;
    }
    const FULL: f32 = -2.0;
    Some(if dy >= FULL {
        1.0
    } else if dy >= -10.0 {
        1.0 - (FULL - dy) / (FULL + 10.0) * 0.42
    } else {
        0.58 - (-10.0 - dy) / 10.0 * 0.33
    })
}

/// Whether a point lies in the map area (inside the circle for a round map).
pub fn on_map(map: Rect, round: bool, point: Point) -> bool {
    if !hit(point, map) {
        return false;
    }
    let c = centre(map);
    !round || (point[0] - c[0]).hypot(point[1] - c[1]) <= map[2].min(map[3]) * 0.5
}

/// A waypoint's square on the minimap: at its point when inside the map, else clamped onto
/// the frame on the line from the centre towards it, centred on the frame bands.
pub fn waypoint_square(map: Rect, round: bool, point: Point) -> Rect {
    let half = WAYPOINT_SIDE * 0.5;
    let square = |at: Point| [at[0] - half, at[1] - half, WAYPOINT_SIDE, WAYPOINT_SIDE];
    if on_map(map, round, point) {
        return square(point);
    }
    let c = centre(map);
    let (dx, dy) = (point[0] - c[0], point[1] - c[1]);
    let reach = map[2].min(map[3]) * 0.5 + FRAME * 0.5;
    let t = if round {
        reach / dx.hypot(dy).max(1e-6)
    } else {
        reach / dx.abs().max(dy.abs()).max(1e-6)
    };
    square([c[0] + dx * t, c[1] + dy * t])
}

/// The box a waypoint square may reach: the map with its frame and half a square around.
pub fn waypoint_bounds(map: Rect) -> Rect {
    let pad = FRAME * 0.5 + WAYPOINT_SIDE * 0.5;
    [map[0] - pad, map[1] - pad, map[2] + 2.0 * pad, map[3] + 2.0 * pad]
}

/// The info lines' rectangles, centred under the frame, one per entry of `widths`.
pub fn info_lines(map: Rect, widths: &[f32]) -> Vec<Rect> {
    let cx = map[0] + map[2] * 0.5;
    let top = map[1] + map[3] + FRAME + INFO_GAP;
    widths
        .iter()
        .enumerate()
        .map(|(i, w)| [(cx - w * 0.5).round(), top + i as f32 * INFO_PITCH, *w, FONT_LINE])
        .collect()
}

/// The height the minimap occupies with `lines` info lines under it, frame included.
pub fn minimap_height(side: f32, lines: usize) -> f32 {
    let frame = side + 2.0 * FRAME;
    if lines == 0 { frame } else { frame + INFO_GAP + (lines - 1) as f32 * INFO_PITCH + FONT_LINE }
}

/// The map area for a minimap of `side` whose outer frame sits `MAP_MARGIN` from the chosen
/// corner of a `screen`. `corner`: 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right;
/// `below` is the height the info lines add under the frame.
pub fn minimap_rect(screen: [f32; 2], side: f32, corner: usize, below: f32) -> Rect {
    let outer = side + 2.0 * FRAME;
    let x = if corner % 2 == 0 { MAP_MARGIN } else { screen[0] - MAP_MARGIN - outer };
    let y = if corner < 2 { MAP_MARGIN } else { screen[1] - MAP_MARGIN - outer - below };
    [x + FRAME, y + FRAME, side, side]
}

/// The four corners of the arrow (tip, left back, notch, right back) at `at`, pointing along
/// the unit `forward`.
pub fn arrow(at: Point, forward: Point, scale: f32) -> [Point; 4] {
    let right = [-forward[1], forward[0]];
    let p = |along: f32, across: f32| {
        [
            at[0] + scale * (forward[0] * along + right[0] * across),
            at[1] + scale * (forward[1] * along + right[1] * across),
        ]
    };
    [p(3.4, 0.0), p(-2.0, -4.0), p(-0.6, 0.0), p(-2.0, 4.0)]
}

/// The crosshair's arms: a horizontal and a vertical bar, and the centre pixel.
pub fn crosshair(at: Point) -> [Rect; 3] {
    let half = CROSSHAIR_SIDE * 0.5;
    [
        [at[0] - half, at[1] - 0.5, CROSSHAIR_SIDE, 1.0],
        [at[0] - 0.5, at[1] - half, 1.0, CROSSHAIR_SIDE],
        [at[0] - 0.5, at[1] - 0.5, 1.0, 1.0],
    ]
}

/// A head's rectangle: `texels` of the face, one texel per GUI pixel times `scale`, centred on
/// the entity.
pub fn head_rect(at: Point, texels: [f32; 2], scale: f32) -> Rect {
    let [w, h] = [texels[0] * scale, texels[1] * scale];
    [at[0] - w * 0.5, at[1] - h * 0.5, w, h]
}

/// How far a head may move from its entity to clear a crowd: half a head, so it still
/// touches its spot.
pub const CROWD_REACH: f32 = 4.0;

/// Heads that overlap pushed apart so each stays readable, every one at most `reach` from its
/// anchor. `sizes` are the heads' sides. The same input gives the same result every frame.
pub fn declutter(anchors: &[Point], sizes: &[f32], reach: f32) -> Vec<Point> {
    let mut at = anchors.to_vec();
    for _ in 0..16 {
        let mut moved = false;
        for i in 0..at.len() {
            for j in i + 1..at.len() {
                let need = (sizes[i] + sizes[j]) * 0.5;
                let (dx, dy) = (at[j][0] - at[i][0], at[j][1] - at[i][1]);
                let (ox, oy) = (need - dx.abs(), need - dy.abs());
                if ox <= 0.0 || oy <= 0.0 {
                    continue;
                }
                // Push along the axis that needs the least, both heads half each; heads on one
                // spot part along a direction their order picks.
                let (sx, sy) = if dx == 0.0 && dy == 0.0 {
                    let a = (i * 7 + j * 13) as f32;
                    (a.cos(), a.sin())
                } else {
                    (dx.signum(), dy.signum())
                };
                if ox < oy {
                    at[i][0] -= sx * ox * 0.5;
                    at[j][0] += sx * ox * 0.5;
                } else {
                    at[i][1] -= sy * oy * 0.5;
                    at[j][1] += sy * oy * 0.5;
                }
                moved = true;
            }
        }
        for (p, a) in at.iter_mut().zip(anchors) {
            let (dx, dy) = (p[0] - a[0], p[1] - a[1]);
            let d = dx.hypot(dy);
            if d > reach {
                *p = [a[0] + dx / d * reach, a[1] + dy / d * reach];
            }
        }
        if !moved {
            break;
        }
    }
    at
}

/// The area two squares of these sides at these centres share.
pub fn shared_area(a: Point, sa: f32, b: Point, sb: f32) -> f32 {
    let need = (sa + sb) * 0.5;
    let ox = (need - (a[0] - b[0]).abs()).clamp(0.0, sa.min(sb));
    let oy = (need - (a[1] - b[1]).abs()).clamp(0.0, sa.min(sb));
    ox * oy
}

/// The four offsets of a silhouette outline `px` wide.
pub fn outline_offsets(px: f32) -> [Point; 4] {
    [[-px, 0.0], [px, 0.0], [0.0, -px], [0.0, px]]
}

/// A name tag under a head: the box and the text's top-left, for a text `width` at the
/// half-size font.
pub fn name_tag(head: Rect, width: f32) -> (Rect, Point) {
    let w = width * NAME_FONT + 2.0;
    let x = head[0] + head[2] * 0.5 - w * 0.5;
    let y = head[1] + head[3] + 0.5;
    ([x, y, w, 5.0], [x + 1.0, y + 0.5])
}

// ---- World map ---------------------------------------------------------------------------------

/// Every world-map control's cell on a `[w, h]` screen; the columns swap sides right to left.
pub fn world_controls(screen: [f32; 2], rtl: bool) -> Vec<(Control, Rect)> {
    let [w, h] = screen;
    let mut cells = vec![(Control::Settings, mirrored([0.0, 0.0, GEAR_CELL, GEAR_CELL], w, rtl))];
    for (i, control) in LEFT.into_iter().enumerate() {
        let cell = [0.0, h - CONTROL_CELL * (i + 1) as f32, CONTROL_CELL, CONTROL_CELL];
        cells.push((control, mirrored(cell, w, rtl)));
    }
    for (i, control) in RIGHT.into_iter().enumerate() {
        let cell = [w - CONTROL_CELL, h - CONTROL_CELL * (i + 1) as f32, CONTROL_CELL, CONTROL_CELL];
        cells.push((control, mirrored(cell, w, rtl)));
    }
    cells
}

/// The cursor readout's lines, centred at the top: one rect per text width.
pub fn cursor_lines(screen: [f32; 2], widths: &[f32]) -> Vec<Rect> {
    widths
        .iter()
        .enumerate()
        .map(|(i, w)| [((screen[0] - w) * 0.5).round(), 2.0 + i as f32 * INFO_PITCH, *w, FONT_LINE])
        .collect()
}

/// The zoom label at the bottom centre: its box and its text's top-left.
pub fn zoom_label(screen: [f32; 2], width: f32) -> (Rect, Point) {
    let w = width + 4.0;
    let x = ((screen[0] - w) * 0.5).round();
    let y = screen[1] - 12.0;
    ([x, y, w, 11.0], [x + 2.0, y + 2.0])
}

/// The right-click menu opened at `at`, kept whole on the screen: the outer box (white
/// border included), the title row and one rect per row. Rows are separated by
/// `MENU_LINE`-thick lines, which are the gaps between consecutive rects.
pub fn context_menu(at: [f32; 2], rows: usize, screen: [f32; 2]) -> (Rect, Rect, Vec<Rect>) {
    let height = MENU_LINE * 2.0 + MENU_ROW_HEIGHT + rows as f32 * (MENU_LINE + MENU_ROW_HEIGHT);
    let width = MENU_WIDTH.min(screen[0]);
    let x = at[0].min(screen[0] - width).max(0.0);
    let y = at[1].min(screen[1] - height).max(0.0);
    let outer = [x, y, width, height];
    let inner_x = x + MENU_LINE;
    let inner_w = width - 2.0 * MENU_LINE;
    let title = [inner_x, y + MENU_LINE, inner_w, MENU_ROW_HEIGHT];
    let rows = (0..rows)
        .map(|i| {
            [
                inner_x,
                title[1] + MENU_ROW_HEIGHT + MENU_LINE + i as f32 * (MENU_ROW_HEIGHT + MENU_LINE),
                inner_w,
                MENU_ROW_HEIGHT,
            ]
        })
        .collect();
    (outer, title, rows)
}

/// A pennant for a world-map waypoint whose point is at `at`: the body and the three points
/// of its tip (left, right, point).
pub fn pennant(at: Point, scale: f32) -> (Rect, [Point; 3]) {
    let [w, h] = [PENNANT[0] * scale, PENNANT[1] * scale];
    let point = PENNANT_POINT * scale;
    let body = [at[0] - w * 0.5, at[1] - point - h, w, h];
    (body, [[body[0], at[1] - point], [body[0] + w, at[1] - point], at])
}

/// A tooltip box for text `size` (`[w, h]`) describing the control `hovered`, placed as
/// vanilla places a widget's tooltip: under it, else above it, else beside it; starting at
/// the pointer, kept on the screen and never covering the control.
pub fn tooltip_box(pointer: Point, size: [f32; 2], screen: [f32; 2], hovered: Rect) -> Rect {
    let w = size[0] + 6.0;
    let h = size[1] + 4.0;
    let x = (pointer[0] + 4.0).min(screen[0] - w).max(0.0);
    let below = hovered[1] + hovered[3] + 2.0;
    if below + h <= screen[1] {
        return [x, below, w, h];
    }
    let above = hovered[1] - 2.0 - h;
    if above >= 0.0 {
        return [x, above, w, h];
    }
    let y = (pointer[1] - h * 0.5).min(screen[1] - h).max(0.0);
    let right = hovered[0] + hovered[2] + 2.0;
    let x = if right + w <= screen[0] { right } else { (hovered[0] - 2.0 - w).max(0.0) };
    [x, y, w, h]
}

/// The mini-menu beside the right control column: `rows` list rows above a set bar, a filter
/// box and the five toggles, all pressed against the column.
pub struct MiniMenu {
    pub rows: Vec<Rect>,
    pub set: Rect,
    pub filter: Rect,
    pub toggles: [Rect; 5],
}

pub fn mini_menu(screen: [f32; 2], rows: usize, rtl: bool) -> MiniMenu {
    let [w, h] = screen;
    let right = w - CONTROL_CELL - 2.0;
    let width = 150.0;
    let left = right - width;
    let toggles = std::array::from_fn(|i| {
        mirrored([right - (5 - i) as f32 * CONTROL_CELL, h - CONTROL_CELL, CONTROL_CELL, CONTROL_CELL], w, rtl)
    });
    let filter = mirrored([left, h - CONTROL_CELL - 22.0, width, 20.0], w, rtl);
    let set = mirrored([left, filter[1] - 2.0 - BAR_HEIGHT, width, BAR_HEIGHT], w, rtl);
    let fit = (((set[1] - 32.0) / INFO_PITCH).floor().max(0.0)) as usize;
    let rows = (0..rows.min(fit))
        .map(|i| {
            let y = set[1] - 2.0 - (i + 1) as f32 * INFO_PITCH;
            mirrored([left, y, width, INFO_PITCH], w, rtl)
        })
        .collect();
    MiniMenu { rows, set, filter, toggles }
}

/// How many list rows the mini-menu holds on a screen this tall.
pub fn mini_menu_rows(height: f32) -> usize {
    let set_top = height - CONTROL_CELL - 22.0 - 2.0 - BAR_HEIGHT;
    (((set_top - 32.0) / INFO_PITCH).floor().max(0.0)) as usize
}

// ---- Vanilla screens ---------------------------------------------------------------------------

/// Where vanilla options screens put things: the title line and the first button row.
pub fn screen_title_y(height: f32) -> f32 {
    (height / 8.0).floor().max(4.0)
}
fn grid_top(height: f32) -> f32 {
    (height / 6.0).floor() + 15.0
}
/// The Back / Done row of a vanilla options screen (`height / 6 + 168`), kept on screen.
fn back_y(height: f32) -> f32 {
    ((height / 6.0).floor() + 168.0).min(height - BUTTON[1] - 4.0)
}

/// A button column's width: 200 when the screen has room for two columns, narrower otherwise.
pub fn column(width: f32) -> f32 {
    BUTTON[0].min(((width - COLUMN_GAP) * 0.5 - 6.0).floor()).max(40.0)
}

/// The width of the two-column block.
fn block(width: f32) -> f32 {
    column(width) * 2.0 + COLUMN_GAP
}

/// A small button's width: 75 unless five of them must share a narrow block.
fn small(width: f32) -> f32 {
    SMALL_BUTTON[0].min(((block(width) - 4.0 * 4.0) / 5.0).floor())
}

/// A centred wide button (Back, Done) that leaves room for the pager at both ends.
fn wide_button(width: f32, y: f32) -> Rect {
    let cx = (width * 0.5).floor();
    let w = BUTTON[0].min(block(width) - 2.0 * (SMALL_BUTTON[0] + 4.0));
    [cx - (w * 0.5).floor(), y, w, BUTTON[1]]
}

/// A vanilla settings page: `count` buttons (at most `PAGE_ROWS` rows) in two columns centred
/// on a `[w, h]` screen, the Back button under them; mirrored right to left.
pub fn settings_grid(screen: [f32; 2], count: usize, rtl: bool) -> (Vec<Rect>, Rect) {
    let [w, h] = screen;
    let cx = (w * 0.5).floor();
    let top = grid_top(h);
    let bw = column(w);
    let buttons = (0..count.min(PAGE_ROWS * 2))
        .map(|i| {
            let x = if i % 2 == 0 { cx - bw - COLUMN_GAP * 0.5 } else { cx + COLUMN_GAP * 0.5 };
            mirrored([x, top + (i / 2) as f32 * ROW_PITCH, bw, BUTTON[1]], w, rtl)
        })
        .collect();
    (buttons, wide_button(w, back_y(h)))
}

/// The `<< Prev` and `Next >>` buttons on the row after the grid, at its two ends (swapped
/// right to left, so "previous" stays at the reading start).
pub fn pager(screen: [f32; 2], rtl: bool) -> [Rect; 2] {
    let [w, h] = screen;
    let cx = (w * 0.5).floor();
    let y = grid_top(h) + PAGE_ROWS as f32 * ROW_PITCH;
    let span = column(w) + COLUMN_GAP * 0.5;
    let prev = [cx - span, y, SMALL_BUTTON[0], SMALL_BUTTON[1]];
    let next = [cx + span - SMALL_BUTTON[0], y, SMALL_BUTTON[0], SMALL_BUTTON[1]];
    [mirrored(prev, w, rtl), mirrored(next, w, rtl)]
}

/// The waypoint list screen.
pub struct WaypointList {
    pub labels: [Point; 2],
    pub bars: [Rect; 2],
    pub options: Rect,
    pub set: Rect,
    pub new_set: Rect,
    pub filter: Rect,
    pub band: Rect,
    pub rows: Vec<Rect>,
    pub actions: [Rect; 5],
    pub done: Rect,
}

/// How many waypoint rows fit on a screen this tall.
pub fn waypoint_rows(height: f32) -> usize {
    let band_top = 57.0;
    let band_bottom = height - 61.0;
    (((band_bottom - band_top - 2.0) / WAYPOINT_ROW[1]).floor().max(1.0)) as usize
}

pub fn waypoint_list(screen: [f32; 2], rows: usize, rtl: bool) -> WaypointList {
    let [w, h] = screen;
    let cx = (w * 0.5).floor();
    let wide = block(w);
    let bw = column(w);
    let sw = small(w);
    let left = cx - wide * 0.5;
    let m = |r: Rect| mirrored(r, w, rtl);
    let bars = [
        m([left, 17.0, bw, BAR_HEIGHT]),
        m([left + bw + COLUMN_GAP, 17.0, bw, BAR_HEIGHT]),
    ];
    let row_w = WAYPOINT_ROW[0].min(wide - 2.0 * (sw + 4.0));
    let band = [0.0, 57.0, w, (h - 61.0 - 57.0).max(WAYPOINT_ROW[1] + 2.0)];
    let rows = (0..rows.min(waypoint_rows(h)))
        .map(|i| [cx - (row_w * 0.5).floor(), 59.0 + i as f32 * WAYPOINT_ROW[1], row_w, WAYPOINT_ROW[1]])
        .collect();
    let pitch = ((wide - sw) / 4.0).floor();
    let actions = std::array::from_fn(|i| m([left + i as f32 * pitch, h - 53.0, sw, SMALL_BUTTON[1]]));
    WaypointList {
        labels: [
            [bars[0][0] + bw * 0.5, 7.0],
            [bars[1][0] + bw * 0.5, 7.0],
        ],
        bars,
        options: m([left, 32.0, sw, SMALL_BUTTON[1]]),
        set: [cx - (row_w * 0.5).floor(), 36.0, row_w, BAR_HEIGHT],
        new_set: m([left + wide - sw, 32.0, sw, SMALL_BUTTON[1]]),
        filter: m([left, 60.0, sw, SMALL_BUTTON[1]]),
        band,
        rows,
        actions,
        done: wide_button(w, h - 29.0),
    }
}

/// The initial square at the reading start of a waypoint row, and the row's name centre.
pub fn waypoint_row_parts(row: Rect, rtl: bool) -> (Rect, Point) {
    let side = WAYPOINT_SIDE;
    let x = if rtl { row[0] + row[2] - 6.0 - side } else { row[0] + 6.0 };
    ([x, row[1] + (row[3] - side) * 0.5, side, side], [row[0] + row[2] * 0.5, row[1] + (row[3] - 8.0) * 0.5])
}

/// The create / edit waypoint screen.
pub struct WaypointForm {
    pub title: Point,
    pub world: Rect,
    pub dimension: Rect,
    pub set: Rect,
    pub color: Rect,
    pub name: Rect,
    /// x, y, z and yaw.
    pub coords: [Rect; 4],
    pub world_3d: Rect,
    pub initials: Rect,
    pub enabled: Rect,
    pub confirm: Rect,
    pub cancel: Rect,
}

pub fn waypoint_form(screen: [f32; 2], rtl: bool) -> WaypointForm {
    let [w, h] = screen;
    let cx = (w * 0.5).floor();
    let m = |r: Rect| mirrored(r, w, rtl);
    let wide = block(w);
    let bw = column(w);
    let left = cx - wide * 0.5;
    let field = 50.0;
    let gap = 6.0;
    let coords_left = cx - (4.0 * field + 3.0 * gap) * 0.5;
    let toggle = 79.0;
    let row = toggle * 2.0 + field + 2.0 * gap;
    let row_left = cx - row * 0.5;
    let buttons = back_y(h);
    let fw = FORM_BUTTON[0].min(bw);
    WaypointForm {
        title: [cx, 23.0],
        world: m([left, 38.0, bw, BAR_HEIGHT]),
        dimension: m([left + bw + COLUMN_GAP, 38.0, bw, BAR_HEIGHT]),
        set: [cx - (bw * 0.5).floor(), 60.0, bw, BAR_HEIGHT],
        color: [cx - 60.0, 82.0, 120.0, BAR_HEIGHT],
        name: [cx - (bw * 0.5).floor(), 104.0, bw, BUTTON[1]],
        coords: std::array::from_fn(|i| m([coords_left + i as f32 * (field + gap), 134.0, field, 20.0])),
        world_3d: m([row_left, 164.0, toggle, 20.0]),
        initials: m([row_left + toggle + gap, 164.0, field, 20.0]),
        enabled: m([row_left + toggle + field + 2.0 * gap, 164.0, toggle, 20.0]),
        confirm: m([cx - fw - 5.0, buttons, fw, FORM_BUTTON[1]]),
        cancel: m([cx + 5.0, buttons, fw, FORM_BUTTON[1]]),
    }
}

/// The nine pieces of a nine-slice sprite drawn into `dst`: `(src, dst)` pairs, `src` in the
/// sprite's texels. `size` is the sprite's texel size, `border` its border in texels and
/// `texel` the GUI size of one texel (1 at the vanilla pack's resolution).
pub fn nine_slice(dst: Rect, size: [f32; 2], border: f32, texel: f32) -> Vec<(Rect, Rect)> {
    let [x, y, w, h] = dst;
    let b = (border * texel).min(w * 0.5).min(h * 0.5);
    let src_b = border;
    let xs = [(x, b, 0.0, src_b), (x + b, w - 2.0 * b, src_b, size[0] - 2.0 * src_b), (x + w - b, b, size[0] - src_b, src_b)];
    let ys = [(y, b, 0.0, src_b), (y + b, h - 2.0 * b, src_b, size[1] - 2.0 * src_b), (y + h - b, b, size[1] - src_b, src_b)];
    let mut pieces = Vec::with_capacity(9);
    for (py, ph, sy, sh) in ys {
        for (px, pw, sx, sw) in xs {
            if pw > 0.0 && ph > 0.0 {
                // A piece narrower than its texels shows their start, as vanilla's tiling does.
                pieces.push(([sx, sy, sw.min(pw / texel), sh.min(ph / texel)], [px, py, pw, ph]));
            }
        }
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREENS: [[f32; 2]; 5] = [[320.0, 240.0], [427.0, 240.0], [640.0, 360.0], [1001.0, 564.0], [960.0, 540.0]];

    fn no_overlaps(rects: &[Rect]) {
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!overlaps(*a, *b), "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn frame_bands_tile_the_frame_without_overlap() {
        let map = [18.0, 18.0, 135.0, 135.0];
        let bands = frame_bands(map);
        assert_eq!(bands.len(), 16);
        let rects: Vec<Rect> = bands.iter().map(|(r, _)| *r).collect();
        no_overlaps(&rects);
        let area: f32 = rects.iter().map(|r| r[2] * r[3]).sum();
        let outer = (135.0 + 8.0) * (135.0 + 8.0) - 135.0 * 135.0;
        assert!((area - outer).abs() < 0.01);
        let frame = [map[0] - FRAME, map[1] - FRAME, map[2] + 2.0 * FRAME, map[3] + 2.0 * FRAME];
        for r in &rects {
            assert!(contains(frame, *r));
            assert!(!overlaps(*r, map));
        }
        // Outer band black, light top-left and dark bottom-right.
        assert_eq!(bands[0].1, FRAME_LIGHT[0]);
        assert_eq!(bands[4].1, FRAME_LIGHT[1]);
        assert_eq!(bands[6].1, FRAME_DARK[1]);
    }

    #[test]
    fn frame_ring_stays_in_its_band() {
        let map = [10.0, 10.0, 100.0, 100.0];
        let quads = frame_ring(map, 64);
        assert_eq!(quads.len(), 256);
        for (points, color) in &quads {
            for p in points {
                let r = (p[0] - 60.0).hypot(p[1] - 60.0);
                assert!(r >= 50.0 - 1e-3 && r <= 54.0 + 1e-3);
            }
            assert!(FRAME_LIGHT.contains(color) || FRAME_DARK.contains(color));
        }
    }

    #[test]
    fn compass_rides_inside_the_frame_and_reaches_the_corners() {
        let map = [18.0, 18.0, 135.0, 135.0];
        for round in [false, true] {
            for step in 0..16 {
                let angle = step as f32 * std::f32::consts::TAU / 16.0;
                let boxes = compass_boxes(map, angle, round);
                let rects: Vec<Rect> = boxes.iter().map(|(_, r)| *r).collect();
                no_overlaps(&rects);
                for r in rects {
                    assert!(contains(map, r), "{r:?} at {angle}");
                }
            }
        }
        let north = compass_boxes(map, 0.0, false)[0].1;
        assert_eq!(north[1], map[1] + COMPASS_INSET);
        assert!((north[0] + north[2] * 0.5 - 85.5).abs() < 1e-3);
        let corner = compass_boxes(map, std::f32::consts::FRAC_PI_4, false)[0].1;
        assert!((corner[1] - (map[1] + COMPASS_INSET)).abs() < 1e-3);
        assert!((corner[0] - (map[0] + COMPASS_INSET)).abs() < 1e-3 || (corner[0] + corner[2] - (map[0] + map[2] - COMPASS_INSET)).abs() < 1e-3);
    }

    #[test]
    fn depth_shade_matches_the_record() {
        assert_eq!(depth_shade(0.0), Some(1.0));
        assert_eq!(depth_shade(15.0), Some(1.0));
        assert!((depth_shade(-10.0).unwrap() - 0.58).abs() < 1e-4);
        assert!((depth_shade(-20.0).unwrap() - 0.25).abs() < 1e-4);
        assert!(depth_shade(-3.0).unwrap() > 0.9);
        assert_eq!(depth_shade(-20.5), None);
        assert_eq!(depth_shade(25.0), None);
        let mut last = 1.0;
        for dy in (0..=20).map(|d| -(d as f32)) {
            let shade = depth_shade(dy).unwrap();
            assert!(shade <= last + 1e-6);
            last = shade;
        }
    }

    #[test]
    fn waypoints_inside_stay_and_outside_ride_the_frame() {
        let map = [18.0, 18.0, 135.0, 135.0];
        for round in [false, true] {
            let inside = waypoint_square(map, round, [85.0, 85.0]);
            assert_eq!(inside, [80.5, 80.5, WAYPOINT_SIDE, WAYPOINT_SIDE]);
            for step in 0..32 {
                let a = step as f32 * std::f32::consts::TAU / 32.0;
                let far = [85.5 + a.cos() * 5000.0, 85.5 + a.sin() * 5000.0];
                let square = waypoint_square(map, round, far);
                assert!(contains(waypoint_bounds(map), square), "{square:?}");
                let c = centre(square);
                let d = (c[0] - 85.5).hypot(c[1] - 85.5);
                assert!(d >= 67.5 + FRAME * 0.5 - 1e-3);
            }
        }
        // Due east, square map: centred on the frame's middle band line.
        let east = waypoint_square(map, false, [5000.0, 85.5]);
        assert!((centre(east)[0] - (map[0] + map[2] + FRAME * 0.5)).abs() < 1e-3);
    }

    #[test]
    fn info_lines_centre_under_the_frame() {
        let map = [9.0, 9.0, 135.0, 135.0];
        for widths in [vec![], vec![40.0], vec![0.0, 200.0, 12.5, 90.0, 70.0, 300.0]] {
            let lines = info_lines(map, &widths);
            assert_eq!(lines.len(), widths.len());
            no_overlaps(&lines);
            for line in &lines {
                assert!(line[1] >= map[1] + map[3] + FRAME + INFO_GAP);
                assert!((line[0] + line[2] * 0.5 - 76.5).abs() <= 0.5);
            }
            let bottom = lines.last().map_or(map[1] + map[3] + FRAME, |l| l[1] + l[3]);
            assert!((map[1] - FRAME + minimap_height(135.0, widths.len()) - bottom).abs() < 1e-3);
        }
    }

    #[test]
    fn minimap_sits_in_every_corner() {
        for screen in SCREENS {
            for corner in 0..4 {
                let below = minimap_height(135.0, 3) - 143.0;
                let map = minimap_rect(screen, 135.0, corner, below);
                let outer = [map[0] - FRAME, map[1] - FRAME, map[2] + 2.0 * FRAME, minimap_height(135.0, 3)];
                assert!(contains([0.0, 0.0, screen[0], screen[1]], outer), "{corner} {screen:?}");
                let gap_x = if corner % 2 == 0 { outer[0] } else { screen[0] - outer[0] - outer[2] };
                assert_eq!(gap_x, MAP_MARGIN);
            }
        }
        assert_eq!(minimap_rect([1001.0, 564.0], 135.0, 0, 0.0), [9.0, 9.0, 135.0, 135.0]);
    }

    #[test]
    fn arrow_points_forward_with_a_notch() {
        let [tip, left, notch, right] = arrow([0.0, 0.0], [1.0, 0.0], ARROW_SCALE);
        assert!(tip[0] > notch[0] && notch[0] > left[0]);
        assert_eq!(left[0], right[0]);
        assert!((right[1] - left[1] - 12.0).abs() < 1e-3);
        assert!((tip[0] - left[0] - 8.1).abs() < 1e-3);
    }

    fn crowd_overlap(at: &[Point], sizes: &[f32]) -> f32 {
        let mut total = 0.0;
        for i in 0..at.len() {
            for j in i + 1..at.len() {
                total += shared_area(at[i], sizes[i], at[j], sizes[j]);
            }
        }
        total
    }

    #[test]
    fn crowds_part_but_stay_anchored() {
        // Mobs 1 to 2 GUI apart (6 blocks at Xaero's zoom 1), many on one spot, and a lone one.
        let mut anchors = vec![[100.0, 100.0]; 4];
        for i in 0..20 {
            anchors.push([100.0 + (i % 5) as f32 * 1.5, 100.0 + (i / 5) as f32 * 2.0]);
        }
        anchors.push([160.0, 160.0]);
        let sizes = vec![8.0; anchors.len()];
        let parted = declutter(&anchors, &sizes, CROWD_REACH);
        let before = crowd_overlap(&anchors, &sizes);
        let after = crowd_overlap(&parted, &sizes);
        eprintln!("crowd overlap {before:.1} -> {after:.1} GUI px²");
        assert!(after < before * 0.5, "overlap {before} -> {after}");
        for (p, a) in parted.iter().zip(&anchors) {
            assert!((p[0] - a[0]).hypot(p[1] - a[1]) <= CROWD_REACH + 1e-3);
        }
        assert_eq!(parted.last(), anchors.last(), "a lone head does not move");
        assert_eq!(declutter(&anchors, &sizes, CROWD_REACH), parted, "the same every frame");
        // Two heads that touch only a little part fully.
        let two = declutter(&[[0.0, 0.0], [6.0, 0.0]], &[8.0, 8.0], CROWD_REACH);
        assert!(crowd_overlap(&two, &[8.0, 8.0]) < 1e-3);
    }

    #[test]
    fn heads_and_tags_stay_attached() {
        let head = head_rect([50.0, 50.0], [8.0, 8.0], 1.0);
        assert_eq!(head, [46.0, 46.0, 8.0, 8.0]);
        for width in [0.0, 20.0, 400.0] {
            let (tag, text) = name_tag(head, width);
            assert!(tag[1] >= head[1] + head[3]);
            assert!((tag[0] + tag[2] * 0.5 - 50.0).abs() < 1e-3);
            assert!(contains(tag, [text[0], text[1], width * NAME_FONT, 4.0]));
        }
    }

    #[test]
    fn world_controls_fit_and_swap_sides() {
        for screen in SCREENS {
            for rtl in [false, true] {
                let cells = world_controls(screen, rtl);
                assert_eq!(cells.len(), 14);
                let rects: Vec<Rect> = cells.iter().map(|(_, r)| *r).collect();
                no_overlaps(&rects);
                for (control, r) in &cells {
                    assert!(contains([0.0, 0.0, screen[0], screen[1]], *r));
                    let rows = glyph(*control);
                    let at = glyph_origin(*r, rows);
                    let [w, h] = glyph_size(rows);
                    assert!(contains(*r, [at[0], at[1], w + 1.0, h + 1.0]), "{control:?} glyph + shadow");
                }
                let gear = cells[0].1;
                assert_eq!(gear[0] == 0.0, !rtl);
                let zoom_in = cells.iter().find(|(c, _)| *c == Control::ZoomIn).unwrap().1;
                assert_eq!(zoom_in[0] == 0.0, rtl);
            }
        }
    }

    #[test]
    fn every_glyph_is_rectangular_and_lit() {
        for control in LEFT.into_iter().chain(RIGHT).chain([Control::Settings]) {
            let rows = glyph(control);
            let width = rows[0].len();
            assert!(rows.iter().all(|r| r.len() == width), "{control:?}");
            assert!(!glyph_runs(rows, [0.0, 0.0]).is_empty());
        }
        assert!(DANGER_GLYPH.iter().all(|r| r.len() == DANGER_GLYPH[0].len()));
        let [w, h] = glyph_size(DANGER_GLYPH);
        assert!(w + 1.0 <= PENNANT[0] && h + 1.0 <= PENNANT[1], "the sword fits its pennant with its shadow");
        for toggle in MINI_TOGGLES {
            let rows = mini_glyph(toggle);
            assert!(rows.iter().all(|r| r.len() == rows[0].len()));
        }
    }

    #[test]
    fn context_menu_stays_whole_on_screen() {
        for screen in SCREENS {
            for at in [[0.0, 0.0], [screen[0] - 1.0, screen[1] - 1.0], [screen[0] * 0.5, screen[1] * 0.5]] {
                for rows in [0, 1, 10, 14] {
                    let (outer, title, rects) = context_menu(at, rows, screen);
                    assert_eq!(rects.len(), rows);
                    if outer[3] <= screen[1] {
                        assert!(contains([0.0, 0.0, screen[0], screen[1]], outer));
                    }
                    let mut all = vec![title];
                    all.extend(&rects);
                    no_overlaps(&all);
                    for r in &all {
                        assert!(contains([outer[0] + MENU_LINE, outer[1] + MENU_LINE, outer[2] - 2.0 * MENU_LINE, outer[3] - 2.0 * MENU_LINE], *r));
                    }
                    for pair in all.windows(2) {
                        assert!((pair[1][1] - pair[0][1] - pair[0][3] - MENU_LINE).abs() < 1e-3);
                    }
                }
            }
        }
        assert_eq!(context_menu([0.0, 0.0], 10, [1001.0, 564.0]).0[2], 156.0);
    }

    #[test]
    fn pennant_hangs_above_its_point() {
        let (body, tip) = pennant([100.0, 100.0], 1.0);
        assert_eq!(tip[2], [100.0, 100.0]);
        assert!(body[1] + body[3] <= tip[0][1] + 1e-3);
        assert!((body[0] + body[2] * 0.5 - 100.0).abs() < 1e-3);
    }

    #[test]
    fn tooltips_stay_on_screen_and_off_the_hovered_control() {
        for screen in SCREENS {
            let mut hovered = vec![[0.0, 0.0, 30.0, 30.0], [screen[0] - 20.0, screen[1] - 20.0, 20.0, 20.0]];
            hovered.extend(settings_grid(screen, 12, false).0);
            hovered.extend(world_controls(screen, true).into_iter().map(|(_, r)| r));
            hovered.extend(waypoint_list(screen, 30, false).rows);
            for control in hovered {
                for p in [[control[0], control[1]], [control[0] + control[2] - 1.0, control[1] + control[3] - 1.0]] {
                    for size in [[0.0, 9.0], [40.0, 9.0], [160.0, 59.0]] {
                        let r = tooltip_box(p, size, screen, control);
                        assert!(contains([0.0, 0.0, screen[0], screen[1]], r), "{r:?} on {screen:?}");
                        assert!(!overlaps(r, control), "{r:?} covers {control:?} on {screen:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn mini_menu_sits_beside_the_column() {
        for screen in SCREENS {
            for rtl in [false, true] {
                for rows in [0, 3, 40] {
                    let menu = mini_menu(screen, rows, rtl);
                    assert!(menu.rows.len() <= rows);
                    assert!(menu.rows.len() <= mini_menu_rows(screen[1]));
                    let controls: Vec<Rect> = world_controls(screen, rtl).into_iter().map(|(_, r)| r).collect();
                    let mut parts = vec![menu.set, menu.filter];
                    parts.extend(menu.toggles);
                    parts.extend(&menu.rows);
                    no_overlaps(&parts);
                    for p in &parts {
                        assert!(contains([0.0, 0.0, screen[0], screen[1]], *p));
                        for c in &controls {
                            assert!(!overlaps(*p, *c), "{p:?} hits {c:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn settings_grid_is_two_vanilla_columns() {
        let (buttons, back) = settings_grid([1001.0, 564.0], 12, false);
        assert_eq!(buttons[0], [295.0, 109.0, 200.0, 20.0]);
        assert_eq!(buttons[1], [505.0, 109.0, 200.0, 20.0]);
        assert_eq!(buttons[2][1], 133.0);
        assert_eq!(back, [400.0, 262.0, 200.0, 20.0]);
        assert_eq!(pager([1001.0, 564.0], false)[0], [295.0, 253.0, 75.0, 20.0]);
        for screen in SCREENS {
            for rtl in [false, true] {
                for count in [0, 1, 7, 12, 30] {
                    let (buttons, back) = settings_grid(screen, count, rtl);
                    assert_eq!(buttons.len(), count.min(12));
                    let pages = pager(screen, rtl);
                    let mut all = buttons.clone();
                    all.extend(pages);
                    no_overlaps(&all);
                    all.push(back);
                    for r in &all {
                        assert!(contains([0.0, 0.0, screen[0], screen[1]], *r), "{r:?} on {screen:?}");
                    }
                    if count > 1 {
                        assert_eq!(buttons[0][0] < buttons[1][0], !rtl);
                    }
                    assert!(!overlaps(back, buttons.last().copied().unwrap_or([0.0; 4])));
                }
            }
        }
    }

    #[test]
    fn waypoint_list_parts_fit() {
        let list = waypoint_list([1001.0, 564.0], 30, false);
        assert_eq!(list.actions[0], [295.0, 511.0, 75.0, 20.0]);
        assert_eq!(list.done, [400.0, 535.0, 200.0, 20.0]);
        assert_eq!(list.rows[0], [390.0, 59.0, 220.0, 18.0]);
        for screen in SCREENS {
            for rtl in [false, true] {
                for rows in [0, 1, 50] {
                    let list = waypoint_list(screen, rows, rtl);
                    let mut all = vec![list.bars[0], list.bars[1], list.options, list.set, list.new_set, list.done];
                    all.extend(list.actions);
                    no_overlaps(&all);
                    let mut fixed = all.clone();
                    fixed.push(list.filter);
                    for r in &fixed {
                        assert!(contains([0.0, 0.0, screen[0], screen[1]], *r), "{r:?}");
                    }
                    for row in &list.rows {
                        assert!(contains(list.band, *row));
                        assert!(!overlaps(*row, list.filter));
                        let (square, _) = waypoint_row_parts(*row, rtl);
                        assert!(contains(*row, square));
                    }
                    no_overlaps(&list.rows);
                }
            }
        }
    }

    #[test]
    fn waypoint_form_parts_fit() {
        for screen in SCREENS {
            for rtl in [false, true] {
                let f = waypoint_form(screen, rtl);
                let mut all = vec![f.world, f.dimension, f.set, f.color, f.name, f.world_3d, f.initials, f.enabled, f.confirm, f.cancel];
                all.extend(f.coords);
                no_overlaps(&all);
                for r in &all {
                    assert!(contains([0.0, 0.0, screen[0], screen[1]], *r), "{r:?} {screen:?}");
                }
            }
        }
        let f = waypoint_form([1001.0, 564.0], false);
        assert_eq!(f.confirm, [345.0, 262.0, 150.0, 20.0]);
    }

    #[test]
    fn nine_slice_covers_the_button_exactly() {
        for dst in [[10.0, 10.0, 200.0, 20.0], [0.0, 0.0, 75.0, 20.0], [5.0, 5.0, 4.0, 4.0]] {
            let pieces = nine_slice(dst, [200.0, 20.0], BUTTON_BORDER, 1.0);
            let rects: Vec<Rect> = pieces.iter().map(|(_, d)| *d).collect();
            no_overlaps(&rects);
            let area: f32 = rects.iter().map(|r| r[2] * r[3]).sum();
            assert!((area - dst[2] * dst[3]).abs() < 1e-2);
            for (src, d) in &pieces {
                assert!(contains([0.0, 0.0, 200.0, 20.0], *src));
                assert!(contains(dst, *d));
            }
        }
    }

    #[test]
    fn mirror_reflects() {
        assert_eq!(mirror([10.0, 5.0, 20.0, 3.0], 100.0), [70.0, 5.0, 20.0, 3.0]);
    }
}
