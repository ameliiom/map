// SPDX-License-Identifier: GPL-3.0-only
// Engine adapter: this module is injected under game, never into the standalone library.
use super::Game;
use super::minimap_compat::MapAccess;
use ::map::look;
use runtime::{Language, language, tr};

macro_rules! localized_format {
    ($en:literal, $ru:literal, $fr:literal, $es:literal, $de:literal $(, $arg:expr)* $(,)?) => {
        match language() {
            Language::Russian => format!($ru $(, $arg)*),
            Language::French => format!($fr $(, $arg)*),
            Language::Spanish => format!($es $(, $arg)*),
            Language::German => format!($de $(, $arg)*),
            Language::English => format!($en $(, $arg)*),
            Language::Arabic => runtime::arabic::format($en, &format!($en $(, $arg)*)),
        }
    };
}

use ::map::runtime::{
    self as runtime, Dimension, Waypoint,
    maps::{self, Layer, Portal, View},
};
use horizon_client::{
    chunk::Chunk, client_resources::ClientResources, extras::Corner, extras_hud::Corners,
    hud::HudBatch, mesher::TintSource,
};
use horizon_core::geometry::Direction;
use horizon_protocol::packets::play::{TrackedWaypoint, WaypointId, WaypointTarget, waypoint_operation};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

static WAYSTONES: Mutex<BTreeMap<u128, super::minimap_providers::Waystone>> = Mutex::new(BTreeMap::new());

static LOCATOR_WAYPOINTS: Mutex<Vec<TrackedWaypoint>> = Mutex::new(Vec::new());

fn apply_locator_waypoint(waypoints: &mut Vec<TrackedWaypoint>, packet: &TrackedWaypoint) {
    let slot = waypoints.iter().position(|point| point.id == packet.id);
    match (packet.operation, slot) {
        (waypoint_operation::TRACK, Some(i)) => waypoints[i] = packet.clone(),
        (waypoint_operation::TRACK, None) if waypoints.len() < 256 => waypoints.push(packet.clone()),
        (waypoint_operation::UNTRACK, Some(i)) => { waypoints.swap_remove(i); }
        (waypoint_operation::UPDATE, Some(i))
            if std::mem::discriminant(&waypoints[i].target) == std::mem::discriminant(&packet.target) =>
        {
            waypoints[i].target = packet.target;
        }
        _ => {}
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Map,
    Waypoints,
    Settings,
    Name,
    Caches,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MapPanel {
    None,
    Waypoints,
}

impl MapPanel {
    fn toggle(&mut self, panel: Self) {
        *self = if *self == panel { Self::None } else { panel };
    }
}

#[derive(Default)]
struct CacheUi {
    attention: bool,
    catalog_revision: u64,
    selected: Option<String>,
    scroll: usize,
    revision: u64,
    icons: BTreeMap<String, Arc<horizon_client::hud::pictures::Picture>>,
    operation: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    message: Option<String>,
    confirm: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Spot {
    dimension: Dimension,
    x: i32,
    z: i32,
    key: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Target {
    dimension: Dimension,
    x: i32,
    y: i32,
    z: i32,
}

/// The create / edit waypoint screen, also used to name a set.
struct NameForm {
    text: String,
    color: u32,
    target: Option<Target>,
    rename: Option<Spot>,
    group: String,
    focus: Field,
    kind: MarkerMode,
    group_edit: Option<GroupEdit>,
    background: Mode,
    dimension: Dimension,
    /// x, y, z and yaw as typed.
    coords: [String; 4],
    initials: String,
    show_3d: bool,
    disabled: bool,
}

#[derive(Clone,Copy,Default)]
enum MarkerMode {
    #[default]
    Saved,
    Temporary,
    Destination,
}

enum GroupEdit { Create, Rename(String) }

#[derive(Clone, Copy, PartialEq, Eq)]
enum FluidKind {
    Water,
    Lava,
}

#[derive(Clone, Copy)]
struct FluidColumn {
    kind: FluidKind,
    bottom: i32,
}

#[derive(Clone, Copy)]
struct ColumnInfo {
    height: i32,
    fluid: Option<FluidColumn>,
}

#[derive(Clone, Copy)]
struct Menu {
    height_approximate: bool,
    block: [i32; 2],
    height: Option<i32>,
    fluid: Option<FluidColumn>,
    dimension: Dimension,
    at: [f32; 2],
    portal: Option<Portal>,
    waypoint: Option<Spot>,
    radar_player: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Copy,
    Waypoint,
    ExportPng,
    ImportXaero,
    ImportXaeroCache,
    ImportXaeroSettings,
    ReloadRadarRules,
    NextXaeroProfile,
    TemporaryWaypoint,
    DestinationWaypoint,
    CopyDestination,
    Teleport,
    TeleportDestination,
    EditWaypoint,
    DeleteWaypoint,
    Toggle3D,
    Information,
    RefreshClaims,
    TeleportPlayer,
    OpenWaypoints,
    OpenSettings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeafTint {
    None,
    Fixed(u32),
    Grass,
    Foliage,
}

#[derive(Clone, Copy)]
struct LeafColor {
    /// Mean colour of the leaf texture, without its tint.
    rgb: u32,
    tint: LeafTint,
}

impl LeafColor {
    fn map_color(self, grass: Option<u32>, foliage: Option<u32>) -> Option<u32> {
        let tint = match self.tint {
            LeafTint::None => 0x00ff_ffff,
            LeafTint::Fixed(color) => color,
            LeafTint::Grass => grass?,
            LeafTint::Foliage => foliage?,
        };
        Some(multiply_color(self.rgb, tint))
    }
}

fn multiply_color(base: u32, tint: u32) -> u32 {
    let channel = |shift: u32| ((((base >> shift) & 255) * ((tint >> shift) & 255)) / 255) & 255;
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

struct LeafPalette {
    /// Block state id → the texture colour the game draws with, before tint.
    states: BTreeMap<u32, LeafColor>,
    snow: u32,
    ice: u32,
    /// Mean colour of the still water texture: the biome's water tint multiplies it, as the
    /// game draws water (and Xaero's accurate colours read it).
    water_texture: u32,
}
impl Default for LeafPalette {
    fn default() -> Self {
        Self {
            states: BTreeMap::new(),
            snow: 0xffffff,
            ice: 0xa0a0ff,
            water_texture: 0xffffff,
        }
    }
}

/// How much of a texture is opaque, 0 to 1.
fn texture_coverage(resources: &ClientResources, name: &str) -> f32 {
    let Some(pixels) = resources.texture(name) else { return 1. };
    let count = pixels.rgba.len() / 4;
    if count == 0 { return 1. }
    let alpha: u64 = pixels.rgba.chunks_exact(4).map(|pixel| u64::from(pixel[3])).sum();
    alpha as f32 / (count as f32 * 255.)
}

fn average_texture(resources: &ClientResources, name: &str) -> Option<u32> {
    let pixels = resources.texture(name)?;
    let mut sum = [0u64; 3];
    let mut weight = 0u64;
    for pixel in pixels.rgba.chunks_exact(4) {
        let alpha = u64::from(pixel[3]);
        if alpha == 0 {
            continue;
        }
        for (total, value) in sum.iter_mut().zip(&pixel[..3]) {
            *total += u64::from(*value) * alpha;
        }
        weight += alpha;
    }
    if weight == 0 {
        return None;
    }
    let red = (sum[0] / weight) as u32;
    let green = (sum[1] / weight) as u32;
    let blue = (sum[2] / weight) as u32;
    Some((red << 16) | (green << 8) | blue)
}

/// Mean texture colour and the game's tint source for every leaf block,
/// including vines and lily pads, whose tint the map takes from the biome.
/// Resource packs move the textures, so the result follows
/// `block_sprites_version`.
fn build_leaf_palette(game: &Game<'_>) -> LeafPalette {
    let names =
        horizon_client::block_sprites::texture_names(&game.pack.context.render.sprite_layers);
    let states = horizon_client::block_sprites::Lookup::new(
        &game.pack.context,
        game.renderer
            .as_ref()
            .map_or(u32::MAX, |r| r.sprite_split()),
        &names,
    );
    let resources = ClientResources::new(game.options.resources.clone());
    let book = game.res.facts();
    let mut textures = BTreeMap::<String, Option<u32>>::new();
    let mut palette = LeafPalette::default();
    for (index, facts) in book.states.iter().enumerate() {
        let id = horizon_world::registry::BlockStateId(index as u32);
        let name = facts.name.as_ref();
        if let Some(rgb) = book
            .states
            .get(id.0 as usize)
            .and_then(|state| horizon_client::blocks::MAP_COLORS.get(state.map_color as usize))
        {
            if name == "minecraft:snow_block" {
                palette.snow = *rgb;
            }
            if name == "minecraft:ice" {
                palette.ice = *rgb;
            }
        }
        let tinted_block = name.contains("leaves")
            || matches!(name, "minecraft:vine" | "minecraft:lily_pad")
            || book.states.get(id.0 as usize).is_some_and(|state| {
                state
                    .classes
                    .iter()
                    .any(|class| class.as_ref() == "LeavesBlock")
            });
        let sprites = states.state(id);
        let face = sprites.sides[Direction::Up as usize]
            .first()
            .or_else(|| sprites.sides.iter().find_map(|side| side.first()));
        let Some(face) = face else {
            continue;
        };
        // A full block whose top the biome tints (grass) reads as its texture times the tint;
        // plants keep their curated colour.
        let tinted_top = plant_color(name).is_none()
            && matches!(face.tint, Some(TintSource::Grass | TintSource::Foliage))
            && sprites.sides[Direction::Up as usize].first().is_some();
        if !tinted_block && !tinted_top {
            continue;
        }
        let texture = face.texture.as_str();
        let rgb = *textures
            .entry(texture.to_owned())
            .or_insert_with(|| average_texture(&resources, texture));
        let Some(mut rgb) = rgb else {
            continue;
        };
        // Seen from above, a canopy's holes show the shaded leaves under it: its colour is
        // the texture's mean darkened by the share of the texture that is open.
        if name.contains("leaves") {
            let coverage = texture_coverage(&resources, texture);
            let level = (coverage * 255.).round() as u32;
            rgb = multiply_color(rgb, (level << 16) | (level << 8) | level);
        }
        let tint = if name == "minecraft:lily_pad" {
            // Vanilla tints a lily pad with the biome's grass colour; the
            // mesher's constant is only a fallback for its own drawing pass.
            LeafTint::Grass
        } else {
            match face.tint {
                None => LeafTint::None,
                Some(TintSource::Fixed(color)) => LeafTint::Fixed(color),
                Some(TintSource::Grass) => LeafTint::Grass,
                Some(TintSource::Foliage) => LeafTint::Foliage,
                // Dry foliage and water belong to non-leaf blocks; keep their
                // map colour instead of pretending a wrong tint.
                Some(_) => continue,
            }
        };
        palette.states.insert(id.0, LeafColor { rgb, tint });
    }
    if let Some(rgb) = average_texture(&resources, "block/water_still") {
        let light = |shift: u32| ((((rgb >> shift) & 255) as f32 * maps::WATER_LIGHT).round() as u32).min(255) << shift;
        palette.water_texture = light(16) | light(8) | light(0);
    }
    palette
}

#[derive(Clone, Copy, Default)]
struct BiomeTints {
    grass: Option<u32>,
    foliage: Option<u32>,
    water: Option<u32>,
}

// Minecraft blends the biome colours over a radius instead of switching at a
// line. Sample a small grid around the block and average each requested tint.
const BIOME_BLEND_RADIUS: i32 = 4;
const BIOME_BLEND_STEP: i32 = 2;

fn blend_biome_tints(
    palette: &horizon_client::biome::Biomes,
    at: [i32; 2],
    radius: i32,
    step: i32,
    grass: bool,
    foliage: bool,
    water: bool,
    mut biome: impl FnMut(i32, i32) -> Option<u16>,
) -> BiomeTints {
    if !(grass || foliage || water) {
        return BiomeTints::default();
    }
    let step = step.max(1);
    let mut sums = [[0u64; 3]; 3];
    let mut counts = [0u64; 3];
    let mut dz = -radius;
    while dz <= radius {
        let mut dx = -radius;
        while dx <= radius {
            let x = at[0].saturating_add(dx);
            let z = at[1].saturating_add(dz);
            if let Some(id) = biome(x, z)
                && let Some(info) = palette.list.get(usize::from(id))
            {
                if grass {
                    add_color(
                        &mut sums[0],
                        &mut counts[0],
                        palette.grass(id, f64::from(x), f64::from(z)),
                    );
                }
                if foliage {
                    add_color(&mut sums[1], &mut counts[1], info.foliage);
                }
                if water {
                    add_color(&mut sums[2], &mut counts[2], info.water);
                }
            }
            dx += step;
        }
        dz += step;
    }
    BiomeTints {
        grass: average_color(sums[0], counts[0]),
        foliage: average_color(sums[1], counts[1]),
        water: average_color(sums[2], counts[2]),
    }
}

fn add_color(sum: &mut [u64; 3], count: &mut u64, color: u32) {
    for (channel, shift) in sum.iter_mut().zip([16, 8, 0]) {
        *channel += u64::from((color >> shift) & 255);
    }
    *count += 1;
}

fn average_color(sum: [u64; 3], count: u64) -> Option<u32> {
    if count == 0 {
        return None;
    }
    let channel = |index: usize| (sum[index] / count) as u32;
    Some((channel(0) << 16) | (channel(1) << 8) | channel(2))
}

struct State {
    caches: CacheUi,
    world: String,
    session: u64,
    dimension: Dimension,
    seen: BTreeMap<(i32, i32), (Weak<Chunk>, Layer, Instant)>,
    appearance: maps::Appearance,
    chunk_keys: BTreeSet<(i32, i32)>,
    chunk_revision: u64,
    chunk_cursor: Option<(i32, i32)>,
    layers: BTreeMap<Dimension, Layer>,
    cave_pending: [Option<(bool,Instant)>;2],
    cave_open: [bool;2],
    selected: Dimension,
    large: bool,
    map_opened: Option<Instant>,
    camera: maps::Camera,
    marker: [f64; 2],
    footsteps: VecDeque<(Dimension, [i32; 2])>,
    elapsed: f64,
    frame: Option<Arc<maps::Frame>>,
    page_images: Vec<(
        Arc<maps::AtlasPage>,
        Arc<horizon_client::hud::pictures::Picture>,
    )>,
    dragging: bool,
    close: bool,
    mode: Mode,
    menu: Option<Menu>,
    name_form: Option<NameForm>,
    waypoints: Vec<Waypoint>,
    waypoint_label_source: Vec<Waypoint>,
    waypoint_labels: Vec<String>,
    waypoint_letter_mode: runtime::minimap::WaypointLetters,
    portals: Vec<(Dimension, [i32; 3], Portal)>,
    waypoint_scroll: usize,
    waypoint_search: String,
    waypoint_search_focus: bool,
    waypoint_sort: usize,
    waypoint_dimension: Dimension,
    waypoint_groups_open: bool,
    waypoint_groups_scroll: usize,
    waypoint_selected: Option<u64>,
    waypoint_group: Option<String>,
    destinations: BTreeMap<u64,Instant>,
    slime_seed: Option<i64>,
    slime_preview: bool,
    slime_overlay: bool,
    claims: BTreeMap<::map::map_claims::Chunk, Option<String>>,
    opac: super::minimap_providers::OpenPac,
    info_displays: Option<Vec<::map::xaero_settings::InfoDisplay>>,
    notice: Option<(String, Instant)>,
    export: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    waypoint_import: Option<std::sync::mpsc::Receiver<Result<usize, String>>>,
    cache_import: Option<std::sync::mpsc::Receiver<Result<usize, String>>>,
    settings_import: Option<std::sync::mpsc::Receiver<std::io::Result<::map::xaero_settings::Import>>>,
    xaero_radar_enabled: Option<bool>,
    xaero_profile: Option<String>,
    map_items: [Option<String>;2],
    pending_copy: Option<String>,
    pending_command: Option<String>,
    map_focus: Option<Spot>,
    map_panel: MapPanel,
    map_panel_scroll: usize,
    /// The mini-menu's filter, whether it has the keyboard, and whether a hop closes it.
    mini_filter: String,
    mini_filter_focus: bool,
    close_on_hop: bool,
    /// The last waypoint the hop control centred on.
    hop: usize,
    /// Team's players' faces by name (`refresh_team_skins`).
    team_skins: BTreeMap<String, String>,
    map_settings_change: Option<(String, String)>,
    settings_return: Option<Mode>,
    /// The settings category open, or none for the home page.
    settings_tab: Option<usize>,
    settings_scroll: usize,
    /// Chunks the server sent since the last sampling pass. A waypoint's biome
    /// is sampled when its chunk arrives, rather than polling every point.
    biome_chunks: BTreeSet<(i32, i32)>,
    leaves: Option<(u64, Arc<LeafPalette>)>,
    lod_elapsed: f64,
    lod_sources:
        BTreeMap<(Dimension, maps::LodKey), (Weak<Vec<horizon_client::lod::Column>>, Instant)>,
    info: Option<Menu>,
    info_scroll: usize,
}
fn known_lod_sources(state: &State, dimension: Dimension) -> BTreeMap<(i32, i32, u8), usize> {
    state
        .lod_sources
        .iter()
        .filter(|((d, _), (source, _))| *d == dimension && source.strong_count() != 0)
        .map(|((_, key), (source, _))| ((key.x, key.z, key.level), source.as_ptr() as usize))
        .collect()
}
impl Default for State {
    fn default() -> Self {
        Self {
            caches: CacheUi::default(),
            world: String::new(),
            session: 0,
            dimension: Dimension::Player,
            seen: BTreeMap::new(),
            appearance: maps::Appearance::default(),
            chunk_keys: BTreeSet::new(),
            chunk_revision: 0,
            chunk_cursor: None,
            layers: BTreeMap::new(),
            cave_pending: [None;2],
            cave_open: [false;2],
            selected: Dimension::Player,
            large: false,
            map_opened: None,
            camera: maps::Camera::new([0.0; 2], 2.0),
            marker: [0.0; 2],
            footsteps: VecDeque::new(),
            elapsed: 1.0,
            frame: None,
            page_images: Vec::new(),
            dragging: false,
            close: false,
            mode: Mode::Map,
            menu: None,
            name_form: None,
            waypoints: Vec::new(),
            waypoint_label_source: Vec::new(),
            waypoint_labels: Vec::new(),
            waypoint_letter_mode: runtime::minimap::WaypointLetters::default(),
            portals: Vec::new(),
            waypoint_scroll: 0,
            waypoint_search: String::new(),
            waypoint_search_focus: false,
            waypoint_sort: 0,
            waypoint_dimension: Dimension::Player,
            waypoint_groups_open: false,
            waypoint_groups_scroll: 0,
            waypoint_selected: None,
            waypoint_group: None,
            destinations: BTreeMap::new(),
            slime_seed: None,
            slime_preview: false,
            slime_overlay: false,
            claims: BTreeMap::new(),
            opac: super::minimap_providers::OpenPac::default(),
            info_displays: None,
            notice: None,
            export: None,
            waypoint_import: None,
            cache_import: None,
            settings_import: None,
            xaero_radar_enabled: None,
            xaero_profile: None,
            map_items: [None,None],
            pending_copy: None,
            pending_command: None,
            map_focus: None,
            map_panel: MapPanel::None,
            map_panel_scroll: 0,
            mini_filter: String::new(),
            mini_filter_focus: false,
            close_on_hop: true,
            hop: 0,
            team_skins: BTreeMap::new(),
            map_settings_change: None,
            settings_return: None,
            settings_tab: None,
            settings_scroll: 0,
            biome_chunks: BTreeSet::new(),
            leaves: None,
            lod_elapsed: 1.0,
            lod_sources: BTreeMap::new(),
            info: None,
            info_scroll: 0,
        }
    }
}
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .get_or_init(|| Mutex::new(State::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}
fn rectangle(width: f32, height: f32) -> [f32; 4] {
    [0.0, 0.0, width.max(1.0), height.max(1.0)]
}
fn inside(mouse: [f32; 2], rect: [f32; 4]) -> bool {
    mouse[0] >= rect[0]
        && mouse[1] >= rect[1]
        && mouse[0] < rect[0] + rect[2]
        && mouse[1] < rect[1] + rect[3]
}

// Ground cover the map leaves out: grass, ferns, bushes and dry grass.
fn hidden_cover(classes: &[Arc<str>], name: &str) -> bool {
    classes.iter().any(|class| {
        matches!(
            class.as_ref(),
            "TallGrassBlock" | "DryVegetationBlock" | "BushBlock" | "FireflyBushBlock"
        )
    }) || matches!(
        name,
        "minecraft:tall_grass" | "minecraft:large_fern" | "minecraft:grass"
    )
}

// Curated petal, crop and plant colours: the vanilla map colour of most plants
// is one generic green, which hides what actually grows there.
fn plant_color(name: &str) -> Option<u32> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    if let Some(inner) = short.strip_prefix("potted_")
        && let Some(color) = plant_color(&format!("minecraft:{inner}"))
    {
        return Some(color);
    }
    Some(match short {
        "poppy" => 0xc0392b,
        "dandelion" => 0xf1c40f,
        "blue_orchid" => 0x2e86c1,
        "allium" => 0xb57edc,
        "azure_bluet" => 0xf0f0f0,
        "oxeye_daisy" => 0xf5f5dc,
        "cornflower" => 0x4a69bd,
        "lily_of_the_valley" => 0xf8f8f8,
        "wither_rose" => 0x2c2c2c,
        "orange_tulip" => 0xe67e22,
        "red_tulip" => 0xc0392b,
        "white_tulip" => 0xecf0f1,
        "pink_tulip" => 0xe88bb0,
        "sunflower" => 0xf4d03f,
        "lilac" => 0xc39bd3,
        "rose_bush" => 0xc0392b,
        "peony" => 0xf5b7d7,
        "torchflower" => 0xe67e22,
        "cactus_flower" => 0xe88bb0,
        "open_eyeblossom" => 0xee8a26,
        "closed_eyeblossom" => 0x7f8c8d,
        "golden_dandelion" => 0xd4a017,
        "wildflowers" | "pink_petals" => 0xf08ac0,
        "spore_blossom" => 0xe88bb0,
        "chorus_flower" => 0xa569bd,
        "wheat" => 0xd9b44a,
        "carrots" => 0xe67e22,
        "potatoes" => 0xb8a06a,
        "beetroots" => 0xa03040,
        "nether_wart" => 0xa03040,
        "melon_stem" | "attached_melon_stem" | "pumpkin_stem" | "attached_pumpkin_stem" => 0x5b8a3c,
        "pitcher_crop" => 0x9dc183,
        "torchflower_crop" => 0xe67e22,
        "oak_sapling" | "spruce_sapling" | "birch_sapling" | "jungle_sapling"
        | "acacia_sapling" | "dark_oak_sapling" | "cherry_sapling" | "pale_oak_sapling"
        | "poplar_sapling" => 0x4f7f2f,
        "mangrove_propagule" | "bamboo_sapling" => 0x6a9f3f,
        "red_mushroom" => 0xd94b44,
        "brown_mushroom" => 0x9a755c,
        "crimson_fungus" => 0xa03030,
        "warped_fungus" => 0x4a8f7f,
        "crimson_roots" => 0x8a3030,
        "warped_roots" => 0x3f7f7f,
        "nether_sprouts" => 0x3f8f5f,
        "cave_vines" | "cave_vines_plant" => 0x3f6f2f,
        "twisting_vines" | "twisting_vines_plant" => 0x3fa0a0,
        "weeping_vines" | "weeping_vines_plant" => 0x8a3030,
        "sugar_cane" => 0x8fbf5a,
        "bamboo" => 0xb7c65a,
        "cactus" => 0x3f7f3f,
        "sea_pickle" => 0x6a8f3f,
        "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant" => 0x3f9f3f,
        "moss_carpet" | "pale_moss_carpet" | "moss_block" => 0x5f8f3f,
        "fern" | "short_grass" | "tall_grass" | "large_fern" => 0x4f8f3f,
        "dead_bush" | "bush" | "red_shrub" | "firefly_bush" => 0x8a6a3a,
        "leaf_litter" => 0x8a6a3a,
        "pale_hanging_moss" => 0x8a9f8a,
        "hanging_roots" => 0x8a6a4a,
        "mangrove_roots" => 0x6a5a3a,
        "big_dripleaf" | "big_dripleaf_stem" | "small_dripleaf" => 0x4f8f3f,
        "azalea" | "flowering_azalea" | "azalea_bush" | "flowering_azalea_bush" => 0x4f8f3f,
        "sweet_berry_bush" => 0x5f8f3f,
        "nether_wart_block" => 0xa03040,
        "warped_wart_block" => 0x2f8f8f,
        _ => return None,
    })
}

// The dye of a stained pane, whose vanilla map colour is unset.
fn dye_color(name: &str) -> Option<u32> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    let dye = short.strip_suffix("_stained_glass_pane")?;
    Some(match dye {
        "white" => 0xecf0f1,
        "orange" => 0xe67e22,
        "magenta" => 0xc0398b,
        "light_blue" => 0x5dade2,
        "yellow" => 0xf4d03f,
        "lime" => 0x7dce3a,
        "pink" => 0xf5b7d7,
        "gray" => 0x7f8c8d,
        "light_gray" => 0xbdc3c7,
        "cyan" => 0x2e9a9a,
        "purple" => 0x8e44ad,
        "blue" => 0x2e4fa3,
        "brown" => 0x8a5a2b,
        "green" => 0x2e7d32,
        "red" => 0xc0392b,
        "black" => 0x1c1c1c,
        _ => return None,
    })
}

fn portal_tint(kind: Portal) -> (u32, u8) {
    match kind {
        Portal::Nether => (0x9b4dff, 150),
        Portal::End => (0x4a1a6a, 190),
        Portal::Gateway => (0x1a0a2a, 190),
    }
}

fn known_biome(game: &Game<'_>, dimension: Dimension, at: [i32; 3]) -> Option<String> {
    let player = game.native_player()?;
    if player.dimension != dimension.id()
        || !game
            .world
            .chunks
            .contains_key(&(at[0].div_euclid(16), at[2].div_euclid(16)))
    {
        return None;
    }
    game.map_biome(at)
}

fn localized_biome(id: &str) -> String {
    horizon_client::lang::get_or(&format!("biome.{}", id.replace(':', ".")), id)
}

// A small skull for automatic death waypoints, drawn from relative blocks so
// it stays legible on the map, in the world and in the waypoint list.
fn draw_skull(game: &Game<'_>, batch: &mut HudBatch, at: [f32; 2], size: f32) {
    let u = size / 8.0;
    let bone = 0xffe9e6dc;
    let socket = 0xff14171c;
    game.native_fill(
        batch,
        [at[0] - 3.0 * u, at[1] - 3.0 * u, 6.0 * u, 5.0 * u],
        bone,
    );
    game.native_fill(
        batch,
        [at[0] - 2.0 * u, at[1] + 2.0 * u, 4.0 * u, 1.5 * u],
        bone,
    );
    for x in [-2.0, 0.75] {
        game.native_fill(
            batch,
            [at[0] + x * u, at[1] - 1.5 * u, 1.25 * u, 1.75 * u],
            socket,
        );
    }
    game.native_fill(
        batch,
        [at[0] - 0.5 * u, at[1] + 0.5 * u, 1.0 * u, 1.25 * u],
        socket,
    );
    for x in [-1.0, 0.5] {
        game.native_fill(
            batch,
            [at[0] + x * u, at[1] + 2.0 * u, 0.5 * u, 1.5 * u],
            socket,
        );
    }
}

fn portal_label(kind: Portal) -> &'static str {
    match kind {
        Portal::Nether => tr(
            "Nether portal",
            "Портал Незера",
            "Portail du Nether",
            "Portal del Nether",
            "Netherportal",
        ),
        Portal::End => tr(
            "End portal",
            "Портал Энда",
            "Portail de l’End",
            "Portal del End",
            "Endportal",
        ),
        Portal::Gateway => tr(
            "End gateway",
            "Врата Энда",
            "Passerelle de l’End",
            "Puerta de enlace del End",
            "Endtransitportal",
        ),
    }
}

// The world block under the pointer, when the pointer is over the map.
fn hovered_block(view: &View, rect: [f32; 4], mouse: [f32; 2]) -> Option<[i32; 2]> {
    if !inside(mouse, rect) {
        return None;
    }
    let x = f64::from((mouse[0] - rect[0]) / rect[2]) * view.width as f64;
    let y = f64::from((mouse[1] - rect[1]) / rect[3]) * view.height as f64;
    Some(view.block_at(x, y))
}

// A block's rectangle in GUI pixels, from the frame's own pixel mapping.
fn block_rect(view: &View, rect: [f32; 4], block: [i32; 2]) -> [f32; 4] {
    let first = view.screen_at([f64::from(block[0]), f64::from(block[1])]);
    let second = view.screen_at([f64::from(block[0]) + 1.0, f64::from(block[1]) + 1.0]);
    let scale_x = rect[2] / view.width as f32;
    let scale_y = rect[3] / view.height as f32;
    [
        rect[0] + first[0].min(second[0]) as f32 * scale_x,
        rect[1] + first[1].min(second[1]) as f32 * scale_y,
        (first[0] - second[0]).abs() as f32 * scale_x,
        (first[1] - second[1]).abs() as f32 * scale_y,
    ]
}

fn highlight(game: &Game<'_>, batch: &mut HudBatch, view: &View, rect: [f32; 4], block: [i32; 2]) {
    let cell = block_rect(view, rect, block);
    if cell[2] < 1.0 || cell[3] < 1.0 {
        return;
    }
    let left = cell[0].max(rect[0]);
    let top = cell[1].max(rect[1]);
    let right = (cell[0] + cell[2]).min(rect[0] + rect[2]);
    let bottom = (cell[1] + cell[3]).min(rect[1] + rect[3]);
    let (width, height) = (right - left, bottom - top);
    if width < 1.0 || height < 1.0 {
        return;
    }
    game.native_fill(batch, [left, top, width, height], 0x40ffffff);
    let line = 1.0_f32.min(width).min(height);
    game.native_fill(batch, [left, top, width, line], 0xff000000);
    game.native_fill(batch, [left, top + height - line, width, line], 0xff000000);
    game.native_fill(batch, [left, top, line, height], 0xff000000);
    game.native_fill(batch, [left + width - line, top, line, height], 0xff000000);
}

fn first_letter(name: &str) -> char {
    name.chars()
        .find(|c| !c.is_whitespace())
        .unwrap_or('?')
        .to_uppercase()
        .next()
        .unwrap_or('?')
}
fn refresh_waypoint_labels(state: &mut State, mode: runtime::minimap::WaypointLetters) {
    if state.waypoint_label_source != state.waypoints || state.waypoint_letter_mode != mode {
        state.waypoint_labels = runtime::waypoint_labels(&state.waypoints, mode);
        state.waypoint_label_source.clone_from(&state.waypoints);
        state.waypoint_letter_mode = mode;
    }
}
fn waypoint_text<'a>(
    state: &'a State,
    index: usize,
    waypoint: &'a Waypoint,
) -> std::borrow::Cow<'a, str> {
    if let Some(initials) = &waypoint.initials {
        return std::borrow::Cow::Borrowed(initials);
    }
    state.waypoint_labels.get(index).map_or_else(
        || std::borrow::Cow::Owned(first_letter(&waypoint.name).to_string()),
        |s| std::borrow::Cow::Borrowed(s.as_str()),
    )
}

fn waypoint_argb(color: u32) -> u32 {
    0xff00_0000 | (color & 0x00ff_ffff)
}

fn waypoint_scale(game: &Game<'_>, key: &str) -> f32 {
    1. + f32::from(game.map_option(key).unwrap_or_default().min(17)) * 0.125
}

fn set_opacity(batch: &mut HudBatch, start: usize, percent: u8) {
    if percent >= 100 { return; }
    for vertex in &mut batch.vertices[start..] {
        let alpha = (vertex.color >> 24) * u32::from(percent) / 100;
        vertex.color = (vertex.color & 0x00ff_ffff) | (alpha << 24);
    }
}

fn default_waypoint_color(state: &State) -> u32 {
    runtime::WAYPOINT_COLORS[state.waypoints.len() % runtime::WAYPOINT_COLORS.len()]
}

fn fit_text(game: &Game<'_>, text: &str, width: f32) -> String {
    if game.map_text_width(text) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for c in text.chars() {
        let mut candidate = out.clone();
        candidate.push(c);
        candidate.push('…');
        if game.map_text_width(&candidate) > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// A world-map waypoint: an opaque pennant of its colour with the initial and a diagonal drop
/// shadow; a death point is a black pennant with a skull.
fn draw_pennant(
    game: &Game<'_>,
    batch: &mut HudBatch,
    view: &View,
    rect: [f32; 4],
    waypoint: &Waypoint,
    letter: &str,
    scale: f32,
) {
    let at = world_screen(view, rect, [f64::from(waypoint.x) + 0.5, f64::from(waypoint.z) + 0.5]);
    if !inside(at, rect) {
        return;
    }
    let skull = waypoint.icon == runtime::WaypointIcon::Skull;
    let width = game.map_text_width(letter) * look::PENNANT_FONT;
    let scale = scale.max(0.25) * ((width + 4.) / look::PENNANT[0]).max(1.);
    let (body, [left, right, tip]) = look::pennant(at, scale);
    let fill = if skull { look::DEATH_FILL } else { waypoint_argb(if waypoint.color == 0 { 0xffffff } else { waypoint.color }) };
    let backgrounds = game.map_option("world_waypoint_backgrounds") != Some(0);
    if backgrounds {
        let o = look::PENNANT_SHADOW_OFFSET * scale;
        let shift = |p: [f32; 2]| [p[0] + o, p[1] + o];
        game.native_fill(batch, [body[0] + o, body[1] + o, body[2], body[3]], look::PENNANT_SHADOW);
        game.native_quad(batch, [shift(left), shift(right), shift(tip), shift(left)], look::PENNANT_SHADOW);
        game.native_fill(batch, body, fill);
        game.native_quad(batch, [left, right, tip, left], fill);
    }
    let centre = [body[0] + body[2] * 0.5, body[1] + body[3] * 0.5];
    if skull {
        draw_skull(game, batch, centre, body[2] * 0.7);
        return;
    }
    let color = if backgrounds { look::INITIAL_TEXT } else { fill };
    let origin = [(centre[0] - width * 0.5).round(), (centre[1] - 4. * look::PENNANT_FONT).round()];
    let first = batch.vertices.len();
    game.map_text(batch, letter, origin, color, true);
    batch.scale_since(first, origin[0], origin[1], look::PENNANT_FONT, look::PENNANT_FONT);
}

fn draw_portal_marker(
    game: &Game<'_>,
    batch: &mut HudBatch,
    view: &View,
    rect: [f32; 4],
    at: [i32; 3],
    kind: Portal,
) {
    let pixel = view.screen_at([f64::from(at[0]) + 0.5, f64::from(at[2]) + 0.5]);
    let x = rect[0] + pixel[0] as f32 * rect[2] / view.width as f32;
    let y = rect[1] + pixel[1] as f32 * rect[3] / view.height as f32;
    if !inside([x, y], rect) {
        return;
    }
    let (color, _) = portal_tint(kind);
    game.native_fill(batch, [x - 3.5, y - 3.5, 7.0, 7.0], 0xff101010);
    game.native_fill(batch, [x - 2.5, y - 2.5, 5.0, 5.0], color);
    match kind {
        Portal::Nether => {
            game.native_fill(batch, [x - 0.5, y - 2.0, 1.0, 4.0], 0xfff0e0ff);
            game.native_fill(batch, [x - 2.0, y - 0.5, 4.0, 1.0], 0xfff0e0ff);
        }
        Portal::End => {
            game.native_fill(batch, [x - 1.0, y - 1.0, 2.0, 2.0], 0xffe0d0ff);
        }
        Portal::Gateway => {
            game.native_fill(batch, [x - 1.5, y - 1.5, 3.0, 3.0], 0xffb090ff);
        }
    }
}

// The visible height at a hovered column, in the player's current dimension
// (the only one whose loaded chunks the client holds). Water and lava also
// report the lowest block of their column.
fn hovered_column(game: &Game<'_>, layer: Layer, block: [i32; 2]) -> Option<ColumnInfo> {
    if layer == Layer::Below(maps::FULL_CAVE) {
        return None;
    }
    let chunk = game
        .world
        .chunks
        .get(&(block[0].div_euclid(16), block[1].div_euclid(16)))?;
    let (x, z) = (block[0].rem_euclid(16), block[1].rem_euclid(16));
    let facts = game.res.facts();
    let ceiling = match layer {
        Layer::Surface => i32::MAX,
        Layer::Below(y) => y,
    };
    let height = chunk.highest(game.world.air.0, |y| {
        if y > ceiling {
            return false;
        }
        let id = chunk.block(x, y, z);
        let Some(entry) = facts.states.get(id.0 as usize) else {
            return false;
        };
        if hidden_cover(entry.classes.as_ref(), entry.name.as_ref()) {
            return false;
        }
        !facts.is_air(id)
            && (matches!(
                entry.fluid,
                horizon_client::blocks::Fluid::Water | horizon_client::blocks::Fluid::Lava
            ) || horizon_client::blocks::MAP_COLORS
                .get(entry.map_color as usize)
                .copied()
                .unwrap_or(0)
                != 0)
    })?;
    let fluid = facts
        .states
        .get(chunk.block(x, height, z).0 as usize)
        .and_then(|entry| match entry.fluid {
            horizon_client::blocks::Fluid::Water => Some(FluidKind::Water),
            horizon_client::blocks::Fluid::Lava => Some(FluidKind::Lava),
            horizon_client::blocks::Fluid::None | horizon_client::blocks::Fluid::Modded => None,
        })
        .map(|kind| {
            let mut bottom = height;
            while bottom > chunk.min_y() {
                let below = chunk.block(x, bottom - 1, z);
                let Some(entry) = facts.states.get(below.0 as usize) else {
                    break;
                };
                let same = match kind {
                    FluidKind::Water => entry.fluid == horizon_client::blocks::Fluid::Water,
                    FluidKind::Lava => entry.fluid == horizon_client::blocks::Fluid::Lava,
                };
                if !same {
                    break;
                }
                bottom -= 1;
            }
            FluidColumn { kind, bottom }
        });
    Some(ColumnInfo { height, fluid })
}


fn teleport_allowed(game: &Game<'_>) -> bool {
    game.ui.registries.permission >= 2 && game.map_option("map_teleport_allowed") != Some(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PortalTarget {
    Exact { dimension: Dimension, at: [i32; 3] },
    Spawn,
    Outer,
    Unknown,
}

fn portal_target(state: &State, menu: Menu, player_y: i32) -> Option<PortalTarget> {
    let kind = menu.portal?;
    Some(match kind {
        Portal::Nether => {
            let (dimension, nominal) = match menu.dimension {
                Dimension::Overworld => (
                    Dimension::Nether,
                    [
                        menu.block[0] / 8,
                        menu.height.unwrap_or(player_y),
                        menu.block[1] / 8,
                    ],
                ),
                Dimension::Nether => (
                    Dimension::Overworld,
                    [
                        menu.block[0] * 8,
                        menu.height.unwrap_or(player_y),
                        menu.block[1] * 8,
                    ],
                ),
                _ => return Some(PortalTarget::Unknown),
            };
            let nearest = state
                .portals
                .iter()
                .filter(|(at_dimension, _, at_kind)| {
                    *at_dimension == dimension && *at_kind == Portal::Nether
                })
                .map(|(_, at, _)| *at)
                .filter(|at| {
                    let dx = f64::from(at[0] - nominal[0]);
                    let dz = f64::from(at[2] - nominal[2]);
                    dx * dx + dz * dz <= 128.0 * 128.0
                })
                .min_by_key(|at| {
                    let dx = i64::from(at[0] - nominal[0]);
                    let dz = i64::from(at[2] - nominal[2]);
                    dx * dx + dz * dz
                });
            PortalTarget::Exact {
                dimension,
                at: nearest.unwrap_or(nominal),
            }
        }
        Portal::End => {
            if menu.dimension == Dimension::End {
                PortalTarget::Spawn
            } else {
                PortalTarget::Exact {
                    dimension: Dimension::End,
                    at: [100, 49, 0],
                }
            }
        }
        Portal::Gateway => PortalTarget::Outer,
    })
}

fn portal_target_text(target: PortalTarget) -> String {
    match target {
        PortalTarget::Exact { dimension, at } => {
            format!("{}: {} {} {}", dimension.label(), at[0], at[1], at[2])
        }
        PortalTarget::Spawn => tr(
            "Overworld: spawn point",
            "Верхний мир: точка спавна",
            "Surface : point d’apparition",
            "Mundo normal: punto de aparición",
            "Oberwelt: Spawnpunkt",
        )
        .into(),
        PortalTarget::Outer => tr(
            "End: outer islands",
            "Энд: внешние острова",
            "End : îles extérieures",
            "End: islas exteriores",
            "Ende: äußere Inseln",
        )
        .into(),
        PortalTarget::Unknown => tr(
            "unknown destination",
            "назначение неизвестно",
            "destination inconnue",
            "destino desconocido",
            "Unbekanntes Ziel",
        )
        .into(),
    }
}

/// Whether the waypoint under the pointer is shown in the 3D world; defaults to
/// true for a point no longer in the current snapshot.
fn waypoint_show_3d(state: &State, spot: Spot) -> bool {
    state
        .waypoints
        .iter()
        .find(|waypoint| {
            waypoint.key() == spot.key
        })
        .map_or(true, |waypoint| waypoint.show_3d)
}

fn teleport_command(dimension: Dimension, at: [i32; 3], from: Dimension) -> String {
    let coords = format!(
        "{:.3} {:.3} {:.3}",
        f64::from(at[0]) + 0.5,
        f64::from(at[1]),
        f64::from(at[2]) + 0.5
    );
    if dimension == from {
        format!("tp {coords}")
    } else {
        format!("execute in {} run tp {coords}", dimension.id())
    }
}

fn configured_teleport(
    game: &Game<'_>, dimension: Dimension, from: Dimension,
    point: Option<&Waypoint>, at: [i32;3], unknown_height: bool,
) -> Option<String> {
    let cross_dimension = dimension != from;
    if cross_dimension && point.is_some() && game.map_option("waypoint_cross_dimension")==Some(0) {
        return None;
    }
    let partial = game.map_option(if point.is_some() {"waypoint_partial_y"} else {"map_partial_y"}) != Some(0);
    let y = if partial && unknown_height && !cross_dimension {"~".to_owned()} else {at[1].to_string()};
    let yaw = point.and_then(|point| point.options.rotation.then_some(point.options.yaw));
    let (key, default) = if yaw.is_some() {
        ("tp_rotation","tp @s {x} {y} {z} {yaw} ~")
    } else if point.is_some() {
        ("tp_waypoint","tp @s {x} {y} {z}")
    } else if cross_dimension {
        ("tp_map_dimension","execute as @s in {d} run tp {x} {y} {z}")
    } else {
        ("tp_map","tp @s {x} {y} {z}")
    };
    use ::map::xaero_settings;
    let directory=game.native_data_dir(runtime::MOD_ID);
    let profile=xaero_settings::active_profile(&directory);
    let templates=xaero_settings::teleport_commands(&xaero_settings::commands_directory(&directory,profile.as_deref())).unwrap_or_default();
    let template = templates.get(key).map(String::as_str).unwrap_or(default);
    let command = template.replace("{x}",&format!("{:.3}",f64::from(at[0])+0.5))
        .replace("{z}",&format!("{:.3}",f64::from(at[2])+0.5))
        .replace("{y}",&y)
        .replace("{d}",dimension.id())
        .replace("{yaw}",&yaw.map_or_else(||"~".to_owned(),|yaw|yaw.to_string()));
    if cross_dimension && point.is_some() && !template.contains("{d}") {
        Some(format!("execute in {} run {}",dimension.id(),command.trim_start_matches('/')))
    } else {Some(command)}
}

fn default_waypoint_name(state: &State) -> String {
    localized_format!(
        "Waypoint {}",
        "Вейпоинт {}",
        "Repère {}",
        "Marcador {}",
        "Wegpunkt {}",
        state.waypoints.len() + 1
    )
}

fn perform_menu(game: &Game<'_>, state: &mut State, menu: Menu, action: MenuAction) {
    let Some(player) = game.native_player() else {
        state.menu = None;
        return;
    };
    let fallback = player.y.floor() as i32;
    let y = menu.height.unwrap_or(fallback);
    let target = portal_target(state, menu, fallback);
    state.menu = None;
    match action {
        MenuAction::ReloadRadarRules => {
            let directory = game.native_data_dir(runtime::MOD_ID);
            let message = match ::map::radar::load_rules(&directory, true) {
                Ok((loaded, skipped)) => localized_format!(
                    "Radar rules {} · skipped {} · radar-rules.txt", "Правил радара {} · пропущено {} · radar-rules.txt",
                    "Règles radar {} · ignorées {} · radar-rules.txt", "Reglas {} · omitidas {} · radar-rules.txt",
                    "Radarregeln {} · übersprungen {} · radar-rules.txt", loaded, skipped
                ),
                Err(error) => error.to_string(),
            };
            state.notice = Some((message, Instant::now()));
        }
        MenuAction::ImportXaeroSettings | MenuAction::NextXaeroProfile => {
            if state.settings_import.is_some() { return; }
            let source = game.native_data_dir(runtime::MOD_ID);
            let next=action==MenuAction::NextXaeroProfile;
            let (send, reply) = std::sync::mpsc::channel();
            let worker = std::thread::Builder::new().name("map-settings-import".into()).spawn(move || {
                let _ = send.send(::map::xaero_settings::read_active(&source,next));
            });
            match worker {
                Ok(_) => state.settings_import = Some(reply),
                Err(error) => state.notice = Some((error.to_string(), Instant::now())),
            }
        }
        MenuAction::ImportXaeroCache => {
            if state.cache_import.is_some() {
                return;
            }
            let directory = game.native_data_dir(runtime::MOD_ID);
            let source = directory.join("xaero-cache").join(menu.dimension.name());
            let read = xaero_cache_reader(game);
            match runtime::initialize(directory).maps.import_xaero(
                state.session, menu.dimension, source, read,
            ) {
                Ok(reply) => state.cache_import = Some(reply),
                Err(error) => state.notice = Some((error, Instant::now())),
            }
        }
        MenuAction::ImportXaero => {
            if state.waypoint_import.is_some() {
                return;
            }
            let directory = game.native_data_dir(runtime::MOD_ID);
            let path = directory.join(format!("xaero-{}.txt", menu.dimension.name()));
            let controller = runtime::initialize(directory);
            let world = state.world.clone();
            let (send, reply) = std::sync::mpsc::channel();
            let worker = std::thread::Builder::new().name("map-waypoint-import".into())
                .spawn(move || {
                    let result = controller.import_xaero_waypoints(&path, &world, menu.dimension);
                    let _ = send.send(result);
                });
            match worker {
                Ok(_) => state.waypoint_import = Some(reply),
                Err(error) => state.notice = Some((error.to_string(), Instant::now())),
            }
        }
        MenuAction::ExportPng => export_map_png(game,state,menu.dimension),
        MenuAction::OpenWaypoints => {
            state.mode = Mode::Waypoints;
            state.waypoint_scroll = 0;
        }
        MenuAction::OpenSettings => open_settings(state, Some(Mode::Map), None),
        MenuAction::Information => {
            state.info = Some(menu);
            state.info_scroll = 0;
        }
        MenuAction::Copy => {
            state.pending_copy = Some(format!("{} {} {}", menu.block[0], y, menu.block[1]));
            state.notice = Some((
                tr(
                    "Coordinates copied",
                    "Координаты скопированы",
                    "Coordonnées copiées",
                    "Coordenadas copiadas",
                    "Koordinaten kopiert",
                )
                .into(),
                Instant::now(),
            ));
        }
        MenuAction::Waypoint | MenuAction::TemporaryWaypoint | MenuAction::DestinationWaypoint => {
            let target = Target {
                dimension: menu.dimension,
                x: menu.block[0],
                y,
                z: menu.block[1],
            };
            let kind = match action {
                MenuAction::TemporaryWaypoint => MarkerMode::Temporary,
                MenuAction::DestinationWaypoint => MarkerMode::Destination,
                _ => MarkerMode::Saved,
            };
            state.name_form = Some(NameForm::new(state, kind, Mode::Map).at(target));
            state.mode = Mode::Name;
            runtime::set_text_input(true);
        }
        MenuAction::CopyDestination => {
            if let Some(PortalTarget::Exact { at, .. }) = target {
                state.pending_copy = Some(format!("{} {} {}", at[0], at[1], at[2]));
                state.notice = Some((
                    tr(
                        "Destination coordinates copied",
                        "Координаты назначения скопированы",
                        "Coordonnées de destination copiées",
                        "Coordenadas del destino copiadas",
                        "Zielkoordinaten kopiert",
                    )
                    .into(),
                    Instant::now(),
                ));
            }
        }
        MenuAction::RefreshClaims => {
            if official_smp(game) {
                state.pending_command = Some("claims".into());
            }
        }
        MenuAction::TeleportPlayer => {
            if teleport_allowed(game) && let Some(name) = menu.radar_player.and_then(|id| game.remote_player_name(id))
                && (1..=16).contains(&name.len())
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                use ::map::xaero_settings;
                let directory = game.native_data_dir(runtime::MOD_ID);
                let profile = xaero_settings::active_profile(&directory);
                let commands = xaero_settings::teleport_commands(&xaero_settings::commands_directory(&directory, profile.as_deref())).unwrap_or_default();
                let template = commands.get("tp_player").map(String::as_str).unwrap_or("tp @s {name}");
                state.pending_command = Some(template.replace("{name}", name));
                state.close = true;
            }
        }
        MenuAction::Teleport => {
            if !teleport_allowed(game) {
                return;
            }
            let point = menu.waypoint.and_then(|spot| state.waypoints.iter().find(|point|point.key()==spot.key));
            let unknown = point.is_some_and(|point|point.options.unspecified_height)
                || (point.is_none() && menu.height.is_none());
            let at = point.map_or([menu.block[0], y.saturating_add(1), menu.block[1]], |point| {
                [point.x, if point.options.unspecified_height {y} else {point.y}, point.z]
            });
            state.pending_command = configured_teleport(game,menu.dimension,Dimension::from_id(player.dimension),point,at,unknown);
            state.close = state.pending_command.is_some();
        }
        MenuAction::TeleportDestination => {
            if !teleport_allowed(game) {
                return;
            }
            if let Some(PortalTarget::Exact { dimension, at }) = target {
                state.pending_command = Some(teleport_command(
                    dimension,
                    at,
                    Dimension::from_id(player.dimension),
                ));
                state.close = true;
            }
        }
        MenuAction::EditWaypoint => {
            if let Some(spot) = menu.waypoint {
                edit_waypoint(state,spot.key);
            }
        }
        MenuAction::DeleteWaypoint => {
            if let Some(spot) = menu.waypoint {
                runtime::initialize(game.native_data_dir(runtime::MOD_ID)).delete_waypoint(&state.world, spot.key);
                state.notice = Some((
                    tr(
                        "Waypoint deleted",
                        "Вейпоинт удалён",
                        "Repère supprimé",
                        "Marcador eliminado",
                        "Wegpunkt gelöscht",
                    )
                    .into(),
                    Instant::now(),
                ));
            }
        }
        MenuAction::Toggle3D => {
            if let Some(spot) = menu.waypoint {
                let show_3d = waypoint_show_3d(state, spot);
                runtime::initialize(game.native_data_dir(runtime::MOD_ID))
                    .change_waypoint(&state.world, spot.key, |point| point.show_3d = !show_3d);
            }
        }
    }
}

fn xaero_cache_reader(game: &Game<'_>) -> maps::ReadXaeroRegion {
    use ::map::xaero_cache as xaero;
    use horizon_core::nbt::{self, Tag};
    use std::io::{self, Read};
    let resources = Arc::clone(&game.res);
    let registry = game.pack.assets.registry;
    let biome_palette = game.world_biomes.as_ref().unwrap_or_else(|| game.res.biomes()).clone();
    Box::new(move |path, coordinate, layer| {
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        let textures = matches!(ext, "xwmc" | "outdated");
        let limit = if textures { xaero::MAX_REGION_BYTES } else { xaero::MAX_BLOCK_REGION_BYTES };
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > limit {
            return Err(io::Error::other("Xaero archive is too large"));
        }
        let mut data = Vec::new();
        if ext == "xaero" {
            file.take(limit + 1).read_to_end(&mut data)?;
        } else {
            let mut archive = zip::ZipArchive::new(file)?;
            if archive.len() != 1 {
                return Err(io::Error::other("Expected one Xaero region entry"));
            }
            let entry = if textures { "cache.xaero" } else { "region.xaero" };
            archive.by_name(entry)?.take(limit + 1).read_to_end(&mut data)?;
        }
        if data.len() as u64 > limit {
            return Err(io::Error::other("Xaero region exceeds the decompressed size limit"));
        }
        if textures {
            return xaero::region(&data, coordinate, layer);
        }
        let decode = |bytes: &[u8]| -> io::Result<_> {
            let limits = nbt::Limits { max_bytes: 65_536, max_nodes: 512, max_depth: 8 };
            let bytes = &bytes[..bytes.len().min(limits.max_bytes)];
            let (root, consumed) = nbt::read_named(bytes, limits).map_err(|error| io::Error::other(error.to_string()))?;
            let mut tag = root.tag;
            if tag.get("id").is_none() {
                let Some(name) = tag.get("Name").cloned() else {
                    return Err(io::Error::other("Xaero block state has no name"));
                };
                let mut fields = vec![("id", name)];
                if let Some(properties) = tag.get("Properties") {
                    fields.push(("properties", properties.clone()));
                }
                tag = Tag::compound(fields);
            }
            let state = horizon_world::chunk::BlockState::from_nbt(&tag)?;
            let id = registry.resolve(&state).ok();
            let book = resources.facts();
            let rgb = id.and_then(|id| book.states.get(id.0 as usize))
                .and_then(|facts| horizon_client::blocks::MAP_COLORS.get(facts.map_color as usize))
                .copied()
                .unwrap_or(0x525968);
            let air = id.is_some_and(|id| book.is_air(id));
            Ok((xaero::SavedBlock { name: Arc::from(state.name), color: rgb, air }, consumed))
        };
        xaero::block_region(&data, coordinate, layer, decode, |block, biome, [x,z]| {
            let id = biome_palette.id(biome.unwrap_or("minecraft:plains"));
            let bi = biome_palette.get(id);
            if block.name.as_ref() == "minecraft:water" {
                return bi.water;
            }
            plant_color(&block.name).unwrap_or_else(|| {
                maps::block_map_color(&block.name, block.color,
                    Some(biome_palette.grass(id, f64::from(x), f64::from(z))), Some(bi.foliage))
            })
        })
    })
}

fn write_map_png(path: &std::path::Path, width: u32, height: u32, pixels: &[u8]) -> std::io::Result<()> {
    use std::{fs, io};
    let temporary = path.with_extension("png.partial");
    let file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = (|| -> io::Result<()> {
        let mut encoder = png::Encoder::new(io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        writer.write_image_data(pixels).map_err(io::Error::other)?;
        writer.finish().map_err(io::Error::other)?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn poll_map_export(state: &mut State) {
    use std::sync::mpsc::TryRecvError;
    let Some(export) = &state.export else { return };
    let message = match export.try_recv() {
        Ok(Ok(file)) => localized_format!(
            "Saved to exports/{}", "Сохранено в exports/{}", "Enregistré dans exports/{}",
            "Guardado en exports/{}", "Gespeichert in exports/{}", file
        ),
        Ok(Err(error)) => error,
        Err(TryRecvError::Disconnected) => tr(
            "Export was interrupted", "Экспорт прерван", "Export interrompu",
            "Exportación interrumpida", "Export unterbrochen"
        ).to_owned(),
        Err(TryRecvError::Empty) => return,
    };
    state.export = None;
    state.notice = Some((message, Instant::now()));
}

fn poll_waypoint_import(state: &mut State) {
    use std::sync::mpsc::TryRecvError;
    let Some(import) = &state.waypoint_import else { return };
    let message = match import.try_recv() {
        Ok(Ok(count)) => localized_format!(
            "Imported waypoints: {}", "Импортировано меток: {}", "Repères importés : {}",
            "Marcadores importados: {}", "Importierte Wegpunkte: {}", count
        ),
        Ok(Err(error)) => error,
        Err(TryRecvError::Disconnected) => tr(
            "Import was interrupted", "Импорт прерван", "Importation interrompue",
            "Importación interrumpida", "Import unterbrochen"
        ).to_owned(),
        Err(TryRecvError::Empty) => return,
    };
    state.waypoint_import = None;
    state.notice = Some((message, Instant::now()));
}

fn poll_cache_import(state: &mut State) {
    use std::sync::mpsc::TryRecvError;
    let Some(import) = &state.cache_import else { return };
    let message = match import.try_recv() {
        Ok(Ok(count)) => localized_format!(
            "Imported chunks: {}", "Импортировано чанков: {}", "Chunks importés : {}",
            "Chunks importados: {}", "Importierte Chunks: {}", count
        ),
        Ok(Err(error)) => error,
        Err(TryRecvError::Disconnected) => tr(
            "Import was interrupted", "Импорт прерван", "Import interrompu",
            "Importación interrumpida", "Import unterbrochen"
        ).to_owned(),
        Err(TryRecvError::Empty) => return,
    };
    state.cache_import = None;
    state.notice = Some((message, Instant::now()));
}

/// One row of the right-click menu: a grey fact, or an action with an optional key name.
struct MenuRow {
    label: String,
    action: Option<MenuAction>,
    key: Option<String>,
}

fn menu_rows(game: &Game<'_>, state: &State, menu: Menu, player: &crate::game::native_hooks::NativePlayer) -> Vec<MenuRow> {
    let fact = |label: String| MenuRow { label, action: None, key: None };
    let act = |key: &str, action: MenuAction| MenuRow { label: ui(key), action: Some(action), key: None };
    let (chunk, region) = maps::chunk_region(menu.block);
    let mut rows = vec![
        fact(format!("C: ({};{}):({};{})", chunk[0], chunk[1], region[0], region[1])),
        fact(match menu.height {
            Some(y) => format!("X: {}, Y: {}{}, Z: {}", menu.block[0], if menu.height_approximate { "≈" } else { "" }, y, menu.block[1]),
            None => format!("X: {}, Z: {}", menu.block[0], menu.block[1]),
        }),
    ];
    let target = portal_target(state, menu, player.y.floor() as i32);
    if let Some(kind) = menu.portal {
        rows.push(fact(format!("{} → {}", portal_label(kind),
            target.map_or_else(|| ui("unknown_destination"), portal_target_text))));
    }
    rows.push(act("create_waypoint", MenuAction::Waypoint));
    rows.push(MenuRow {
        key: key_name(game, "instant_waypoint", "key.keyboard.unknown"),
        ..act("temporary_waypoint", MenuAction::TemporaryWaypoint)
    });
    rows.push(act("destination", MenuAction::DestinationWaypoint));
    if menu.waypoint.is_some() {
        let show_3d = menu.waypoint.map_or(true, |spot| waypoint_show_3d(state, spot));
        rows.push(act("edit_waypoint", MenuAction::EditWaypoint));
        rows.push(act("delete_waypoint", MenuAction::DeleteWaypoint));
        rows.push(act(if show_3d { "hide_in_world" } else { "show_in_world" }, MenuAction::Toggle3D));
    }
    if teleport_allowed(game) {
        if menu.radar_player.is_some() {
            rows.push(act("teleport_to_player", MenuAction::TeleportPlayer));
        }
        if menu.portal.is_some() {
            if matches!(target, Some(PortalTarget::Exact { .. })) {
                rows.push(act("teleport_to_portal", MenuAction::TeleportDestination));
            }
        } else {
            rows.push(act("teleport_here", MenuAction::Teleport));
        }
    }
    if menu.portal.is_some() && matches!(target, Some(PortalTarget::Exact { .. })) {
        rows.push(act("copy_destination", MenuAction::CopyDestination));
    }
    rows.push(act("copy_location", MenuAction::Copy));
    rows.push(act("chunk_info", MenuAction::Information));
    if official_smp(game) {
        rows.push(act("refresh_claims", MenuAction::RefreshClaims));
    }
    rows.push(MenuRow {
        key: key_name(game, runtime::WAYPOINTS_KEY, runtime::WAYPOINTS_DEFAULT),
        ..act("open_waypoints", MenuAction::OpenWaypoints)
    });
    rows.push(act("export_png", MenuAction::ExportPng));
    rows.push(MenuRow {
        key: key_name(game, "map_settings", "key.keyboard.y"),
        ..act("open_settings", MenuAction::OpenSettings)
    });
    if let Some(distance) = menu_distance(game, state, menu, player) {
        rows.push(fact(distance));
    }
    rows
}

/// The distance from the player to the clicked point, when both are in one dimension.
fn menu_distance(game: &Game<'_>, state: &State, menu: Menu, player: &crate::game::native_hooks::NativePlayer) -> Option<String> {
    if Dimension::from_id(player.dimension) != menu.dimension {
        return None;
    }
    let dx = f64::from(menu.block[0]) + 0.5 - player.x;
    let dz = f64::from(menu.block[1]) + 0.5 - player.z;
    let y = menu.waypoint.and_then(|spot| state.waypoints.iter().find(|point| point.key() == spot.key))
        .filter(|point| !point.options.unspecified_height).map(|point| f64::from(point.y) - player.y)
        .or_else(|| menu.height.map(|y| f64::from(y) - player.y));
    let meters = dx.hypot(dz).hypot(y.unwrap_or(0.));
    let limit = game.map_number("world_map_km_at").unwrap_or(-1.);
    let precision = usize::from(game.map_option("world_map_dist_precision").unwrap_or(1));
    Some(if limit >= 0. && meters >= limit {
        format!("{km:.precision$}km", km = meters / 1000.)
    } else {
        format!("{meters:.precision$}m")
    })
}

fn menu_layout(size: [f32; 2], menu: Menu, rows: usize) -> ([f32; 4], [f32; 4], Vec<[f32; 4]>) {
    look::context_menu(menu.at, rows, size)
}

fn draw_menu(game: &Game<'_>, batch: &mut HudBatch, state: &State) {
    let Some(menu) = state.menu else { return };
    let Some(player) = game.native_player() else { return };
    let rows = menu_rows(game, state, menu, &player);
    let (outer, title, rects) = menu_layout([batch.width, batch.height], menu, rows.len());
    let mouse = game.cursor();
    let line = look::MENU_LINE;
    for edge in [
        [outer[0], outer[1], outer[2], line],
        [outer[0], outer[1] + outer[3] - line, outer[2], line],
        [outer[0], outer[1] + line, line, outer[3] - 2. * line],
        [outer[0] + outer[2] - line, outer[1] + line, line, outer[3] - 2. * line],
    ] {
        game.native_fill(batch, edge, look::MENU_BORDER);
    }
    let centred = |game: &Game<'_>, batch: &mut HudBatch, rect: [f32; 4], parts: &[(String, u32)]| {
        let width: f32 = parts.iter().map(|(text, _)| game.map_text_width(text)).sum();
        let mut x = (rect[0] + (rect[2] - width) * 0.5).round();
        for (text, color) in reading_order(parts) {
            draw_text(game, batch, &text, [x, rect[1] + 1.], color, true);
            x += game.map_text_width(&text);
        }
    };
    game.native_fill(batch, title, look::MENU_TITLE);
    centred(game, batch, title, &[(ui("choose_option"), look::SCREEN_TEXT)]);
    let mut above = title;
    for (row, rect) in rows.iter().zip(rects) {
        game.native_fill(batch, [rect[0], above[1] + above[3], rect[2], line], look::MENU_SEPARATOR);
        let hovered = row.action.is_some() && look::hit(mouse, rect);
        game.native_fill(batch, rect, if hovered { look::MENU_HOVER } else { look::MENU_ROW });
        let key = row.key.as_ref().map(|key| format!(" ({key})"));
        let room = rect[2] - 6. - key.as_ref().map_or(0., |key| game.map_text_width(key));
        let mut parts = vec![(fit_text(game, &row.label, room), if row.action.is_some() { look::SCREEN_TEXT } else { look::GREY_TEXT })];
        if let Some(key) = key {
            parts.push((key, look::KEY_TEXT));
        }
        centred(game, batch, rect, &parts);
        above = rect;
    }
}

fn draw_notice(game: &Game<'_>, batch: &mut HudBatch, state: &State) {
    if let Some(error) = runtime::initialize(game.native_data_dir(runtime::MOD_ID))
        .maps
        .status()
    {
        let text = localized_format!(
            "Map cache: {}",
            "Кеш карты: {}",
            "Cache carte : {}",
            "Caché: {}",
            "Karten-Cache: {}",
            error
        );
        game.native_fill(
            batch,
            [0.0, batch.height - 20.0, batch.width, 20.0],
            0xe0402020,
        );
        game.map_text(batch, &text, [6.0, batch.height - 15.0], 0xffffd24a, false);
    }
    let Some((text, since)) = state.notice.as_ref() else {
        return;
    };
    if since.elapsed() > Duration::from_secs(2) {
        return;
    }
    let width = game.map_text_width(text);
    let x = ((batch.width - width) / 2.0).round();
    let y = batch.height - 38.0;
    game.native_fill(batch, [x - 3.0, y - 2.0, width + 6.0, 12.0], look::TOOLTIP_BOX);
    game.map_text(batch, text, [x, y], look::SCREEN_TEXT, true);
}

fn waypoint_visible_rows(height: f32) -> usize {
    look::waypoint_rows(height)
}

fn shown_waypoints(state: &State) -> impl DoubleEndedIterator<Item = (usize, &Waypoint)> {
    state.waypoints.iter().enumerate().filter(|(_, point)| {
        state.waypoint_group.as_ref().is_none_or(|group| *group == point.options.group)
    })
}

fn map_waypoint_visible(game: &Game<'_>, state: &State, point: &Waypoint) -> bool {
    !point.options.disabled && !(point.options.purpose == 2 && game.map_option("old_deathpoints") == Some(0))
        && (game.map_option("all_waypoint_groups") == Some(1) || point.options.visibility % 2 == 1
        || state.waypoint_group.as_ref().is_none_or(|group| *group == point.options.group))
}

fn group_label(group:Option<&str>)->String {
    match group {
        None=>ui("all_sets"),
        Some("")=>ui("default_set"),
        Some(name)=>name.to_owned(),
    }
}

fn waypoint_groups(game:&Game<'_>,state:&State)->Vec<Option<String>> {
    let groups=runtime::initialize(game.native_data_dir(runtime::MOD_ID)).waypoint_group_names(&state.world);
    std::iter::once(None).chain(groups.into_iter().map(Some)).collect()
}

/// A row of the set dropdown.
#[derive(Clone, PartialEq)]
enum SetRow {
    Select(Option<String>),
    Rename(String),
    DeleteEmpty(String),
}

/// The set dropdown's rows under the set bar, as many as fit above the actions.
fn set_rows(game: &Game<'_>, state: &State, bar: [f32; 4], bottom: f32) -> Vec<(SetRow, [f32; 4])> {
    let mut rows: Vec<SetRow> = waypoint_groups(game, state).into_iter().map(SetRow::Select).collect();
    if let Some(name) = state.waypoint_group.as_ref().filter(|name| !name.is_empty()) {
        rows.push(SetRow::Rename(name.clone()));
        if shown_waypoints(state).next().is_none() {
            rows.push(SetRow::DeleteEmpty(name.clone()));
        }
    }
    let fit = (((bottom - bar[1] - bar[3]) / bar[3]).floor().max(1.)) as usize;
    let start = state.waypoint_groups_scroll.min(rows.len().saturating_sub(fit));
    rows.into_iter().skip(start).take(fit).enumerate()
        .map(|(i, row)| (row, [bar[0], bar[1] + bar[3] * (i + 1) as f32, bar[2], bar[3]]))
        .collect()
}

fn set_row_label(row: &SetRow) -> String {
    match row {
        SetRow::Select(group) => group_label(group.as_deref()),
        SetRow::Rename(_) => ui("rename_set"),
        SetRow::DeleteEmpty(_) => ui("delete_empty_set"),
    }
}

fn listed_waypoints<'a>(game:&Game<'_>,state:&'a State) -> Vec<(usize,&'a Waypoint)> {
    let query=state.waypoint_search.trim().to_lowercase();
    let mut points:Vec<_>=shown_waypoints(state).filter(|(_,point)| {
        (state.waypoint_dimension==Dimension::Player||point.dimension==state.waypoint_dimension)
            && (query.is_empty()||point.name.to_lowercase().contains(&query))
    }).collect();
    match state.waypoint_sort {
        1=>points.sort_by_cached_key(|(_,point)| point.name.to_lowercase()),
        2=>if let Some(player)=game.native_player() {
            let dimension=Dimension::from_id(player.dimension);
            let distance=|point:&Waypoint| if point.dimension==dimension {
                (f64::from(point.x)+0.5-player.x).powi(2)+(f64::from(point.z)+0.5-player.z).powi(2)
            } else {f64::INFINITY};
            points.sort_by(|(_,a),(_,b)| distance(a).total_cmp(&distance(b)));
        },
        _=>if game.map_option("waypoints_new_bottom")==Some(0){points.reverse();},
    }
    points
}

/// The waypoint list's layout and the rows it shows now.
fn waypoint_screen<'a>(game: &Game<'_>, state: &'a State, size: [f32; 2]) -> (look::WaypointList, Vec<(usize, &'a Waypoint, [f32; 4])>) {
    let points = listed_waypoints(game, state);
    let count = look::waypoint_rows(size[1]);
    let start = state.waypoint_scroll.min(points.len().saturating_sub(count));
    let list = look::waypoint_list(size, points.len().saturating_sub(start).min(count), runtime::right_to_left());
    let rows = points.into_iter().skip(start).zip(list.rows.iter().copied())
        .map(|((index, point), row)| (index, point, row)).collect();
    (list, rows)
}

fn selected_waypoint(state: &State) -> Option<&Waypoint> {
    let selected = state.waypoint_selected?;
    shown_waypoints(state).find_map(|(_, point)| (point.key() == selected).then_some(point))
}

fn show_selected_waypoint(state: &mut State) -> Option<Dimension> {
    let waypoint = selected_waypoint(state)?;
    if waypoint.world != state.world {
        return None;
    }
    let spot = Spot {
        dimension: waypoint.dimension,
        x: waypoint.x,
        z: waypoint.z,
        key: waypoint.key(),
    };
    state.map_focus = Some(spot);
    state.mode = Mode::Map;
    state.menu = None;
    state.dragging = false;
    state.elapsed = 1.0;
    Some(spot.dimension)
}

// Apply after the normal dimension/mode transition so it cannot recenter on
// the player and overwrite the requested waypoint on the following frame.
fn apply_map_focus(state: &mut State, large: bool, selected: Dimension) {
    if large
        && let Some(spot) = state.map_focus.take()
        && spot.dimension == selected
    {
        state
            .camera
            .snap([f64::from(spot.x) + 0.5, f64::from(spot.z) + 0.5]);
        state.frame = None;
    }
}

fn sort_label(sort: usize) -> String {
    ui(match sort { 1 => "sort_name", 2 => "sort_distance", _ => "sort_added" })
}

fn dimension_filter_label(dimension: Dimension) -> String {
    if dimension == Dimension::Player { ui("all_dimensions") } else { dimension.label().to_owned() }
}

/// A waypoint's distance and place, for its row's tooltip.
fn waypoint_details(game: &Game<'_>, point: &Waypoint) -> Vec<Vec<(String, u32)>> {
    let mut lines = Vec::new();
    if let Some(player) = game.native_player()
        && point.dimension == Dimension::from_id(player.dimension)
    {
        let dy = if point.options.unspecified_height { 0. } else { f64::from(point.y) - player.y };
        let meters = (f64::from(point.x) + 0.5 - player.x).hypot(f64::from(point.z) + 0.5 - player.z).hypot(dy);
        lines.push(vec![(waypoint_distance_text(game, meters), look::TOOLTIP_TEXT)]);
    }
    if game.map_option("waypoint_hide_coords") != Some(1) {
        let y = if point.options.unspecified_height { "~".to_owned() } else { point.y.to_string() };
        lines.push(vec![
            (format!("{} · ", point.dimension.label()), look::GREY_TEXT),
            (format!("{}, {y}, {}", point.x, point.z), look::GREY_TEXT),
        ]);
    }
    lines
}

fn draw_waypoints_screen(game: &Game<'_>, batch: &mut HudBatch, state: &State) {
    draw_backdrop(game, batch, state);
    let size = [batch.width, batch.height];
    let mouse = game.cursor();
    let (list, rows) = waypoint_screen(game, state, size);
    let centred = |game: &Game<'_>, batch: &mut HudBatch, at: [f32; 2], text: &str, color: u32| {
        let width = game.map_text_width(text);
        game.map_text(batch, text, [(at[0] - width * 0.5).round(), at[1]], color, true);
    };
    centred(game, batch, list.labels[0], &ui("dimension"), look::SCREEN_TEXT);
    centred(game, batch, list.labels[1], &ui("sort"), look::SCREEN_TEXT);
    draw_bar(game, batch, list.bars[0], &dimension_filter_label(state.waypoint_dimension), look::SCREEN_TEXT, look::hit(mouse, list.bars[0]));
    draw_bar(game, batch, list.bars[1], &sort_label(state.waypoint_sort), look::SCREEN_TEXT, look::hit(mouse, list.bars[1]));
    button(game, batch, list.options, &ui("options"), mouse, true);
    draw_bar(game, batch, list.set, &group_label(state.waypoint_group.as_deref()), look::SCREEN_TEXT, look::hit(mouse, list.set));
    button(game, batch, list.new_set, &ui("new_set"), mouse, true);
    game.native_fill(batch, list.band, look::LIST_BAND);
    game.native_fill(batch, [list.band[0], list.band[1], list.band[2], 1.], look::LIST_EDGE);
    game.native_fill(batch, [list.band[0], list.band[1] + list.band[3] - 1., list.band[2], 1.], look::LIST_EDGE);
    draw_field(game, batch, list.filter, &state.waypoint_search, &ui("filter"), state.waypoint_search_focus);
    let rtl = runtime::right_to_left();
    let mut tooltip = None;
    for (index, waypoint, row) in &rows {
        if state.waypoint_selected == Some(waypoint.key()) {
            game.native_fill(batch, *row, look::SELECTED_BORDER);
            game.native_fill(batch, [row[0] + 1., row[1] + 1., row[2] - 2., row[3] - 2.], look::SELECTED_ROW);
        }
        let (square, centre) = look::waypoint_row_parts(*row, rtl);
        draw_initial(game, batch, square, waypoint, &waypoint_text(state, *index, waypoint));
        let name = fit_text(game, &waypoint.name, row[2] - 2. * (square[2] + 10.));
        let width = game.map_text_width(&name);
        game.map_text(batch, &name, [(centre[0] - width * 0.5).round(), centre[1].round()],
            if waypoint.options.disabled { look::DISABLED_TEXT } else { look::SCREEN_TEXT }, true);
        if look::hit(mouse, *row) && !state.waypoint_groups_open {
            tooltip = Some((waypoint_details(game, waypoint), *row));
        }
    }
    if rows.is_empty() {
        let text = ui(if state.waypoints.is_empty() { "no_waypoints" } else { "no_matching_waypoints" });
        centred(game, batch, [batch.width * 0.5, list.band[1] + 8.], &text, look::GREY_TEXT);
    }
    let selected = selected_waypoint(state);
    let [edit, teleport, share, enable, delete] = list.actions;
    button(game, batch, edit, &ui(if selected.is_some() { "edit" } else { "add" }), mouse, true);
    button(game, batch, teleport, &ui("teleport"), mouse, selected.is_some() && teleport_allowed(game));
    button(game, batch, share, &ui("share"), mouse, selected.is_some());
    button(game, batch, enable, &ui(if selected.is_some_and(|point| point.options.disabled) { "enable" } else { "disable" }), mouse, selected.is_some());
    button(game, batch, delete, &ui("delete"), mouse, selected.is_some());
    button(game, batch, list.done, &ui("done"), mouse, true);
    if look::hit(mouse, share) {
        tooltip = Some((vec![vec![(ui("tip_share"), look::TOOLTIP_TEXT)]], share));
    }
    if state.waypoint_groups_open {
        for (row, rect) in set_rows(game, state, list.set, list.actions[0][1]) {
            let chosen = matches!(&row, SetRow::Select(group) if *group == state.waypoint_group);
            let color = if matches!(row, SetRow::Select(_)) { look::SCREEN_TEXT } else { look::GREY_TEXT };
            let label = set_row_label(&row);
            draw_bar(game, batch, rect, &if chosen { format!("> {label} <") } else { label }, color, look::hit(mouse, rect));
        }
        tooltip = None;
    }
    draw_notice(game, batch, state);
    if let Some((lines, hovered)) = tooltip {
        draw_tooltip_lines(game, batch, &lines, mouse, hovered);
    }
}

/// The create / edit form's fields that take typing, in Tab order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    X,
    Y,
    Z,
    Yaw,
    Initials,
}

const FIELDS: [Field; 6] = [Field::Name, Field::X, Field::Y, Field::Z, Field::Yaw, Field::Initials];

impl NameForm {
    fn new(state: &State, kind: MarkerMode, background: Mode) -> Self {
        Self {
            text: default_waypoint_name(state),
            color: default_waypoint_color(state),
            target: None,
            rename: None,
            group: state.waypoint_group.clone().unwrap_or_default(),
            focus: Field::Name,
            kind,
            group_edit: None,
            background,
            dimension: state.selected,
            coords: [String::new(), String::new(), String::new(), String::new()],
            initials: String::new(),
            show_3d: true,
            disabled: false,
        }
    }

    fn at(mut self, target: Target) -> Self {
        self.dimension = target.dimension;
        self.coords = [target.x.to_string(), target.y.to_string(), target.z.to_string(), String::new()];
        self.target = Some(target);
        self
    }

    fn field(&mut self, field: Field) -> &mut String {
        match field {
            Field::Name => &mut self.text,
            Field::X => &mut self.coords[0],
            Field::Y => &mut self.coords[1],
            Field::Z => &mut self.coords[2],
            Field::Yaw => &mut self.coords[3],
            Field::Initials => &mut self.initials,
        }
    }

    /// The typed position, when every coordinate reads as a number.
    fn position(&self) -> Option<[i32; 3]> {
        let mut at = [0; 3];
        for (value, text) in at.iter_mut().zip(&self.coords) {
            *value = text.trim().parse().ok()?;
        }
        Some(at)
    }

    fn yaw(&self) -> Option<i32> {
        self.coords[3].trim().parse().ok()
    }

    fn valid(&self) -> bool {
        !self.text.trim().is_empty() && (self.group_edit.is_some() || self.position().is_some())
    }

    fn accepts(&self, field: Field, c: char) -> bool {
        let text = match field {
            Field::Name => &self.text,
            Field::X => &self.coords[0],
            Field::Y => &self.coords[1],
            Field::Z => &self.coords[2],
            Field::Yaw => &self.coords[3],
            Field::Initials => &self.initials,
        };
        match field {
            Field::Name => if self.group_edit.is_some() { text.len() + c.len_utf8() <= 128 } else { text.chars().count() < 32 },
            Field::Initials => text.chars().count() < 2,
            _ => text.len() < 9 && (c.is_ascii_digit() || (c == '-' && text.is_empty())),
        }
    }
}

/// The form's field rects by field.
fn form_field(form: &look::WaypointForm, field: Field) -> [f32; 4] {
    match field {
        Field::Name => form.name,
        Field::X => form.coords[0],
        Field::Y => form.coords[1],
        Field::Z => form.coords[2],
        Field::Yaw => form.coords[3],
        Field::Initials => form.initials,
    }
}

fn color_name(color: u32) -> String {
    runtime::WAYPOINT_COLORS.iter().position(|c| *c == color)
        .map_or_else(|| format!("#{color:06X}"), |i| ui(&format!("color_{i}")))
}

fn draw_name_form(game: &Game<'_>, batch: &mut HudBatch, state: &State, form: &NameForm) {
    draw_backdrop(game, batch, state);
    let size = [batch.width, batch.height];
    let mouse = game.cursor();
    let layout = look::waypoint_form(size, runtime::right_to_left());
    let title = if form.group_edit.is_some() {
        ui(if matches!(form.group_edit, Some(GroupEdit::Create)) { "new_set" } else { "rename_set" })
    } else if form.rename.is_some() { ui("edit_waypoint") } else { ui("new_waypoint") };
    let width = game.map_text_width(&title);
    game.map_text(batch, &title, [(layout.title[0] - width * 0.5).round(), layout.title[1]], look::SCREEN_TEXT, true);
    draw_field(game, batch, layout.name, &form.text, &ui("name"), form.focus == Field::Name);
    if form.group_edit.is_none() {
        draw_bar(game, batch, layout.world, &format!("{} ({})", state.world.rsplit(['/', '\\', ':']).next().unwrap_or(&state.world), ui("auto")),
            look::GREY_TEXT, false);
        draw_bar(game, batch, layout.dimension, form.dimension.label(), look::SCREEN_TEXT, look::hit(mouse, layout.dimension));
        draw_bar(game, batch, layout.set, &group_label(Some(&form.group)), look::SCREEN_TEXT, look::hit(mouse, layout.set));
        draw_bar(game, batch, layout.color, &color_name(form.color), waypoint_argb(form.color), look::hit(mouse, layout.color));
        for (field, hint) in [(Field::X, "x"), (Field::Y, "y"), (Field::Z, "z"), (Field::Yaw, "yaw")] {
            let text = match field { Field::X => &form.coords[0], Field::Y => &form.coords[1], Field::Z => &form.coords[2], _ => &form.coords[3] };
            draw_field(game, batch, form_field(&layout, field), text, &ui(hint), form.focus == field);
        }
        let auto = first_letter(&form.text).to_string();
        draw_field(game, batch, layout.initials, &form.initials, &auto, form.focus == Field::Initials);
        button(game, batch, layout.world_3d, &format!("{}: {}", ui("in_world"), ui(if form.show_3d { "on" } else { "off" })), mouse, true);
        button(game, batch, layout.enabled, &ui(if form.disabled { "disabled" } else { "enabled" }), mouse, true);
    }
    button(game, batch, layout.confirm, &ui("confirm"), mouse, form.valid());
    button(game, batch, layout.cancel, &ui("cancel"), mouse, true);
}

// Hit-test the full short label of the waypoint under the map pointer.
fn waypoint_at_pointer<'a>(
    game: &Game<'_>,
    state: &'a State,
    view: &View,
    rect: [f32; 4],
    mouse: [f32; 2],
) -> Option<&'a Waypoint> {
    let scale = game.map_number("world_waypoint_symbol_scale").unwrap_or(1.) as f32;
    state
        .waypoints
        .iter()
        .enumerate()
        .rev()
        .find(|(index, waypoint)| {
            if waypoint.dimension != view.dimension || !map_waypoint_visible(game, state, waypoint) {
                return false;
            }
            let at = world_screen(view, rect, [f64::from(waypoint.x) + 0.5, f64::from(waypoint.z) + 0.5]);
            let width = game.map_text_width(&waypoint_text(state, *index, waypoint)) * look::PENNANT_FONT;
            let scale = scale.max(0.25) * ((width + 4.) / look::PENNANT[0]).max(1.);
            let (body, _) = look::pennant(at, scale);
            look::hit(mouse, [body[0], body[1], body[2], at[1] - body[1] + 1.])
        })
        .map(|(_, point)| point)
}

fn portal_near(state: &State, dimension: Dimension, block: [i32; 2]) -> Option<(Portal, [i32; 3])> {
    state
        .portals
        .iter()
        .filter(|(at_dimension, at, _)| {
            *at_dimension == dimension
                && (at[0] - block[0]).abs() <= 2
                && (at[2] - block[1]).abs() <= 2
        })
        .min_by_key(|(_, at, _)| {
            let dx = at[0] - block[0];
            let dz = at[2] - block[1];
            dx * dx + dz * dz
        })
        .map(|(_, at, kind)| (*kind, *at))
}

fn world_screen(view: &View, rect: [f32; 4], at: [f64; 2]) -> [f32; 2] {
    let pixel = view.screen_at(at);
    [
        rect[0] + pixel[0] as f32 * rect[2] / view.width as f32,
        rect[1] + pixel[1] as f32 * rect[3] / view.height as f32,
    ]
}

type ScreenSegment = ([f32; 2], [f32; 2]);

fn draw_segments(
    game: &Game<'_>,
    batch: &mut HudBatch,
    rect: [f32; 4],
    segments: &[ScreenSegment],
    color: u32,
) {
    for &(from, to) in segments {
        draw_line(game, batch, from, to, rect, color);
    }
}

fn world_box_segments(view: &View, rect: [f32; 4], bounds: [f64; 4]) -> Vec<ScreenSegment> {
    let [x0, z0, x1, z1] = bounds;
    let corners = [[x0, z0], [x1, z0], [x1, z1], [x0, z1]];
    let points = corners.map(|corner| world_screen(view, rect, corner));
    (0..4)
        .map(|index| (points[index], points[(index + 1) % 4]))
        .collect()
}

fn draw_world_box(
    game: &Game<'_>,
    batch: &mut HudBatch,
    view: &View,
    rect: [f32; 4],
    bounds: [f64; 4],
    color: u32,
) {
    draw_segments(
        game,
        batch,
        rect,
        &world_box_segments(view, rect, bounds),
        color,
    );
}

fn draw_claims(game: &Game<'_>, batch: &mut HudBatch, state: &State, view: &View, rect: [f32; 4]) {
    if game.map_option(if view.large { "world_claims" } else { "minimap_claims" }) == Some(0) {
        return;
    }
    let fill = game.map_option(if view.large { "world_claim_fill" } else { "minimap_claim_fill" }).unwrap_or(46);
    let border = game.map_option(if view.large { "world_claim_border" } else { "minimap_claim_border" }).unwrap_or(80);
    let opacity = |percent| (u32::from(percent) * 255 / 100) << 24;
    let region = view.atlas_region();
    let chunk_at = |x: i64| x.div_euclid(16) as i32;
    let min = [chunk_at(region[0]), chunk_at(region[1])];
    let max = [chunk_at(region[0] + region[2]), chunk_at(region[1] + region[3])];
    let native = state.claims.iter().filter_map(|(&chunk, owner)| {
        (chunk.0 == view.dimension).then_some((chunk, owner.is_none()))
    });
    let opac = state.opac.chunks(view.dimension, min, max).filter_map(|chunk| {
        let (owner, _) = state.opac.owner(chunk)?;
        Some((chunk, owner == game.own_uuid()))
    });
    for ((_, x, z), own) in native.chain(opac) {
        let [x, z] = [i64::from(x) * 16, i64::from(z) * 16];
        if x + 16 < region[0] || z + 16 < region[1] || x > region[0] + region[2] || z > region[1] + region[3] {
            continue;
        }
        let points = maps::terrain_polygon(view, [x, z, 16, 16], [0.; 4]);
        let points: Vec<_> = points.into_iter().map(|p| [rect[0]+(p[0] as f32+0.5)*rect[2], rect[1]+(p[1] as f32+0.5)*rect[3]]).collect();
        if points.len() < 3 { continue; }
        let color = if own { 0x5ddd91 } else { 0xffac69 };
        for i in 1..points.len()-1 {
            game.native_quad(batch, [points[0], points[i], points[i+1], points[i+1]], opacity(fill) | color);
        }
        for i in 0..points.len() {
            draw_line(game, batch, points[i], points[(i+1)%points.len()], rect, opacity(border) | color);
        }
    }
}

fn draw_waystones(game: &Game<'_>, batch: &mut HudBatch, view: &View, rect: [f32; 4]) {
    if game.map_option(if view.large { "world_map_waypoints" } else { "minimap_waypoints" }) == Some(0) {
        return;
    }
    let discovered = game.map_option("waystones_discovered") != Some(0);
    let other = game.map_option("waystones_other") != Some(0);
    for stone in WAYSTONES.lock().unwrap_or_else(|error| error.into_inner()).values() {
        if stone.dimension != view.dimension || stone.hidden || !(if stone.discovered { discovered } else { other }) {
            continue;
        }
        let [x,y] = world_screen(view, rect, [f64::from(stone.at[0]) + 0.5, f64::from(stone.at[2]) + 0.5]);
        if !map_overlay_inside([x,y], rect, view.round) { continue; }
        game.native_quad(batch, [[x,y-5.],[x+4.,y],[x,y+5.],[x-4.,y]], 0xffc9a7ed);
        if view.large && !stone.name.is_empty() && map_overlay_inside([x+5.+game.map_text_width(&stone.name),y], rect, false) {
            game.map_text(batch, &stone.name, [x+5.,y-4.], 0xffead9ff, true);
        }
    }
}

fn circle_segments(
    view: &View,
    rect: [f32; 4],
    center: [f64; 2],
    radius: f64,
) -> Vec<ScreenSegment> {
    if radius <= 0.0 {
        return Vec::new();
    }
    let count = 96;
    let mut segments = Vec::with_capacity(count);
    let mut previous = world_screen(view, rect, [center[0] + radius, center[1]]);
    for index in 1..=count {
        let angle = std::f64::consts::TAU * index as f64 / count as f64;
        let point = world_screen(
            view,
            rect,
            [
                center[0] + angle.cos() * radius,
                center[1] + angle.sin() * radius,
            ],
        );
        segments.push((previous, point));
        previous = point;
    }
    segments
}

// The outer edges of the chunks the game actually draws at this radius,
// including the slack `in_view_distance` allows on each axis.
fn chunk_border_segments(
    view: &View,
    rect: [f32; 4],
    center: (i32, i32),
    radius: i32,
) -> Vec<ScreenSegment> {
    if radius < 1 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for dz in -radius..=radius {
        let mut min_dx = None;
        let mut max_dx = None;
        for dx in -radius..=radius {
            if horizon_client::streaming::in_view_distance((0, 0), radius, dx, dz) {
                min_dx = Some(min_dx.map_or(dx, |value: i32| value.min(dx)));
                max_dx = Some(max_dx.map_or(dx, |value: i32| value.max(dx)));
            }
        }
        if let (Some(min_dx), Some(max_dx)) = (min_dx, max_dx) {
            rows.push((dz, min_dx, max_dx));
        }
    }
    let Some(first) = rows.first().copied() else {
        return Vec::new();
    };
    let last = *rows.last().unwrap();
    let world = |dx: i32, dz: i32| {
        [
            f64::from(center.0 + dx) * 16.0,
            f64::from(center.1 + dz) * 16.0,
        ]
    };
    let mut segments = Vec::new();
    segments.push((
        world_screen(view, rect, world(first.1, first.0)),
        world_screen(view, rect, world(first.2 + 1, first.0)),
    ));
    segments.push((
        world_screen(view, rect, world(last.2 + 1, last.0 + 1)),
        world_screen(view, rect, world(last.1, last.0 + 1)),
    ));
    for (index, &(dz, min_dx, max_dx)) in rows.iter().enumerate() {
        let left = world(min_dx, dz);
        let right = world(max_dx + 1, dz);
        segments.push((
            world_screen(view, rect, left),
            world_screen(view, rect, [left[0], left[1] + 16.0]),
        ));
        segments.push((
            world_screen(view, rect, right),
            world_screen(view, rect, [right[0], right[1] + 16.0]),
        ));
        if let Some(&(_, next_min, next_max)) = rows.get(index + 1) {
            if next_min != min_dx {
                segments.push((
                    world_screen(view, rect, [left[0], left[1] + 16.0]),
                    world_screen(view, rect, [world(next_min, dz + 1)[0], left[1] + 16.0]),
                ));
            }
            if next_max != max_dx {
                segments.push((
                    world_screen(view, rect, [right[0], right[1] + 16.0]),
                    world_screen(
                        view,
                        rect,
                        [world(next_max + 1, dz + 1)[0], right[1] + 16.0],
                    ),
                ));
            }
        }
    }
    segments
}

fn segment_distance(point: [f32; 2], from: [f32; 2], to: [f32; 2]) -> f32 {
    let vx = to[0] - from[0];
    let vy = to[1] - from[1];
    let wx = point[0] - from[0];
    let wy = point[1] - from[1];
    let length = vx * vx + vy * vy;
    let t = if length > 0.0 {
        ((wx * vx + wy * vy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dx = point[0] - (from[0] + vx * t);
    let dy = point[1] - (from[1] + vy * t);
    (dx * dx + dy * dy).sqrt()
}

fn segments_distance(point: [f32; 2], segments: &[ScreenSegment]) -> f32 {
    segments
        .iter()
        .map(|&(from, to)| segment_distance(point, from, to))
        .fold(f32::INFINITY, f32::min)
}

fn draw_border_hint(
    game: &Game<'_>,
    batch: &mut HudBatch,
    text: &str,
    mouse: [f32; 2],
    rect: [f32; 4],
) {
    if !inside(mouse, rect) {
        return;
    }
    let width = game.map_text_width(text);
    let x = (mouse[0] + 12.0)
        .min(rect[0] + rect[2] - width - 6.0)
        .max(rect[0] + 4.0);
    let y = (mouse[1] - 16.0).max(rect[1] + 4.0);
    game.native_fill(batch, [x - 4.0, y - 3.0, width + 8.0, 13.0], 0xe010151d);
    game.map_text(batch, text, [x, y], 0xffff9a9a, true);
}

fn draw_border(
    game: &Game<'_>,
    batch: &mut HudBatch,
    view: &View,
    rect: [f32; 4],
    mouse: Option<[f32; 2]>,
) {
    let border_color = 0xffe04040;
    let mut labels = Vec::<(String, Vec<ScreenSegment>)>::new();
    if matches!(view.dimension, Dimension::Overworld | Dimension::Nether) {
        let [mut min_x, mut min_z, mut max_x, mut max_z] = game.border.bounds();
        if view.dimension == Dimension::Nether {
            min_x /= 8.0;
            min_z /= 8.0;
            max_x /= 8.0;
            max_z /= 8.0;
        }
        let segments = world_box_segments(view, rect, [min_x, min_z, max_x, max_z]);
        draw_segments(game, batch, rect, &segments, border_color);
        if view.large {
            labels.push((
                tr(
                    "World border",
                    "Граница мира",
                    "Bordure du monde",
                    "Borde del mundo",
                    "Weltgrenze",
                )
                .to_owned(),
                segments,
            ));
        }
    }

    let Some(player) = game.native_player() else {
        return;
    };
    if view.dimension != Dimension::from_id(player.dimension) {
        return;
    }
    let center = game.streaming.center;
    let center_blocks = [
        f64::from(center.0) * 16.0 + 8.0,
        f64::from(center.1) * 16.0 + 8.0,
    ];
    // The game's view still shows the first ring outside the nominal radius
    // while chunks arrive, so draw one extra chunk to contain it.
    let chunk_radius = match &game.remote {
        Some(remote) => game.streaming.radius.min(remote.view_distance + 1),
        None => game.streaming.radius,
    }
    .max(0)
        + 1;
    let full_radius = if game.lod.is_some() {
        chunk_radius.max(game.lod_distance() as i32)
    } else {
        chunk_radius
    };
    // Xaero's "Display Server Chunk Radius": off unless asked.
    if game.map_option("map_server_radius") != Some(1) {
        return;
    }
    let chunk_segments = chunk_border_segments(view, rect, center, chunk_radius);
    draw_segments(game, batch, rect, &chunk_segments, border_color);
    if view.large {
        labels.push((
            localized_format!(
                "Chunk render: {} chunks",
                "Прорисовка чанков: {} чанков",
                "Rendu des chunks : {} chunks",
                "Render de chunks: {} chunks",
                "Chunk-Sichtweite: {} Chunks",
                chunk_radius
            ),
            chunk_segments,
        ));
    }
    if full_radius > chunk_radius {
        let lod_segments =
            circle_segments(view, rect, center_blocks, f64::from(full_radius) * 16.0);
        draw_segments(game, batch, rect, &lod_segments, border_color);
        if view.large {
            labels.push((
                localized_format!(
                    "Actual render (LOD): {} chunks",
                    "Фактическая прорисовка (LOD): {} чанков",
                    "Rendu réel (LOD) : {} chunks",
                    "Render real (LOD): {} chunks",
                    "Tatsächliche Sichtweite (LOD): {} Chunks",
                    full_radius
                ),
                lod_segments,
            ));
        }
    }
    if view.large
        && let Some(mouse) = mouse
    {
        let mut nearest: Option<(f32, &String)> = None;
        for (text, segments) in &labels {
            let distance = segments_distance(mouse, segments);
            let closer = nearest.is_none_or(|(best, _)| distance < best);
            if distance <= 8.0 && closer {
                nearest = Some((distance, text));
            }
        }
        if let Some((_, text)) = nearest {
            draw_border_hint(game, batch, text, mouse, rect);
        }
    }
}

fn draw_line(
    game: &Game<'_>,
    batch: &mut HudBatch,
    mut from: [f32; 2],
    mut to: [f32; 2],
    rect: [f32; 4],
    color: u32,
) {
    let [x0, y0, x1, y1] = [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]];
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let mut t0 = 0.0_f32;
    let mut t1 = 1.0_f32;
    for (p, q) in [
        (-dx, from[0] - x0),
        (dx, x1 - from[0]),
        (-dy, from[1] - y0),
        (dy, y1 - from[1]),
    ] {
        if p.abs() < f32::EPSILON {
            if q < 0.0 {
                return;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            if r > t1 {
                return;
            }
            if r > t0 {
                t0 = r;
            }
        } else {
            if r < t0 {
                return;
            }
            if r < t1 {
                t1 = r;
            }
        }
    }
    from = [from[0] + t0 * dx, from[1] + t0 * dy];
    to = [from[0] + (t1 - t0) * dx, from[1] + (t1 - t0) * dy];
    let steps = ((to[0] - from[0]).abs().max((to[1] - from[1]).abs()) / 2.0).ceil() as i32;
    for step in 0..=steps {
        let t = step as f32 / steps.max(1) as f32;
        game.native_fill(
            batch,
            [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
                2.0,
                2.0,
            ],
            color,
        );
    }
}

fn project(camera: &mod_api::Camera, world: [f64; 3], size: [f32; 2]) -> Option<[f32; 2]> {
    let relative = [
        (world[0] - camera.position[0]) as f32,
        (world[1] - camera.position[1]) as f32,
        (world[2] - camera.position[2]) as f32,
        1.0,
    ];
    let transform = |matrix: [f32; 16], vector: [f32; 4]| {
        [
            matrix[0] * vector[0]
                + matrix[4] * vector[1]
                + matrix[8] * vector[2]
                + matrix[12] * vector[3],
            matrix[1] * vector[0]
                + matrix[5] * vector[1]
                + matrix[9] * vector[2]
                + matrix[13] * vector[3],
            matrix[2] * vector[0]
                + matrix[6] * vector[1]
                + matrix[10] * vector[2]
                + matrix[14] * vector[3],
            matrix[3] * vector[0]
                + matrix[7] * vector[1]
                + matrix[11] * vector[2]
                + matrix[15] * vector[3],
        ]
    };
    let view = transform(camera.view, relative);
    let clip = transform(camera.projection, view);
    if clip[3] <= camera.near {
        return None;
    }
    let x = clip[0] / clip[3];
    let y = clip[1] / clip[3];
    Some([(x * 0.5 + 0.5) * size[0], (1.0 - (y * 0.5 + 0.5)) * size[1]])
}

/// The exact camera the game uses this frame, rebuilt from its own view state.
/// `native_camera` only returns a frame when a trait mod (`mod_api::mods`) is
/// registered; this injection-based mod has none, so it reads the same camera
/// position, rotation and field of view (including FOV/zoom changes) directly.
fn world_camera(
    game: &Game<'_>,
    player: &crate::game::native_hooks::NativePlayer,
    size: [f32; 2],
) -> mod_api::Camera {
    let partial = game.clock.camera_fraction() as f32;
    let position = game.view.position;
    let yaw = game.view.y_rot;
    let pitch = game.view.x_rot;
    let fov = game.fov_degrees(partial, position);
    mod_api::Camera {
        position,
        eye_height: game.view.eye_height,
        partial_tick: partial,
        yaw,
        pitch,
        fov_degrees: fov,
        near: 0.05,
        far: 2048.0,
        // Match `Game::render` exactly: the bob and spin matrices go into the
        // view, so markers stay glued to the world instead of drifting as the
        // camera sways.
        view: (game.bob_matrix(partial)
            * game.spinning_matrix(partial)
            * horizon_client::camera::view_matrix(yaw, pitch))
        .to_cols_array(),
        projection: horizon_client::camera::perspective(fov, size[0], size[1], 2048.0, 0.05, true),
        dimension: player.dimension.to_owned(),
    }
}

fn waypoint_distance_text(game:&Game<'_>,meters:f64) -> String {
    let limit=game.map_number("waypoint_km_at").unwrap_or(-1.);
    let precision=usize::from(game.map_option("waypoint_dist_precision").unwrap_or(1));
    // Xaero's form: one decimal and the unit right after (`205.9m`); Arabic writes its own
    // units after a space (`205.9 م`).
    let (m, km) = if matches!(language(), Language::Arabic) {(" م", " كم")} else {("m", "km")};
    if limit>=0. && meters>=limit {format!("{value:.precision$}{km}",value=meters/1000.)}
    else {format!("{meters:.precision$}{m}")}
}

fn waypoint_label_left(x: f32, marker_width: f32, text_width: f32, viewport_width: f32) -> f32 {
    let gap = marker_width / 2. + 4.;
    if x + text_width + gap + 3. > viewport_width {
        x - text_width - gap
    } else {
        x + gap
    }
}

fn draw_world_waypoints(
    game: &Game<'_>,
    batch: &mut HudBatch,
    state: &State,
    settings: &runtime::minimap::Settings,
) {
    if !settings.world_waypoints {
        return;
    }
    let Some(player) = game.native_player() else {
        return;
    };
    let camera = world_camera(game, &player, [batch.width, batch.height]);
    let dimension = Dimension::from_id(player.dimension);
    let style = settings.world_waypoint_style;
    // In the 3D view the pointer is the crosshair at the screen centre: a label
    // is "hovered" when its marker sits under it. The radius is forgiving so a
    // marker just off the crosshair still counts.
    let crosshair = [batch.width / 2.0, batch.height / 2.0];
    let min_distance = game.map_number("waypoint_min_distance").unwrap_or(0.);
    let mut max_distance = game.map_number("waypoint_max_distance").unwrap_or(0.);
    if dimension == Dimension::Nether && game.map_option("world_waypoint_distance_scale")==Some(1) {
        max_distance/=8.;
    }
    let base_scale = waypoint_scale(game,"waypoint_world_scale");
    let close_scale = game.map_number("waypoint_close_scale").unwrap_or(1.) as f32;
    let opacity = game.map_option("waypoint_world_opacity").unwrap_or(100);
    const HOVER_RADIUS: f32 = 24.0;
    let show_multiple = match game.map_option("waypoint_multiple_info").unwrap_or(1) {
        1 => game.player.as_ref().is_some_and(|player| player.local_crouching),
        2 => true,
        _ => false,
    };
    let mut hover_shown = false;
    for (index, waypoint) in state.waypoints.iter().enumerate() {
        if !waypoint.show_3d || waypoint.map_only() || waypoint.dimension != dimension
            || !map_waypoint_visible(game, state, waypoint)
        {
            continue;
        }
        let world = [
            f64::from(waypoint.x) + 0.5,
            if waypoint.options.unspecified_height { player.y + 1.5 } else { f64::from(waypoint.y) + 1.5 },
            f64::from(waypoint.z) + 0.5,
        ];
        let distance2 = (world[0]-player.x).powi(2) + (world[1]-player.y).powi(2) + (world[2]-player.z).powi(2);
        if distance2 < min_distance.powi(2) || (max_distance > 0. && distance2 > max_distance.powi(2)) {
            continue;
        }
        let scale=base_scale*(1.+(close_scale-1.)*(1.-distance2.sqrt() as f32/12.).clamp(0.,1.));
        let Some([x, y]) = project(&camera, world, [batch.width, batch.height]) else {
            continue;
        };
        if x < -8.0 || y < -8.0 || x > batch.width + 8.0 || y > batch.height + 8.0 {
            continue;
        }
        let letter = waypoint_text(state, index, waypoint);
        let width = if waypoint.icon == runtime::WaypointIcon::Letter {
            (game.map_text_width(&letter) + 4.).max(10.)
        } else {
            10.
        };
        let start = batch.vertices.len();
        game.native_fill(
            batch,
            [x - width / 2. - 1., y - 6.0, width + 2., 12.0],
            0xff101010,
        );
        game.native_fill(
            batch,
            [x - width / 2., y - 5.0, width, 10.0],
            waypoint_argb(waypoint.color),
        );
        if waypoint.icon == runtime::WaypointIcon::Skull {
            draw_skull(game, batch, [x, y], 9.5);
        } else {
            game.map_text(
                batch,
                &letter,
                [x - game.map_text_width(&letter) / 2.0, y - 4.0],
                if waypoint.color == 0 { 0xffffffff } else { 0xff201800 },
                false,
            );
        }
        batch.scale_since(start,x,y,scale,scale);
        // The coloured marker is always drawn; the label below it depends on the
        // selected style. Distance changes each frame, so it is only composed
        // when the style asks for it and, with "labels only on hover", when the
        // crosshair is over the marker.
        let hovered =
            (x - crosshair[0]).abs() <= HOVER_RADIUS && (y - crosshair[1]).abs() <= HOVER_RADIUS;
        use runtime::minimap::WorldMarkerStyle;
        let dx=world[0]-player.x;
        let dz=world[2]-player.z;
        let dy=world[1]-player.y-1.62;
        let yaw=(-dx).atan2(dz).to_degrees();
        let pitch=(-dy).atan2(dx.hypot(dz)).to_degrees();
        let within_angle = ((yaw-f64::from(player.yaw)+180.).rem_euclid(360.)-180.).abs()
            <= game.map_number("waypoint_horizontal_angle").unwrap_or(10.)
            && (pitch-f64::from(player.pitch)).abs()<=game.map_number("waypoint_vertical_angle").unwrap_or(180.);
        let distance_mode=game.map_option("waypoint_distance_mode").unwrap_or(3);
        let show_distance=match distance_mode {
            0=>false,1=>within_angle,2=>true,_=>style!=WorldMarkerStyle::Letter,
        };
        let show_name=game.map_option("waypoint_name_enabled")!=Some(0) &&
            (style==WorldMarkerStyle::NameDistance || distance_mode!=3);
        let distance=if game.map_option("waypoint_short_distance")==Some(1) {dx.hypot(dz)} else {distance2.sqrt()};
        let dist=show_distance.then(|| waypoint_distance_text(game,distance));
        let name=show_name.then_some(waypoint.name.as_str());
        if (!settings.labels_on_hover || hovered) && (dist.is_some() || name.is_some())
            && (!hovered || show_multiple || !hover_shown)
        {
            hover_shown |= hovered;
            let name_scale=waypoint_scale(game,"waypoint_name_scale");
            let dist_scale=waypoint_scale(game,"waypoint_dist_scale");
            let name_width=name.map_or(0.,|name|game.map_text_width(name)*name_scale);
            let dist_width=dist.as_ref().map_or(0.,|distance|game.map_text_width(distance)*dist_scale);
            let gap=if name.is_some() && dist.is_some(){5.} else {0.};
            let label_x=waypoint_label_left(x,width*scale,name_width+dist_width+gap,batch.width);
            let label_y=y-4.;
            let label_color=if waypoint.icon==runtime::WaypointIcon::Skull {0xffe9e6dc}
                else {waypoint_argb(waypoint.color)};
            game.native_fill(batch,[label_x-3.,label_y-2.,name_width+dist_width+gap+6.,11.*name_scale.max(dist_scale)],0xd010151d);
            for (text,left,scale) in [(name,label_x,name_scale),(dist.as_deref(),label_x+name_width+gap,dist_scale)] {
                if let Some(text)=text {
                    let first=batch.vertices.len();
                    game.map_text(batch,text,[left,label_y],label_color,true);
                    batch.scale_since(first,left,label_y,scale,scale);
                }
            }
        }
        set_opacity(batch,start,opacity);
    }
}

// Camera motion never waits for terrain preparation. Newly exposed areas may
// be empty until the next atlas arrives; existing tiles retain world positions.
fn complete_map_view(desired: View, prepared: &View) -> View {
    if desired.session != prepared.session
        || desired.dimension != prepared.dimension
        || desired.layer != prepared.layer
        || desired.large != prepared.large
    {
        return prepared.clone();
    }
    desired
}

fn official_smp(game: &Game<'_>) -> bool {
    game.remote.as_ref().is_some_and(|remote| {
        let address = remote.session.address.as_str();
        address.eq_ignore_ascii_case("play.playhorizon.dev:24454")
            || address.eq_ignore_ascii_case("play.playhorizon.dev")
    })
}

fn waypoint_identity(identity: &super::minimap_compat::ClientState) -> Option<String> {
    use super::minimap_compat::WorldId as NativeWorldId;
    match identity.world.as_ref()? {
        NativeWorldId::Singleplayer(folder) => runtime::waypoint_world(true, folder),
        NativeWorldId::Server(address) => runtime::waypoint_world(false, address),
    }
}

/// Hears the server's death screen packet. The player has not respawned yet,
/// so their current block position is the place they died.
pub(super) fn client_packet(
    game: &mut Game<'_>,
    packet: &horizon_protocol::packets::play::Clientbound,
) {
    use horizon_protocol::packets::play::Clientbound;
    if !game.native_window() || !game.map_on(runtime::MOD_ID) {
        return;
    }
    if matches!(packet, Clientbound::Login(_)) {
        WAYSTONES.lock().unwrap_or_else(|error| error.into_inner()).clear();
    }
    match packet {
        Clientbound::CustomPayload(payload) if payload.channel == "openpartiesandclaims:main" => {
            state().opac.update(&payload.data);
        }
        Clientbound::CustomPayload(payload) if payload.channel.starts_with("waystones:") => {
            super::minimap_providers::update_waystones(
                &mut WAYSTONES.lock().unwrap_or_else(|error| error.into_inner()),
                &payload.channel, &payload.data,
            );
        }
        Clientbound::Login(_) | Clientbound::Respawn(_) => {
            LOCATOR_WAYPOINTS.lock().unwrap_or_else(|error| error.into_inner()).clear();
        }
        Clientbound::TrackedWaypoint(waypoint) => {
            apply_locator_waypoint(&mut LOCATOR_WAYPOINTS.lock().unwrap_or_else(|error| error.into_inner()),waypoint);
        }
        // The chunk is handled by the game after this hook, so sampling waits
        // for the next frame instead of reading a chunk that is still absent.
        Clientbound::LevelChunkWithLight(chunk) => {
            state().biome_chunks.insert((chunk.x, chunk.z));
        }
        Clientbound::PlayerCombatKill(death)
            if game.remote.as_ref().is_some_and(|remote| remote.entity_id == death.id) =>
        {
            let controller = runtime::initialize(game.native_data_dir(runtime::MOD_ID));
            if !controller.settings().death_waypoints {
                return;
            }
            let Some(player) = game.native_player() else {
                return;
            };
            let Some(identity) = game.map_client_state() else {
                return;
            };
            let Some(world) = waypoint_identity(&identity) else {
                return;
            };
            let dimension = Dimension::from_id(player.dimension);
            if dimension == Dimension::Player {
                return;
            }
            let at = [
                player.x.floor() as i32,
                player.y.floor() as i32,
                player.z.floor() as i32,
            ];
            let biome = known_biome(game, dimension, at);
            if controller.add_death_waypoint(&world, dimension, at[0], at[1], at[2], biome.as_deref())
                && game.map_option("auto_subworld_on_death") != Some(0)
            {
                let mut state = state();
                if state.world == world {
                    state.waypoint_group = None;
                    state.waypoint_scroll = 0;
                    state.waypoint_selected = None;
                    controller.select_waypoint_group(&world, None);
                }
            }
        }
        _ => {}
    }
}

fn cave_allowed(game:&Game<'_>,large:bool,dimension:Dimension)->bool {
    if large && (game.map_option("world_cave_enabled") == Some(0) || game.map_option("world_cave_type") == Some(0)) {
        return false;
    }
    let key=match (large,dimension) {
        (false,Dimension::Overworld)=>"minimap_cave_surface",
        (false,Dimension::Nether)=>"minimap_cave_nether",
        (false,Dimension::End)=>"minimap_cave_end",
        (true,Dimension::Overworld)=>"world_cave_surface",
        (true,Dimension::Nether)=>"world_cave_nether",
        (true,Dimension::End)=>"world_cave_end",
        (_,Dimension::Player)=>return true,
    };
    game.map_option(key)!=Some(0)
}

fn terrain_appearance(game: &Game<'_>, large: bool) -> maps::Appearance {
    use maps::Appearance as A;
    let mut appearance = 0;
    let option = |mini, world| game.map_option(if large { world } else { mini });
    if option("minimap_block_colors", "world_block_colors") == Some(1) {
        appearance |= A::VANILLA;
        if option("minimap_vanilla_biomes", "world_vanilla_biomes") == Some(1) {
            appearance |= A::VANILLA_BIOMES;
        }
    }
    if (!A(appearance).has(A::VANILLA) || A(appearance).has(A::VANILLA_BIOMES))
        && option("minimap_biome_blending", "world_biome_blending") == Some(0)
    {
        appearance |= A::HARD_BIOME_EDGES;
    }
    if !large && game.map_option("minimap_block_transparency") == Some(0) {
        appearance |= A::OPAQUE;
    }
    if option("minimap_flowers", "world_flowers") == Some(0) {
        appearance |= A::HIDE_FLOWERS;
    }
    if option("minimap_stained_glass", "world_stained_glass") == Some(0) {
        appearance |= A::HIDE_STAINED_GLASS;
    }
    if !large && game.map_option("minimap_redstone") == Some(0) {
        appearance |= A::HIDE_REDSTONE;
    }
    if option("minimap_short_block_height", "world_short_block_height") == Some(1) {
        appearance |= A::SHORT_HEIGHT;
    }
    if option("minimap_lighting", "world_lighting") != Some(0) {
        appearance |= A::LIGHTING;
    }
    A(appearance)
}

pub(super) fn frame(game: &mut Game<'_>, elapsed: Duration) {
    let _timing = runtime::profiling::measure("client-frame");
    if !game.native_window() {
        return;
    }
    let controller = runtime::initialize(game.native_data_dir(runtime::MOD_ID));
    if !game.map_on(runtime::MOD_ID) {
        if game.mod_screen() == Some(runtime::MOD_ID) {
            game.mod_screen_close(runtime::MOD_ID);
        }
        return;
    }
    let Some(player) = game.native_player() else {
        return;
    };
    let Some(identity) = game.map_client_state() else {
        return;
    };
    if game.map_on("team") {
        refresh_team_skins(game);
    }
    let (copy, command, change) = {
        let mut state = state();
        (state.pending_copy.take(), state.pending_command.take(), state.map_settings_change.take())
    };
    if let Some((key, value)) = change {
        super::minimap_compat::write_map_setting(game, &key, &value);
    }
    if let Some(text) = copy {
        use horizon_client::clipboard::Clipboard;
        game.clipboard.set(&text);
    }
    if let Some(command) = command {
        game.click_command(&command);
    }
    let mut state = state();
    if let Some(import) = state.settings_import.as_ref().and_then(|rx| match rx.try_recv() {
        Ok(result) => Some(result),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(std::io::Error::other("Settings import was interrupted"))),
        Err(std::sync::mpsc::TryRecvError::Empty) => None,
    }) {
        state.settings_import = None;
        let message = match import.and_then(|data| super::minimap_compat::apply_xaero_settings(game,data)) {
            Ok((applied,skipped,profile)) => {
                state.xaero_radar_enabled=None;
                state.xaero_profile=profile;

                if !skipped.is_empty() {
                    eprintln!("[{}] Unapplied Xaero settings: {}", runtime::MOD_ID, skipped.join(", "));
                }
                let mut summary=localized_format!(
                    "Settings imported {} · skipped {}", "Настроек перенесено {} · пропущено {}",
                    "{} réglages importés · {} ignorés", "{} ajustes importados · {} omitidos",
                    "{} Einstellungen übernommen · {} übersprungen", applied, skipped.len()
                );
                if let Some(name)=&state.xaero_profile {summary.push_str(&format!(" · {name}"));}
                summary
            }
            Err(error) => error.to_string(),
        };
        state.notice = Some((message, Instant::now()));
    }
    let sprites_version = game.block_sprites_version();
    if state.leaves.as_ref().map(|(version, _)| *version) != Some(sprites_version) {
        state.leaves = Some((sprites_version, Arc::new(build_leaf_palette(game))));
    }
    if state.close {
        state.close = false;
        runtime::set_text_input(false);
        game.mod_screen_close(runtime::MOD_ID);
    }
    if game.mod_screen() != Some(runtime::MOD_ID) {
        runtime::set_text_input(false);
        if state.mode == Mode::Name {
            state.mode = Mode::Map;
            state.name_form = None;
        }
    }
    if let Some(request) = runtime::take_screen_request() {
        let desired = match request {
            runtime::ScreenRequest::Map | runtime::ScreenRequest::Slime => Mode::Map,
            runtime::ScreenRequest::Waypoints => Mode::Waypoints,
            runtime::ScreenRequest::Create if state.large => Mode::Map,
            runtime::ScreenRequest::Create => Mode::Waypoints,
            runtime::ScreenRequest::Settings | runtime::ScreenRequest::SettingsOverlay => Mode::Settings,
        };
        // The key helper opens the screen; this side only chooses its mode.
        if game.mod_screen() == Some(runtime::MOD_ID) {
            if matches!(request,runtime::ScreenRequest::Settings|runtime::ScreenRequest::SettingsOverlay) {
                let back = if request == runtime::ScreenRequest::Settings {
                    None
                } else {
                    match state.mode {
                        Mode::Map => Some(Mode::Map),
                        Mode::Waypoints => Some(Mode::Waypoints),
                        Mode::Settings => state.settings_return,
                        _ => None,
                    }
                };
                let tab = back.map(|mode| if mode == Mode::Map {1} else {2});
                open_settings(&mut state,back,tab);
            } else {
                state.mode = desired;
            }
            state.map_panel = MapPanel::None;
            state.slime_preview = request == runtime::ScreenRequest::Slime;
            if state.slime_preview && state.slime_seed.is_none() {
                state.notice = Some((tr("Slime chunks require a known seed", "Для карты слизи нужен сид мира",
                    "Les chunks à slimes nécessitent la graine", "Los chunks de slime necesitan la semilla",
                    "Schleim-Chunks benötigen den Welt-Seed").into(), Instant::now()));
            }
            state.info = None;
            state.name_form = None;
            state.menu = None;
            state.waypoint_search_focus = false;
            state.waypoint_groups_open = false;
            state.waypoint_selected = None;
            runtime::set_text_input(false);
            if request == runtime::ScreenRequest::Create {
                create_waypoint_here(game, &mut state);
            }
        }
    }
    state.portals = controller.maps.portals();
    let world = waypoint_identity(&identity).unwrap_or_default();
    if state.world != world {
        controller.clear_temporary_waypoints(&state.world);
        let next = state.session + 1;
        if let Err(error) = ::map::radar::load_rules(&game.native_data_dir(runtime::MOD_ID), false) {
            eprintln!("[{}] Radar rules: {error}", runtime::MOD_ID);
        }
        let slime_seed = if game.integrated.is_some() {
            Some(game.level.seed)
        } else {
            use std::io::Read;
            let mut text = String::new();
            let path = game.native_data_dir(runtime::MOD_ID).join("slime-seeds.ini");
            std::fs::File::open(path).ok().and_then(|file| {
                file.take(64 * 1024).read_to_string(&mut text).ok()?;
                runtime::slime_seed(&text, &world)
            })
        };
        *state = State {
            waypoint_group: controller.active_waypoint_group(&world),
            world,
            session: next,
            slime_seed,
            xaero_profile: ::map::xaero_settings::active_profile(&game.native_data_dir(runtime::MOD_ID)),
            pending_command: (official_smp(game)
                && (game.map_option("minimap_claims") != Some(0)
                    || game.map_option("world_claims") != Some(0)
                    || game.map_option("minimap_current_claim") != Some(0))).then(|| "claims".to_owned()),
            ..State::default()
        };
        controller.maps.reset(next);
    }
    use ::map::map_claims;
    let claim_replies = map_claims::take_replies();
    if official_smp(game) {
        for reply in claim_replies {
            match reply {
                map_claims::Reply::Own(own) => {
                    log::info!("[{}] Synced {} owned chunks from Horizon SMP", runtime::MOD_ID, own.len());
                    state.claims.retain(|_, owner| owner.is_some());
                    for chunk in own { state.claims.insert(chunk, None); }
                }
                map_claims::Reply::Here(chunk, Some(owner)) => {
                    if state.claims.len() < 8192 { state.claims.entry(chunk).or_insert(Some(owner)); }
                }
                map_claims::Reply::Here(chunk, None) => { state.claims.remove(&chunk); }
            }
        }
    }
    let enabled = game.map_option("use_xaero_radar") == Some(1);
    if state.xaero_radar_enabled != Some(enabled) {
        let categories = enabled.then(|| {
            use ::map::xaero_settings;
            let directory=xaero_settings::profile_directory(&game.native_data_dir(runtime::MOD_ID),state.xaero_profile.as_deref());
            let source=xaero_settings::read_radar_categories(&directory).ok().flatten()?;
            super::minimap_compat::xaero_radar_categories(&source,&mut Vec::new())
        }).flatten();
        use ::map::xaero_settings;
        let directory=game.native_data_dir(runtime::MOD_ID);
        let item_config=xaero_settings::commands_directory(&directory,state.xaero_profile.as_deref());
        let values=xaero_settings::teleport_commands(&item_config).unwrap_or_default();
        state.map_items=["minimap_item","map_item"].map(|key|values.get(key)
            .filter(|item|item.as_str()!="-").cloned());
        ::map::radar::use_imported(categories);
        let info_directory = xaero_settings::profile_directory(&directory, state.xaero_profile.as_deref());
        state.info_displays = xaero_settings::read_info_display(&info_directory).ok().flatten();
        state.xaero_radar_enabled = Some(enabled);
    }
    if let Some(point) = runtime::take_shared_waypoint()
        && point.world == state.world
    {
        let message = match controller.merge_waypoints(&state.world,vec![point]) {
            Ok(_) => tr("Waypoint imported", "Метка импортирована", "Repère importé", "Marcador importado", "Wegpunkt importiert").to_owned(),
            Err(error) => error,
        };
        state.notice = Some((message,Instant::now()));
    }
    update_cache_ui(game, &mut state);
    poll_map_export(&mut state);
    poll_waypoint_import(&mut state);
    poll_cache_import(&mut state);
    state.waypoints = controller.waypoints(&state.world);
    let dimension = Dimension::from_id(player.dimension);
    controller.maps.configure(
        state.session,
        &state.world,
        dimension,
        controller.settings().cache_ram_mib,
    );
    if state.dimension != dimension {
        state.lod_sources.clear();
        state.frame = None;
        state.portals.clear();

        state.seen.clear();
        state.chunk_keys.clear();
        state.chunk_revision = 0;
        state.chunk_cursor = None;
        state.cave_pending=[None;2];
        state.cave_open=[false;2];
        state.dimension = dimension;
    }
    let head = (player.y + 1.62).floor() as i32;
    let settings = controller.settings();
    use runtime::minimap::CaveView;
    let screen_open=game.mod_screen()==Some(runtime::MOD_ID);
    let large=screen_open && map_behind(&state);
    let appearance = terrain_appearance(game, large);
    if state.appearance != appearance {
        state.seen.clear();
        state.appearance = appearance;
    }
    let index=usize::from(large);
    let override_mode=if large {game.map_number("world_auto_cave").unwrap_or(-1.) as i32}else{-1};
    let cave_view=if override_mode==0 {CaveView::Surface}else if override_mode>0 {CaveView::Auto}else{settings.cave_view};
    let roof=if override_mode>0 {override_mode} else {i32::from(game.map_option("minimap_cave_roof").unwrap_or(1))};
    let roof_radius=roof.clamp(1,3)-1;
    let covered=cave_view==CaveView::Auto && (dimension==Dimension::Nether ||
        (-roof_radius..=roof_radius).all(|dx|(-roof_radius..=roof_radius).all(|dz| {
            game.map_light_if_loaded([player.x.floor() as i32+dx,head,player.z.floor() as i32+dz])
                .is_some_and(|(_,sky)|sky==0)
        })));
    if cave_view==CaveView::Auto && state.cave_open[index]!=covered {
        if state.cave_pending[index].is_none_or(|(expected,_)|expected!=covered) {
            state.cave_pending[index]=Some((covered,Instant::now()));
        }
        let ms=game.map_number(if large {"world_cave_timer"}else{"minimap_cave_timer"}).unwrap_or(0.) as u64;
        if state.cave_pending[index].is_some_and(|(_,since)|since.elapsed()>=Duration::from_millis(ms)) {
            state.cave_open[index]=covered;
            state.cave_pending[index]=None;
        }
    } else {
        state.cave_pending[index]=None;
    }
    let layer=if cave_allowed(game,large,dimension) {
        match cave_view {
            CaveView::Underground=>Layer::Below(head),
            CaveView::Manual=>Layer::Below(settings.cave_y),
            CaveView::Auto if state.cave_open[index]=>Layer::Below(head),
            _=>Layer::Surface,
        }
    } else {Layer::Surface};
    let layer = if large && game.map_option("world_cave_type") == Some(2) && matches!(layer, Layer::Below(_)) {
        Layer::Below(maps::FULL_CAVE)
    } else {
        layer
    };
    state.layers.insert(dimension,layer);
    if settings.footsteps {
        let point = (dimension, [player.x.floor() as i32, player.z.floor() as i32]);
        if state.footsteps.back() != Some(&point) {
            state.footsteps.push_back(point);
            if state.footsteps.len() > 512 {
                state.footsteps.pop_front();
            }
        }
    } else {
        state.footsteps.clear();
    }
    if dimension != Dimension::Player && game.screen != super::Screen::Dead {
        let at = [player.x,player.y,player.z];
        let mut reached = Vec::new();
        let mut previous = std::mem::take(&mut state.destinations);
        let mut destinations = BTreeMap::new();
        for point in state.waypoints.iter().filter(|point| {
            point.options.destination || (point.dimension==dimension && point.icon==runtime::WaypointIcon::Skull)
        }) {
            let key = point.key();
            let mut arrived = false;
            if point.options.destination {
                let start = previous.remove(&key).unwrap_or_else(Instant::now);
                arrived = point.dimension==dimension && start.elapsed()>=Duration::from_secs(5) && point.within_reach(at);
                if !arrived {destinations.insert(key,start);}
            }
            if arrived || (point.dimension==dimension && settings.death_waypoint_auto_remove && runtime::death_waypoint_reached(at,point)) {
                reached.push(key);
            }
        }
        state.destinations = destinations;
        if !reached.is_empty() {
            for key in reached {
                controller.delete_waypoint(&state.world, key);
            }
            state.waypoints = controller.waypoints(&state.world);
        }
    }
    let selected = if large {
        settings.world_dimension
    } else {
        settings.mini_dimension
    }
    .resolve(dimension);
    let position = dimension.convert(selected, [player.x, player.z]);
    if selected != state.selected || large != state.large {
        if large!=state.large {state.map_opened=large.then(Instant::now);}
        state.camera.snap(position);
        state.marker = position;
        state.selected = selected;
        state.large = large;
        state.dragging = false;
        state.menu = None;
    }
    if !large {
        state.camera.follow(position);
    }
    apply_map_focus(&mut state, large, selected);
    let dt = elapsed.as_secs_f64();
    state.camera.advance(dt);
    state.marker = maps::follow_point(state.marker, position, dt);
    state.lod_elapsed += dt;
    if state.lod_elapsed >= 0.05 {
        state.lod_elapsed = 0.0;
        // Far terrain the game only estimates (its LOD) fills the map beyond what was explored;
        // the maps show explored ground only unless asked.
        if game.map_option("world_far_terrain") == Some(1)
            && let Some(lod) = game.lod.as_ref()
        {
            let _timing = runtime::profiling::measure("lod-snapshot-pick");
            state
                .lod_sources
                .retain(|_, (source, _)| source.strong_count() != 0);
            let known = known_lod_sources(&state, dimension);
            let span = controller
                .maps
                .requested()
                .filter(|v| v.dimension == dimension)
                .map_or([256.; 2], |v| v.span);
            let focus = if selected == dimension {
                state.camera.target
            } else {
                [player.x, player.z]
            };
            for (key, columns) in super::minimap_lod_adapter::snapshots(lod, &known, focus, span, 8)
            {
                let source = Arc::downgrade(&columns);
                let biome_palette = game
                    .world_biomes
                    .as_ref()
                    .unwrap_or_else(|| game.res.biomes())
                    .clone();
                let leaves = state.leaves.as_ref().map(|(_, leaves)| Arc::clone(leaves));
                let resources = Arc::clone(&game.res);
                let key = maps::LodKey {
                    x: key.x,
                    z: key.z,
                    level: key.level,
                };
                let accepted =
                    controller
                        .maps
                        .submit_lod(state.session, dimension, key, move || {
                            let mut biome_names = BTreeMap::<u16, Arc<str>>::new();
                            let book = resources.facts();
                            let [origin_x, origin_z, size, _] = key.bounds();
                            let stride = size / 16;
                            let biome_at = |wx: i32, wz: i32| -> Option<u16> {
                                let dx = i64::from(wx) - origin_x;
                                let dz = i64::from(wz) - origin_z;
                                if dx < 0 || dz < 0 || dx >= size || dz >= size {
                                    return None;
                                }
                                let cell = ((dz / stride) * 16 + (dx / stride)) as usize;
                                columns.get(cell)?.surface.map(|surface| surface.biome)
                            };
                            let color = |id: horizon_world::registry::BlockStateId,
                                         at: [i32; 2]| {
                                let state = book.states.get(id.0 as usize)?;
                                if book.is_air(id) {
                                    return None;
                                }
                                let base = plant_color(state.name.as_ref()).unwrap_or_else(|| {
                                    horizon_client::blocks::MAP_COLORS
                                        .get(state.map_color as usize)
                                        .copied()
                                        .unwrap_or(0)
                                });
                                let leaf = leaves
                                    .as_ref()
                                    .and_then(|leaves| leaves.states.get(&id.0))
                                    .copied();
                                let water =
                                    matches!(state.fluid, horizon_client::blocks::Fluid::Water);
                                let needs_grass = leaf
                                    .is_some_and(|leaf| leaf.tint == LeafTint::Grass)
                                    || maps::needs_grass_tint(state.name.as_ref());
                                let needs_foliage = leaf
                                    .is_some_and(|leaf| leaf.tint == LeafTint::Foliage)
                                    || maps::needs_foliage_tint(state.name.as_ref());
                                let blend = blend_biome_tints(
                                    &biome_palette,
                                    at,
                                    BIOME_BLEND_RADIUS,
                                    BIOME_BLEND_STEP,
                                    needs_grass,
                                    needs_foliage,
                                    water,
                                    |wx, wz| biome_at(wx, wz),
                                );
                                let rgb = if water {
                                    multiply_color(
                                        blend.water.unwrap_or_else(|| if base == 0 { 0x3f76e4 } else { base }),
                                        leaves.as_ref().map_or(0xffffff, |p| p.water_texture),
                                    )
                                } else {
                                    leaf.and_then(|leaf| leaf.map_color(blend.grass, blend.foliage))
                                        .unwrap_or_else(|| {
                                            plant_color(state.name.as_ref()).unwrap_or_else(|| {
                                                maps::block_map_color(
                                                    state.name.as_ref(),
                                                    base,
                                                    blend.grass,
                                                    blend.foliage,
                                                )
                                            })
                                        })
                                };
                                Some(rgb)
                            };
                            let mut tile = maps::Tile {
                                pixels: vec![0; 1024],
                                biomes: vec![None; 256],
                                ..maps::Tile::default()
                            };
                            for (i, column) in columns.iter().enumerate() {
                                let Some(surface) = column.surface else {
                                    continue;
                                };
                                let at = [
                                    (origin_x + (i as i64 % 16) * stride + stride / 2) as i32,
                                    (origin_z + (i as i64 / 16) * stride + stride / 2) as i32,
                                ];
                                let height = surface
                                    .water
                                    .map_or(surface.height, |(h, _)| h)
                                    .saturating_sub(1);
                                let snows = dimension == Dimension::Overworld
                                    && biome_palette.list.get(surface.biome as usize).is_some_and(
                                        |b| b.snows_at(at[0], height, at[1]) == Some(true),
                                    );
                                let snow = leaves.as_ref().map_or(0xffffff, |p| p.snow);
                                let ice = leaves.as_ref().map_or(0xa0a0ff, |p| p.ice);
                                let base = color(surface.state, at);
                                let rgb = if let Some((height, fluid)) = surface.water {
                                    let Some(fluid_color) = color(fluid, at) else {
                                        continue;
                                    };
                                    if book.states.get(fluid.0 as usize).is_some_and(|s| {
                                        matches!(s.fluid, horizon_client::blocks::Fluid::Water)
                                    }) {
                                        maps::lod_water_color(
                                            base,
                                            fluid_color,
                                            height.saturating_sub(surface.height).max(0) as u32,
                                            snows,
                                            ice,
                                        )
                                    } else {
                                        fluid_color
                                    }
                                } else {
                                    let Some(base) = base else {
                                        continue;
                                    };
                                    let name = book
                                        .states
                                        .get(surface.state.0 as usize)
                                        .map_or("", |s| s.name.as_ref());
                                    maps::lod_snow_color(name, base, snow, snows)
                                };
                                if let Some(info) = biome_palette.list.get(surface.biome as usize) {
                                    let name = biome_names
                                        .entry(surface.biome)
                                        .or_insert_with(|| Arc::from(info.name.as_str()));
                                    tile.biomes[i] = Some(name.clone());
                                }
                                tile.heights[i] = surface
                                    .water
                                    .map(|(height, _)| height)
                                    .unwrap_or(surface.height)
                                    .saturating_sub(1);
                                tile.pixels[i * 4..i * 4 + 4].copy_from_slice(&[
                                    (rgb >> 16) as u8,
                                    (rgb >> 8) as u8,
                                    rgb as u8,
                                    255,
                                ]);
                            }
                            tile
                        });
                if accepted {
                    state
                        .lod_sources
                        .insert((dimension, key), (source, Instant::now()));
                }
            }
        }
    }
    state.elapsed += dt;
    if state.elapsed >= 0.05 {
        state.elapsed = 0.0;
        // Persist each newly sampled biome with its waypoint. A known biome is
        // only refreshed when its chunk arrives; an unknown one is retried
        // until the chunk is loaded. Only a change reaches the settings
        // worker, so an unchanged sample writes nothing.
        let arrived = std::mem::take(&mut state.biome_chunks);
        for index in 0..state.waypoints.len() {
            let (world, dimension, x, y, z, current) = {
                let waypoint = &state.waypoints[index];
                (
                    waypoint.world.clone(),
                    waypoint.dimension,
                    waypoint.x,
                    waypoint.y,
                    waypoint.z,
                    waypoint.biome.clone(),
                )
            };
            if current.is_some() && !arrived.contains(&(x.div_euclid(16), z.div_euclid(16))) {
                continue;
            }
            let Some(biome) = known_biome(game, dimension, [x, y, z]) else {
                continue;
            };
            if current.as_deref() == Some(biome.as_str()) {
                continue;
            }
            state.waypoints[index].biome = Some(biome.clone());
            controller.change_waypoint(&world, state.waypoints[index].key(), |point| point.biome=Some(biome));
        }
        let cx = (player.x.floor() as i32).div_euclid(16);
        let cz = (player.z.floor() as i32).div_euclid(16);
        let _timing = runtime::profiling::measure("chunk-collection");
        let revision = game.world.chunks.membership_revision();
        if state.chunk_revision != revision {
            if let Some(deltas) = game.world.chunks.deltas_since(state.chunk_revision) {
                use horizon_client::chunk_map::ChunkMembershipDelta;
                for (_, delta) in deltas {
                    match delta {
                        ChunkMembershipDelta::Added(pos) => {
                            state.chunk_keys.insert(pos);
                        }
                        ChunkMembershipDelta::Removed(pos) => {
                            state.chunk_keys.remove(&pos);
                            state.seen.remove(&pos);
                        }
                        ChunkMembershipDelta::Reset => {
                            unreachable!("reset requires membership rebuild")
                        }
                    }
                }
            } else {
                let keys: BTreeSet<_> = game.world.chunks.keys().copied().collect();
                state.seen.retain(|pos, _| keys.contains(pos));
                state.chunk_keys = keys;
            }
            state.chunk_revision = revision;
        }
        let requested = controller.maps.requested();
        let anchor = requested
            .as_ref()
            .filter(|v| v.dimension == dimension)
            .map_or([cx, cz], |v| {
                v.center.map(|v| (v.floor() as i32).div_euclid(16))
            });
        let mut positions: BTreeSet<_> = arrived.iter().copied().take(64).collect();
        for z in -4..=4 {
            for x in -4..=4 {
                positions.insert((anchor[0].saturating_add(x), anchor[1].saturating_add(z)));
            }
        }
        use std::ops::Bound::{Excluded, Unbounded};
        let scan: Vec<_> = state
            .chunk_keys
            .range((state.chunk_cursor.map_or(Unbounded, Excluded), Unbounded))
            .take(64)
            .copied()
            .collect();
        state.chunk_cursor = if scan.len() == 64 {
            scan.last().copied()
        } else {
            None
        };
        positions.extend(scan);
        let mut chunks: Vec<_> = positions
            .into_iter()
            .filter_map(|pos| {
                game.world
                    .chunks
                    .get(&pos)
                    .map(|chunk| (pos, Arc::downgrade(chunk)))
            })
            .collect();
        chunks.sort_by_key(|(pos, _)| {
            (
                !requested
                    .as_ref()
                    .is_some_and(|v| v.demands(dimension, layer, [pos.0, pos.1])),
                i64::from(pos.0 - cx).pow(2) + i64::from(pos.1 - cz).pow(2),
            )
        });
        let mut sent = 0;
        let mut attempted = 0;
        let collection_start = Instant::now();
        let server=game.remote.is_some() && game.integrated.is_none();
        let write_radius=game.map_number("map_write_radius").unwrap_or(-1.) as i32;
        let load=game.map_option("map_load_chunks")!=Some(0);
        let update=game.map_option("map_update_chunks")!=Some(0);
        let limited=server && write_radius>=0;
        let coverage=(!load || !update).then(||controller.maps.coverage());
        let budget=if game.map_option("map_fast_writes")==Some(1) {1400}else{800};
        let chunk_budget=if budget>800 {8}else{4};
        for (pos, weak) in chunks {
            if limited && ((pos.0-cx).abs()>write_radius || (pos.1-cz).abs()>write_radius) {
                continue;
            }
            if let Some(coverage)=coverage.as_ref() {
                let known=coverage.detailed_at(dimension,layer,[pos.0*16,pos.1*16]);
                if (known && !update) || (!known && !load) {continue;}
            }
            if attempted > 0 && collection_start.elapsed() >= Duration::from_micros(budget) {
                break;
            }
            let visible = requested
                .as_ref()
                .is_some_and(|v| v.demands(dimension, layer, [pos.0, pos.1]));
            if state.seen.get(&pos).is_some_and(|(old, old_layer, sent)| {
                Weak::ptr_eq(old, &weak)
                    && *old_layer == layer
                    && (!visible
                        || sent.elapsed() < Duration::from_millis(500)
                        || controller.maps.contains(dimension, layer, [pos.0, pos.1]))
            }) {
                continue;
            }
            attempted += 1;
            let snapshot = {
                let _sampling = runtime::profiling::measure("snapshot-collection");
                game.map_chunks([pos.0 - 1, pos.1 - 1], [pos.0 + 1, pos.1 + 1])
            };
            let Ok(snapshot) = snapshot else {
                continue;
            };
            let resources = Arc::clone(&game.res);
            let biome_palette = game
                .world_biomes
                .as_ref()
                .unwrap_or_else(|| game.res.biomes())
                .clone();
            let leaves = state.leaves.as_ref().map(|(_, leaves)| Arc::clone(leaves));
            let accepted = controller.maps.submit_with_appearance(
                state.session,
                dimension,
                layer,
                [pos.0, pos.1],
                move || {
                    let book = resources.facts();
                    let minimum = snapshot.height().start;
                    use maps::Appearance as A;
                    let sample = |style: A| {
                        let mut names = BTreeMap::<u16, Arc<str>>::new();
                        maps::sample_with_biomes(
                            |x, y, z| {
                                let id = snapshot.block([x, y, z]);
                                let air = book.is_air(id);
                                let Some(state) = book.states.get(id.0 as usize) else {
                                    return maps::Block {
                                        rgb: 0,
                                        air,
                                        alpha: 0,
                                        hidden: false,
                                        water: false,
                                        portal: None,
                                    };
                                };
                                let name = state.name.as_ref();
                                let class = |name| state.classes.iter().any(|class| class.as_ref() == name);
                                let stained_glass = name.contains("stained_glass");
                                let redstone = matches!(name, "minecraft:redstone_wire" | "minecraft:redstone_torch" | "minecraft:redstone_wall_torch" | "minecraft:repeater" | "minecraft:comparator");
                                let flower = class("FlowerBlock") || class("TallFlowerBlock")
                                    || matches!(name, "minecraft:torchflower" | "minecraft:pitcher_plant" | "minecraft:pink_petals" | "minecraft:wildflowers" | "minecraft:flowerbed");
                                let hidden = (hidden_cover(state.classes.as_ref(), name)
                                    && !flower)
                                    || (style.has(A::HIDE_FLOWERS) && flower)
                                    || (style.has(A::HIDE_REDSTONE) && redstone)
                                    || (style.has(A::HIDE_STAINED_GLASS) && stained_glass);
                                let portal = match state.name.as_ref() {
                                    "minecraft:nether_portal" => Some(Portal::Nether),
                                    "minecraft:end_portal" => Some(Portal::End),
                                    "minecraft:end_gateway" => Some(Portal::Gateway),
                                    _ => None,
                                };
                                if air || hidden {
                                    return maps::Block {
                                        rgb: 0,
                                        air,
                                        alpha: 0,
                                        hidden,
                                        water: false,
                                        portal: None,
                                    };
                                }
                                if let Some(kind) = portal {
                                    let (rgb, alpha) = portal_tint(kind);
                                    return maps::Block {
                                        rgb,
                                        air,
                                        alpha,
                                        hidden: false,
                                        water: false,
                                        portal: Some(kind),
                                    };
                                }
                                let water = matches!(state.fluid, horizon_client::blocks::Fluid::Water);
                                let fluid = !matches!(state.fluid, horizon_client::blocks::Fluid::None);
                                let invisible = matches!(
                                    state.render_shape,
                                    horizon_client::blocks::RenderShape::Invisible
                                );
                                if invisible && !fluid {
                                    // Barriers, light blocks, moving pistons and
                                    // other render-invisible blocks leave no pixel.
                                    return maps::Block {
                                        rgb: 0,
                                        air,
                                        alpha: 0,
                                        hidden: true,
                                        water: false,
                                        portal: None,
                                    };
                                }
                                let base = horizon_client::blocks::MAP_COLORS
                                    .get(state.map_color as usize)
                                    .copied()
                                    .unwrap_or(0);
                                let leaf = leaves
                                    .as_ref()
                                    .and_then(|leaves| leaves.states.get(&id.0))
                                    .copied();
                                let needs_grass = leaf.is_some_and(|leaf| leaf.tint == LeafTint::Grass)
                                    || maps::needs_grass_tint(state.name.as_ref());
                                let needs_foliage = leaf
                                    .is_some_and(|leaf| leaf.tint == LeafTint::Foliage)
                                    || maps::needs_foliage_tint(state.name.as_ref());
                                let tint_biomes = !style.has(A::VANILLA) || style.has(A::VANILLA_BIOMES);
                                let blend = blend_biome_tints(
                                    &biome_palette,
                                    [x, z],
                                    if style.has(A::HARD_BIOME_EDGES) { 0 } else { BIOME_BLEND_RADIUS },
                                    BIOME_BLEND_STEP,
                                    tint_biomes && needs_grass,
                                    tint_biomes && needs_foliage,
                                    tint_biomes && water,
                                    |wx, wz| snapshot.biome([wx, y, wz]).map(|biome| biome.0),
                                );
                                let rgb = if redstone && base == 0 && !style.has(A::VANILLA) {
                                    0xad160a
                                } else if water {
                                    multiply_color(
                                        blend.water.unwrap_or_else(|| if base == 0 { 0x3f76e4 } else { base }),
                                        leaves.as_ref().map_or(0xffffff, |p| p.water_texture),
                                    )
                                } else if style.has(A::VANILLA) {
                                    blend.grass.or(blend.foliage).unwrap_or(base)
                                } else {
                                    leaf.and_then(|leaf| leaf.map_color(blend.grass, blend.foliage))
                                        .unwrap_or_else(|| {
                                            plant_color(state.name.as_ref()).unwrap_or_else(|| {
                                                maps::block_map_color(
                                                    state.name.as_ref(),
                                                    base,
                                                    blend.grass,
                                                    blend.foliage,
                                                )
                                            })
                                        })
                                };
                                let glass = state.classes.iter().any(|class| {
                                    class.as_ref() == "TransparentBlock"
                                        || class.as_ref() == "HalfTransparentBlock"
                                }) || state.name.contains("glass");
                                let soft = matches!(
                                    state.name.as_ref(),
                                    "minecraft:scaffolding" | "minecraft:cobweb"
                                );
                                let (rgb, mut alpha) = if water {
                                    (if rgb == 0 { 0x3f76e4 } else { rgb }, 120)
                                } else if glass {
                                    let tint = if rgb == 0 {
                                        dye_color(state.name.as_ref()).unwrap_or(0xbfd8e6)
                                    } else {
                                        rgb
                                    };
                                    let alpha = match state.name.as_ref() {
                                        "minecraft:tinted_glass" => 190,
                                        "minecraft:ice"
                                        | "minecraft:blue_ice"
                                        | "minecraft:frosted_ice" => 130,
                                        _ if state.name.contains("glass_pane") => 80,
                                        _ => 96,
                                    };
                                    (tint, alpha)
                                } else if soft {
                                    (if rgb == 0 { 0xbfb8a8 } else { rgb }, 90)
                                } else {
                                    (rgb, 0)
                                };
                                if style.has(A::OPAQUE) {
                                    alpha = 0;
                                }
                                maps::Block {
                                    rgb,
                                    air,
                                    alpha,
                                    hidden: false,
                                    water,
                                    portal: None,
                                }
                            },
                            |x, z| snapshot.section_top([x, z]),
                            [pos.0, pos.1],
                            minimum,
                            layer,
                            |at| {
                                let id = snapshot.biome(at)?.0;
                                let info = biome_palette.list.get(id as usize)?;
                                Some(
                                    names
                                        .entry(id)
                                        .or_insert_with(|| Arc::from(info.name.as_str()))
                                        .clone(),
                                )
                            },
                        )
                    };
                    let base = sample(A::default());
                    let alternate = (appearance != A::default()).then(|| {
                        let adjusted = appearance.0 & !(A::SHORT_HEIGHT | A::LIGHTING);
                        let mut tile = if adjusted == 0 { base.clone() } else { sample(A(adjusted)) };
                        for (i, height) in tile.heights.iter_mut().enumerate() {
                            if tile.pixels[i * 4 + 3] == 0 {
                                continue;
                            }
                            let x = pos.0 * 16 + i as i32 % 16;
                            let z = pos.1 * 16 + i as i32 / 16;
                            if appearance.has(A::LIGHTING) {
                                let light = snapshot.light([x, *height + 1, z]);
                                let level = (light & 15).max(light >> 4);
                                if level < 15 {
                                    let shade = 96 + u16::from(level) * 159 / 15;
                                    for channel in &mut tile.pixels[i * 4..i * 4 + 3] {
                                        *channel = (u16::from(*channel) * shade / 255) as u8;
                                    }
                                }
                            }
                            if appearance.has(A::SHORT_HEIGHT)
                                && let Some(block) = book.states.get(snapshot.block([x, *height, z]).0 as usize)
                                && !block.outline.is_empty()
                                && block.outline.iter().all(|bounds| bounds[4] <= 0.25)
                            {
                                *height -= 1;
                            }
                        }
                        (appearance, tile)
                    });
                    (base, alternate)
                },
            );
            if !accepted {
                break;
            }
            state.seen.insert(pos, (weak, layer, Instant::now()));
            sent += 1;
            if sent == chunk_budget {
                break;
            }
        }
    }
    let mut current_view = None;
    if let Some(renderer) = game.renderer.as_ref() {
        let viewport = HudBatch::new(
            [renderer.config.width, renderer.config.height],
            game.settings.gui_scale_inputs(),
        );
        let rect = rectangle(viewport.width, viewport.height);
        let (width, height, span) = if large {
            // The HUD draws in GUI pixels, but the texture is sampled at the
            // window's physical resolution. Keep this coordinate space for
            // overlays; terrain tiles are now rasterized directly by the GPU.
            let scale = viewport.scale.max(1.0);
            (
                ((rect[2] * scale).ceil() as usize).max(1),
                ((rect[3] * scale).ceil() as usize).max(1),
                [
                    f64::from(rect[2]) * state.camera.zoom,
                    f64::from(rect[3]) * state.camera.zoom,
                ],
            )
        } else {
            let size = fitted_minimap_size(game,settings,viewport.width,viewport.height)
                .unwrap_or(128.0);
            let sampling = if game.map_option("minimap_safe_mode") == Some(1) { 1. } else { 4. };
            let texture = (size * sampling).round().clamp(64.0, 1024.0) as usize;
            let cave = matches!(state.layers.get(&selected),Some(Layer::Below(_))) || settings.cave_view == CaveView::Manual;
            let mut zoom = f64::from(settings.blocks_per_pixel);
            if zoom >= 4. && game.map_option("minimap_further_zoomout") == Some(1) {
                zoom *= 2.;
            }
            if cave {
                zoom /= game.map_number("minimap_cave_zoom").unwrap_or(1.);
            }
            if quick_minimap() {
                let setting = game.map_number("quick_minimap_zoom").unwrap_or_default();
                if setting > 0. { zoom = 1. / setting; }
            }
            (
                texture,
                texture,
                [zoom * 128.0; 2],
            )
        };
        let selected_layer = if !cave_allowed(game,large,selected) || override_mode==0 {Layer::Surface}
            else if cave_view == CaveView::Manual {
                Layer::Below(if large && game.map_option("world_cave_type") == Some(2) { maps::FULL_CAVE } else { settings.cave_y })
            }
            else {state.layers.get(&selected).copied().unwrap_or(Layer::Surface)};
        let view = View {
            terrain_style: maps::TerrainStyle {
                appearance,
                live_chunks_only: !large && game.map_option("minimap_world_chunks") == Some(0),
                cpu_raster: !large && game.map_option("minimap_safe_mode") == Some(1),
                slopes:game.map_option(if large {"world_terrain_slopes"}else{"minimap_terrain_slopes"}).unwrap_or(2),
                depth:game.map_option(if large {"world_terrain_depth"}else{"minimap_terrain_depth"})!=Some(0),
                cave_depth:game.map_option(if large {"world_cave_depth"}else{"minimap_cave_depth"}).unwrap_or(0),
                legible_caves:game.map_option(if large {"world_legible_caves"}else{"minimap_legible_caves"})==Some(1),
            },
            session: state.session,
            dimension: selected,
            layer: selected_layer,
            center: state.camera.center,
            span,
            width,
            height,
            angle: if !large && (settings.rotate_with_player || quick_minimap() && game.map_option("minimap_unlocked_enlarged")==Some(1)) && selected == dimension {
                (f64::from(player.yaw) + 180.0).to_radians()
            } else {
                0.0
            },
            round: !large && settings.round,
            large,
        };
        let mut target = view.clone();
        target.center = state.camera.target;
        if large {
            let ratio = state.camera.zoom_target / state.camera.zoom;
            target.span = span.map(|size| size * ratio);
        }
        controller.maps.request_ahead(view.clone(), target);
        current_view = Some(view);
    }
    if let Some(image) = controller.maps.frame()
        && image.view.session == state.session
        && image.view.dimension == selected
        && image.view.large == large
    {
        if current_view
            .as_ref()
            .is_some_and(|view| view.layer != image.view.layer)
        {
            return;
        }
        // Camera movement is independent of worker sampling/upload latency.
        // Pointer queries and overlays use this exact same displayed view.
        let next = if let (Some(atlas), Some(view)) = (&image.atlas, current_view.clone()) {
            let view = complete_map_view(view, &image.view);
            Arc::new(maps::Frame {
                raster_source: None,
                serial: image.serial,
                view,
                pixels: Vec::new(),
                known: image.known,
                atlas: Some(atlas.clone()),
            })
        } else if let Some(view) = current_view.filter(|view| view.large) {
            Arc::new(maps::Frame {
                raster_source: Some(image.clone()),
                serial: image.serial,
                view: complete_map_view(view, &image.view),
                pixels: Vec::new(),
                known: image.known,
                atlas: None,
            })
        } else {
            image
        };
        let paged = next.atlas.as_ref().is_some_and(|a| !a.pages.is_empty());
        if paged {
            let blur = !large && game.map_option("minimap_antialias") == Some(1);
            let _timing = runtime::profiling::measure("page-upload");
            state.page_images = next
                .atlas
                .as_ref()
                .unwrap()
                .pages
                .iter()
                .map(|page| {
                    let image = state
                        .page_images
                        .iter()
                        .find(|(old, image)| Arc::ptr_eq(old, page) && image.flags.blur == blur)
                        .map(|(_, image)| image.clone())
                        .unwrap_or_else(|| {
                            runtime::profiling::record(
                                "texture-transfer",
                                Duration::ZERO,
                                1,
                                page.pixels.len() as u64,
                            );
                            super::minimap_pages::with_filter(
                                page.width as u32,
                                page.height as u32,
                                page.pixels.clone(),
                                blur,
                            )
                        });
                    (page.clone(), image)
                })
                .collect();
        } else {
            state.page_images.clear();
        }
        let previous = if next.atlas.is_some() {
            state.frame.as_ref().and_then(|f| f.atlas.clone())
        } else {
            None
        };
        maps::present_frame(&mut state.frame, next, |width, height, pixels| {
            if paged {
                true
            } else if let Some(previous) = &previous {
                upload_terrain_patches(game, previous, width, height, pixels)
            } else {
                game.mod_image(width, height, pixels)
            }
        });
    }
    refresh_waypoint_labels(&mut state, controller.settings().waypoint_letters);
}

static QUICK_MINIMAP: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
fn quick_minimap() -> bool {
    QUICK_MINIMAP.load(std::sync::atomic::Ordering::Relaxed) != 0
}

pub(super) fn quick_minimap_key(action: mod_api::KeyAction, playing: bool, toggle: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    match action {
        mod_api::KeyAction::Press if playing => {
            let current = QUICK_MINIMAP.load(Relaxed);
            QUICK_MINIMAP.store(if toggle { if current == 2 {0} else {2} } else {1},Relaxed);
        }
        mod_api::KeyAction::Release => { let _ = QUICK_MINIMAP.compare_exchange(1,0,Relaxed,Relaxed); }
        _ => {}
    }
}

fn minimap_ui_scale(game: &Game<'_>) -> f32 {
    let scale = game.map_option("minimap_ui_scale").unwrap_or(1);
    // Xaero's first choice follows the current GUI scale.
    if scale <= 1 { 1. } else { (f32::from(scale) - 1.) * 0.25 + 0.5 }
}

fn fitted_minimap_size(game: &Game<'_>, mut settings: runtime::minimap::Settings, width: f32, height: f32) -> Option<f32> {
    if let Some(size) = game.map_option("minimap_exact_size").filter(|n| *n!=0) {
        settings.size_px = u16::from(size.max(16));
    }
    let regular = settings.fitted_size(width,height)?;
    if quick_minimap() { Some(regular.max((width.min(height) * 0.52).min(440.))) }
    else { Some(regular) }
}

fn missing_item<'a>(game:&Game<'_>,state:&'a State,large:bool)->Option<&'a str> {
    let required=state.map_items[usize::from(large)].as_deref()?;
    let slots=game.inventory.iter().take(9).chain(std::iter::once(&game.offhand)).chain(game.equipment.iter());
    (!slots.flatten().any(|stack| stack.item.as_ref()==required && stack.count>0)).then_some(required)
}

pub(super) fn toggle_slime_overlay() {
    let mut state = state();
    state.slime_overlay = !state.slime_overlay;
}

pub(super) fn check_map_item(game:&mut Game<'_>,large:bool)->bool {
    let state=state();
    if let Some(item)=missing_item(game,&state,large) {
        game.native_chat(&localized_format!(
            "Map needs {} in the hotbar","Для карты нужен {} на панели быстрого доступа",
            "La carte requiert {} dans la barre rapide","El mapa requiere {} en la barra rápida",
            "Für die Karte wird {} in der Schnellleiste benötigt",item
        ));
        return false;
    }
    true
}

type InfoLine = (String, Option<u32>, Option<u32>);
fn claim_text(game: &Game<'_>, state: &State, key: ::map::map_claims::Chunk) -> Option<String> {
    let prefix = tr("Claim", "Приват", "Claim", "Parcela", "Claim");
    let own = tr("yours", "твой", "à toi", "tuya", "dein");
    if let Some(owner) = state.claims.get(&key) {
        return Some(format!("{prefix} · {}",owner.as_deref().unwrap_or(own)));
    }
    let (owner, name) = state.opac.owner(key)?;
    Some(format!("{prefix} · {}", if owner == game.own_uuid() { own }
        else { name.unwrap_or(tr("claimed", "занят", "protégé", "ocupada", "geschützt")) }))
}

fn minimap_lines(game: &Game<'_>, state: &State, settings: runtime::minimap::Settings) -> Vec<InfoLine> {
    let Some(player) = game.native_player() else { return Vec::new(); };
    let dimension = Dimension::from_id(player.dimension);
    let position = [player.x, player.y, player.z];
    let block = position.map(|value| value.floor() as i32);
    let chunk = (dimension, block[0].div_euclid(16), block[2].div_euclid(16));
    let claim = (game.map_option("minimap_current_claim") != Some(0)).then(|| claim_text(game,state,chunk)).flatten();
    let mut lines = Vec::new();
    let Some(config) = state.info_displays.as_ref() else {
        // One fact per line: here, then (when asked) the same place in the other dimension.
        if settings.overworld_coordinates {
            lines.push((format!("{}, {}, {}", block[0], block[1], block[2]), None, None));
        }
        let other = if dimension == Dimension::Nether { Dimension::Overworld } else { Dimension::Nether };
        if settings.nether_coordinates
            && let Some([x, _, z]) = runtime::portal_coordinates(dimension, other, position)
        {
            lines.push((format!("{}: {x}, {z}", other.label()), None, None));
        }
        lines.extend(claim.into_iter().map(|text| (text,None,None)));
        return lines;
    };
    for entry in config {
        let text = match entry.id.as_str() {
            "coords" => format!("X {}  Y {}  Z {}", block[0], block[1], block[2]),
            "overworld_coords" if dimension == Dimension::Nether => {
                format!("{} · X {}  Z {}", Dimension::Overworld.label(), block[0] * 8, block[2] * 8)
            }
            "chunk_coords" => format!("{} · X {}  Z {}",tr("Chunk","Чанк","Chunk","Chunk","Chunk"),chunk.1,chunk.2),
            "angles" => format!("{:.1}° / {:.1}°",player.yaw,player.pitch),
            "dimension" => dimension.label().to_owned(),
            "biome" => known_biome(game,dimension,block).unwrap_or_default(),
            "weather" if game.world.weather.thunder_level.1 > 0. => tr("Thunder", "Гроза", "Orage", "Tormenta", "Gewitter").into(),
            "weather" if game.world.weather.rain_level.1 > 0. => tr("Rain", "Дождь", "Pluie", "Lluvia", "Regen").into(),
            "light_level" => game.map_light_if_loaded(block).map_or_else(String::new,|(block,sky)| {
                format!("{} · {block}/{sky}",tr("Light","Свет","Lumière","Luz","Licht"))
            }),
            "time" => {
                let minute = ((game.world.day_time.rem_euclid(24_000) + 6000) * 60 / 1000) % 1440;
                format!("{:02}:{:02}", minute / 60, minute % 60)
            }
            "real_time" => {
                let minute = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default().as_secs() / 60 % 1440;
                format!("{:02}:{:02} UTC", minute / 60, minute % 60)
            }
            "light_overlay_indicator" if game.map_option("light_overlay") == Some(1) => tr("Light overlay", "Освещение", "Lumière", "Luz", "Licht").into(),
            "manual_cave_mode_indicator" if settings.cave_view == runtime::minimap::CaveView::Manual => format!("Y {}",settings.cave_y),
            "custom_sub_world" => state.waypoint_group.clone().unwrap_or_default(),
            "highlights" => claim.clone().unwrap_or_default(),
            _ => continue,
        };
        if !text.is_empty() {
            lines.push((text, entry.foreground, entry.background));
        }
    }
    if !config.iter().any(|entry| entry.id == "highlights") {
        lines.extend(claim.into_iter().map(|text| (text,None,None)));
    }
    lines
}

pub(super) fn hud(game: &Game<'_>, batch: &mut HudBatch, corners: &mut Corners) {
    if game.map_option("minimap_hud_layer").unwrap_or(1) == 1 {
        draw_hud(game, batch, corners);
    }
}

fn effect_height(game: &Game<'_>) -> f32 {
    let effects = &game.potions.effects.list;
    if effects.is_empty() || game.screen != super::Screen::Playing || game.pvp_effects_shown() {
        return 0.;
    }
    if effects.iter().any(|e| horizon_client::brewing::effect(e.id)
        .is_some_and(|d| d.category != horizon_client::brewing::Category::Beneficial))
    { 50. } else { 25. }
}

pub(super) fn hud_under(game: &Game<'_>, batch: &mut HudBatch) {
    if game.map_option("minimap_hud_layer") == Some(0) {
        draw_hud(game, batch, &mut Corners::new(batch.width,batch.height,effect_height(game)));
    }
}

fn draw_hud(game: &Game<'_>, batch: &mut HudBatch, corners: &mut Corners) {
    if (game.debug && game.map_option("minimap_hide_in_debug")==Some(1))
        || (game.screen!=super::Screen::Playing && game.map_option("minimap_hide_screens")==Some(1))
        || !game.native_window()
        || !game.map_on(runtime::MOD_ID)
        || game.native_player().is_none()
        || game.mod_screen().is_some()
    {
        return;
    }
    let settings = runtime::initialize(game.native_data_dir(runtime::MOD_ID)).settings();
    if missing_item(game,&state(),false).is_some() {return;}
    if game.native_minimap_now().is_some() || !settings.enabled {
        return;
    }
    let Some(side) = fitted_minimap_size(game,settings,batch.width,batch.height) else { return };
    // Corners are logical: a right-to-left language mirrors them.
    let rtl = runtime::right_to_left();
    let corner = match (settings.corner, rtl) {
        (runtime::minimap::Corner::TopLeft, false) | (runtime::minimap::Corner::TopRight, true) => Corner::TopLeft,
        (runtime::minimap::Corner::TopRight, false) | (runtime::minimap::Corner::TopLeft, true) => Corner::TopRight,
        (runtime::minimap::Corner::BottomLeft, false) | (runtime::minimap::Corner::BottomRight, true) => Corner::BottomLeft,
        (runtime::minimap::Corner::BottomRight, false) | (runtime::minimap::Corner::BottomLeft, true) => Corner::BottomRight,
    };
    if !game.native_corner_shown(corner) {
        return;
    }
    let state = state();
    let lines = minimap_lines(game, &state, settings);
    let outer = side + 2. * look::FRAME;
    let height = look::minimap_height(side, lines.len());
    // `Corners` keeps two pixels from the edge; the frame keeps MAP_MARGIN.
    let pad = look::MAP_MARGIN - 2.;
    let (x, mut y) = corners.place(corner, outer + pad, height + pad);
    let x = if matches!(corner, Corner::TopLeft | Corner::BottomLeft) { x + pad } else { x };
    if matches!(corner, Corner::TopLeft | Corner::TopRight) { y += pad; }
    let effects = game.map_option("minimap_effect_push").unwrap_or(1);
    if corner == Corner::TopRight && effects == 0 {
        y -= effect_height(game);
    } else if corner == Corner::BottomRight && effects == 2 {
        y -= effect_height(game);
    }
    let boss = game.map_option("minimap_boss_push").unwrap_or(1);
    if boss != 0 {
        let bars = game.boss_overlay.bars(horizon_client::boss_overlay::now_millis()).len();
        let bottom = horizon_client::hud_overlays::boss_bars_bottom(batch.height as i32,bars) as f32;
        let overlaps = x < batch.width*0.5+92. && x+outer > batch.width*0.5-92.;
        if overlaps && bottom > 0. {
            if boss == 2 { y += bottom; }
            else if y < bottom { y = bottom + 2.; }
        }
    }
    let centered = quick_minimap() && game.map_option("quick_minimap_centered") == Some(1);
    let (x, y) = if centered { ((batch.width-outer)*0.5,(batch.height-height)*0.5) } else { (x, y) };
    let map = [x + look::FRAME, y + look::FRAME, side, side];
    let first_vertex = batch.vertices.len();
    draw(game, batch, &state, map, false, None);
    draw_minimap_frame(game, batch, map, settings.round);
    if let Some(frame)=state.frame.as_ref().filter(|frame| !frame.view.large &&
        frame.view.dimension==state.selected && frame.view.session==state.session) {
        draw_radar(game,batch,&frame.view,map,settings.round,true);
        draw_minimap_waypoints(game,batch,&state,&frame.view,map,settings.round);
        draw_team(game,batch,&state,&frame.view,map,settings.round);
        if let Some(player) = game.native_player() {
            draw_player(game,batch,&frame.view,map,state.marker,&player,settings.rotate_with_player);
        }
        draw_compass(game,batch,&frame.view,map,settings.round,&settings);
    }
    set_opacity(batch, first_vertex, game.map_option("minimap_opacity").unwrap_or(100));
    let widths: Vec<f32> = lines.iter().map(|(line, _, _)| game.map_text_width(line)).collect();
    for ((line, fg, bg), rect) in lines.iter().zip(look::info_lines(map, &widths)) {
        let left = rect[0].clamp(2., (batch.width - rect[2] - 2.).max(2.));
        if let Some(color) = bg {
            let opacity = u32::from(game.map_option("minimap_info_background").unwrap_or(40).min(100));
            game.native_fill(batch, [left - 1., rect[1] - 1., rect[2] + 2., rect[3] + 1.],
                (opacity * 255 / 100) << 24 | color);
        }
        draw_text(game, batch, line, [left, rect[1]], fg.map_or(look::INFO_TEXT, |rgb| 0xff00_0000 | rgb), true);
    }
}

pub(super) fn gui_render_post(
    game: &Game<'_>,
    batch: &mut HudBatch,
    gui_hidden: bool,
    focused: bool,
) {
    if gui_hidden
        || !game.native_window()
        || !game.map_on(runtime::MOD_ID)
        || game.mod_screen().is_some()
        || game.screen != crate::game::Screen::Playing
        || game.native_player().is_none()
    {
        return;
    }
    if game.map_option("minimap_hud_layer").unwrap_or(1) > 1 {
        draw_hud(game, batch, &mut Corners::new(batch.width,batch.height,effect_height(game)));
    }
    if !focused { return; }
    let state = state();
    if missing_item(game,&state,false).is_some() {return;}
    let settings = runtime::initialize(game.native_data_dir(runtime::MOD_ID)).settings();
    draw_world_waypoints(game, batch, &state, &settings);
    draw_tracked_in_world(game,batch);
}

fn direction_label(style: runtime::minimap::DirectionStyle, index: usize) -> &'static str {
    match style {
        runtime::minimap::DirectionStyle::Compass => match index {
            0 => tr("N", "С", "N", "N", "N"),
            1 => tr("NE", "СВ", "NE", "NE", "NO"),
            2 => tr("E", "В", "E", "E", "O"),
            3 => tr("SE", "ЮВ", "SE", "SE", "SO"),
            4 => tr("S", "Ю", "S", "S", "S"),
            5 => tr("SW", "ЮЗ", "SO", "SO", "SW"),
            6 => tr("W", "З", "O", "O", "W"),
            7 => tr("NW", "СЗ", "NO", "NO", "NW"),
            _ => "",
        },
        runtime::minimap::DirectionStyle::Axes => match index {
            0 => "Z-",
            1 => "X+Z-",
            2 => "X+",
            3 => "X+Z+",
            4 => "Z+",
            5 => "X-Z+",
            6 => "X-",
            7 => "X-Z-",
            _ => "",
        },
    }
}

/// Text at a fraction of the font's size with its top-left at `at`, and its own shadow one
/// font pixel down and right (so one screen pixel at half size).
fn small_text(game: &Game<'_>, batch: &mut HudBatch, text: &str, at: [f32; 2], scale: f32, color: u32, shadow: Option<u32>) {
    let first = batch.vertices.len();
    if let Some(shadow) = shadow {
        game.map_text(batch, text, [at[0] + 1., at[1] + 1.], shadow, false);
    }
    game.map_text(batch, text, at, color, false);
    batch.scale_since(first, at[0], at[1], scale, scale);
}

/// N E S W riding the inside of the frame on their blue boxes, always upright.
fn draw_compass(
    game: &Game<'_>,
    batch: &mut HudBatch,
    view: &View,
    map: [f32; 4],
    round: bool,
    settings: &runtime::minimap::Settings,
) {
    if !settings.directions || game.map_option("minimap_compass_location") == Some(0) {
        return;
    }
    for (index, (_, rect)) in look::compass_boxes(map, view.angle as f32, round).into_iter().enumerate() {
        let label = direction_label(settings.direction_style, index * 2);
        let width = game.map_text_width(label) * look::COMPASS_FONT;
        let centre = [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5];
        let side = rect[2].max(width + 1.);
        game.native_fill(batch, [centre[0] - side * 0.5, rect[1], side, rect[3]], look::COMPASS_BOX);
        small_text(game, batch, label, [centre[0] - width * 0.5, centre[1] - 2.], look::COMPASS_FONT,
            look::COMPASS_TEXT, Some(look::COMPASS_SHADOW));
    }
}

/// The player: a crosshair on a map that turns with the player, the red notched arrow when
/// north is locked (and always on the world map).
fn draw_player(
    game: &Game<'_>,
    batch: &mut HudBatch,
    view: &View,
    map: [f32; 4],
    at: [f64; 2],
    player: &crate::game::native_hooks::NativePlayer,
    rotating: bool,
) {
    let [x, y] = world_screen(view, map, at);
    if !inside([x, y], map) {
        return;
    }
    let same_dimension = Dimension::from_id(player.dimension) == view.dimension;
    let opacity = if view.large { 255 } else {
        u32::from(game.map_option("minimap_arrow_opacity").unwrap_or(100)) * 255 / 100
    };
    let alpha = if same_dimension { opacity } else { opacity * 3 / 4 };
    let tint = |argb: u32| (((argb >> 24) * alpha / 255) << 24) | (argb & 0x00ff_ffff);
    let mode = if view.large { 2 } else { game.map_option("minimap_marker_mode").unwrap_or(2) };
    if !view.large && (rotating || mode != 2) {
        let [horizontal, vertical, centre] = look::crosshair([x, y]);
        game.native_fill(batch, horizontal, tint(look::CROSSHAIR));
        game.native_fill(batch, vertical, tint(look::CROSSHAIR));
        game.native_fill(batch, centre, tint(look::CROSSHAIR_CENTRE));
        return;
    }
    let key = if view.large { "world_marker_color" } else { "minimap_marker_color" };
    let fill = 0xff00_0000 | game.map_color_option(key).unwrap_or(look::ARROW & 0x00ff_ffff);
    let scale = if view.large { look::ARROW_SCALE } else {
        game.map_number("minimap_arrow_scale").unwrap_or(f64::from(look::ARROW_SCALE)) as f32
    };
    let forward = view.player_heading(f64::from(player.yaw), [map[2], map[3]]);
    let rim = look::arrow([x, y], forward, scale + 0.35);
    let body = look::arrow([x, y], forward, scale);
    for ([tip, left, notch, right], color) in [(rim, tint(look::ARROW_RIM)), (body, tint(fill))] {
        game.native_quad(batch, [tip, left, notch, tip], color);
        game.native_quad(batch, [tip, notch, right, tip], color);
    }
}

/// The minimap's waypoints: squares of their colour with the initial, clamped onto the frame
/// when out of range; a death point is a black square with a skull.
fn draw_minimap_waypoints(
    game: &Game<'_>,
    batch: &mut HudBatch,
    state: &State,
    view: &View,
    map: [f32; 4],
    round: bool,
) {
    if game.map_option("minimap_waypoints") == Some(0) {
        return;
    }
    let alpha = u32::from(game.map_option("waypoint_map_opacity").unwrap_or(90).min(100)) * 255 / 100;
    for (index, waypoint) in state.waypoints.iter().enumerate()
        .filter(|(_, point)| point.dimension == view.dimension && map_waypoint_visible(game, state, point))
    {
        let at = world_screen(view, map, [f64::from(waypoint.x) + 0.5, f64::from(waypoint.z) + 0.5]);
        let square = look::waypoint_square(map, round, at);
        let centre = [square[0] + square[2] * 0.5, square[1] + square[3] * 0.5];
        if waypoint.icon == runtime::WaypointIcon::Skull {
            game.native_fill(batch, square, look::DEATH_FILL);
            draw_skull(game, batch, centre, square[2] * 0.8);
            continue;
        }
        let letter = waypoint_text(state, index, waypoint);
        let width = game.map_text_width(&letter);
        let side = square[2].max(width + 2.);
        let rect = [centre[0] - side * 0.5, square[1], side, square[3]];
        let rgb = if waypoint.color == 0 { 0x00ff_ffff } else { waypoint.color & 0x00ff_ffff };
        game.native_fill(batch, rect, (alpha << 24) | rgb);
        game.map_text(batch, &letter, [(centre[0] - width * 0.5).round(), (centre[1] - 4.).round()], look::INITIAL_TEXT, true);
    }
}

fn upload_terrain_patches(
    game: &mut Game<'_>,
    previous: &maps::TerrainAtlas,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> bool {
    let _timing = runtime::profiling::measure("upload");
    if previous.width != width as usize || previous.height != height as usize {
        return game.mod_image(width, height, pixels);
    }
    let Some(sprite) = game
        .gui
        .atlas
        .sprites
        .get(horizon_client::hud::MOD_IMAGE_SPRITE)
        .copied()
    else {
        return false;
    };
    if width > sprite.w || height > sprite.h || pixels.len() != width as usize * height as usize * 4
    {
        return false;
    }
    let origin = [
        (sprite.u0 * game.gui.atlas.width as f32).round() as u32,
        (sprite.v0 * game.gui.atlas.height as f32).round() as u32,
    ];
    for [x, y, w, h] in
        maps::image_patch_regions(&previous.pixels, pixels, width as usize, height as usize)
    {
        runtime::profiling::record("texture-transfer", Duration::ZERO, 1, (w * h * 4) as u64);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in y..y + h {
            let start = (row * width as usize + x) * 4;
            rgba.extend_from_slice(&pixels[start..start + w * 4]);
        }
        game.map_patches.push(horizon_client::font::Patch {
            x: origin[0] + x as u32,
            y: origin[1] + y as u32,
            width: w as u32,
            height: h as u32,
            rgba,
        });
    }
    true
}

fn draw_terrain_polygon(
    game: &Game<'_>,
    batch: &mut HudBatch,
    view: &View,
    rect: [f32; 4],
    bounds: [i64; 4],
    uv: [f32; 4],
) -> bool {
    let points = maps::terrain_polygon(view, bounds, uv);
    let Some(sprite) = game
        .gui
        .atlas
        .sprites
        .get(horizon_client::hud::MOD_IMAGE_SPRITE)
    else {
        return false;
    };
    let pos = |p: [f64; 4]| {
        [
            rect[0] + (p[0] as f32 + 0.5) * rect[2],
            rect[1] + (p[1] as f32 + 0.5) * rect[3],
        ]
    };
    let texture = |p: [f64; 4]| {
        [
            sprite.u0 + p[2] as f32 / sprite.w as f32 * (sprite.u1 - sprite.u0),
            sprite.v0 + p[3] as f32 / sprite.h as f32 * (sprite.v1 - sprite.v0),
        ]
    };
    for i in 1..points.len().saturating_sub(1) {
        let p = [points[0], points[i], points[i + 1], points[i + 1]];
        batch.quad(p.map(pos), p.map(texture), u32::MAX);
    }
    points.len() >= 3
}

fn draw_paged_terrain(
    batch: &mut HudBatch,
    state: &State,
    frame: &maps::Frame,
    atlas: &maps::TerrainAtlas,
    rect: [f32; 4],
) -> bool {
    let mut visible = false;
    // Keep the padded/predicted page set resident on the GPU, including pages
    // just behind the camera. Horizon drops picture textures absent from a batch.
    for (_, image) in &state.page_images {
        if !batch.pictures.iter().any(|old| Arc::ptr_eq(old, image)) {
            batch.pictures.push(image.clone());
        }
    }
    let mut paint = |bounds: [i64; 4], mut uv: [f32; 4]| {
        if frame.view.terrain_style.live_chunks_only
            && (state.dimension != frame.view.dimension
                || bounds[2] != 16 || bounds[3] != 16
                || !state.chunk_keys.contains(&((bounds[0] / 16) as i32, (bounds[1] / 16) as i32)))
        {
            return;
        }
        let page = uv[1] as usize / 64;
        let Some((_, image)) = state.page_images.get(page) else {
            return;
        };
        uv[1] -= (page * 64) as f32;
        if image.flags.blur {
            // Keep linear filtering within this atlas tile.
            for axis in 0..2 {
                let inset = (uv[axis + 2] * 0.5).min(0.5);
                uv[axis] += inset;
                uv[axis + 2] -= inset * 2.;
            }
        }
        let mut points = maps::terrain_polygon(&frame.view, bounds, uv);
        for p in &mut points {
            p[0] = f64::from(rect[0]) + (p[0] + 0.5) * f64::from(rect[2]);
            p[1] = f64::from(rect[1]) + (p[1] + 0.5) * f64::from(rect[3]);
        }
        visible |= points.len() >= 3;
        super::minimap_pages::paint(batch, image, &points);
    };
    for coarse in &atlas.coarse {
        if frame.view.terrain_style.live_chunks_only && coarse.key.level > 0 {
            continue;
        }
        let bounds = coarse.key.bounds();
        for part in &coarse.fragments {
            paint(
                *part,
                [
                    coarse.uv_origin[0] as f32
                        + ((part[0] - bounds[0]) as f64 * 16. / bounds[2] as f64) as f32,
                    coarse.uv_origin[1] as f32
                        + ((part[1] - bounds[1]) as f64 * 16. / bounds[3] as f64) as f32,
                    (part[2] as f64 * 16. / bounds[2] as f64) as f32,
                    (part[3] as f64 * 16. / bounds[3] as f64) as f32,
                ],
            );
        }
    }
    for &i in &atlas.draw_order {
        let chunk = &atlas.chunks[i];
        let slot = atlas.slots[i];
        paint(
            [i64::from(chunk[0]) * 16, i64::from(chunk[1]) * 16, 16, 16],
            [(slot % 64 * 16) as f32, (slot / 64 * 16) as f32, 16., 16.],
        );
    }
    visible
}

fn draw_chunk_grid(game: &Game<'_>, batch: &mut HudBatch, view: &View, rect: [f32; 4]) {
    let scale = (f64::from(rect[2]) / view.span[0]).min(f64::from(rect[3]) / view.span[1]);
    let width=if view.large {1.} else {f32::from(game.map_option("minimap_grid_width").unwrap_or(1))};
    let chunk_color=0x70000000|if view.large {0x525f70} else {game.map_color_option("minimap_grid_color").unwrap_or(0x525f70)};
    for (spacing, color) in [(16,chunk_color),(512,0xd0ffc871)] {
        if scale * f64::from(spacing) < 10.0 {
            continue;
        }
        for line in view.grid_lines(spacing) {
            let [mut from, mut to] = line.map(|point| world_screen(view, rect, point));
            if view.round {
                let center = [rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5];
                let dx = to[0] - from[0];
                let dy = to[1] - from[1];
                let length = dx.hypot(dy);
                if length < f32::EPSILON {
                    continue;
                }
                let [nx, ny] = [dx / length, dy / length];
                let [ox, oy] = [center[0] - from[0], center[1] - from[1]];
                let offset = ox * ny - oy * nx;
                let radius = rect[2].min(rect[3]) * 0.5 - 1.0;
                let inside = radius * radius - offset * offset;
                if inside <= 0.0 {
                    continue;
                }
                let half = inside.sqrt();
                let middle = ox * nx + oy * ny;
                let left = (middle - half).max(0.0);
                let right = (middle + half).min(length);
                if left >= right {
                    continue;
                }
                to = [from[0] + right * nx, from[1] + right * ny];
                from = [from[0] + left * nx, from[1] + left * ny];
            }
            let (x0, x1) = (from[0].min(to[0]), from[0].max(to[0]));
            let (y0, y1) = (from[1].min(to[1]), from[1].max(to[1]));
            if (x1 - x0) < 0.01 && x0 >= rect[0] && x0 <= rect[0] + rect[2] - 1.0 {
                let start = y0.max(rect[1]);
                let end = y1.min(rect[1] + rect[3]);
                if end > start {
                    game.native_fill(batch, [x0.floor(),start,width.min(rect[0]+rect[2]-x0),end-start], color);
                }
            } else if (y1 - y0) < 0.01 && y0 >= rect[1] && y0 <= rect[1] + rect[3] - 1.0 {
                let start = x0.max(rect[0]);
                let end = x1.min(rect[0] + rect[2]);
                if end > start {
                    game.native_fill(batch, [start,y0.floor(),end-start,width.min(rect[1]+rect[3]-y0)], color);
                }
            } else {
                draw_line(game, batch, from, to, rect, color);
            }
        }
    }
}

pub(super) fn tick_radar(game: &Game<'_>) {
    use ::map::radar::{self, Facts};
    if !game.native_window() || game.is_logical_server() || !game.map_on(runtime::MOD_ID) {
        return;
    }
    let setting = |key: &str| game.map_number(key).or_else(|| game.map_option(key).map(f64::from))
        .unwrap_or_else(|| match key {
            "radar" | "radar_icons" | "radar_world_map" | "radar_smooth" | "radar_hide_invisible" | "radar_players" | "radar_mobs" | "radar_items" | "radar_other" | "radar_name_scale" => 1.,
            "radar_range" => 64.,
            "radar_height" => 20.,
            "radar_dot_size" => 2.,
            "radar_icon_size" => 8.,
            _ => 0.,
        });
    if !radar::sample_due(setting("radar")>0.) {return;}
    let mobs: horizon_client::fast_hash::FxHashMap<_,_> = game.mobs.iter().map(|mob| (mob.id,mob)).collect();
    let items: horizon_client::fast_hash::FxHashMap<_,_> = game.dropped.iter().map(|item| (item.id,item.stack.item.as_ref())).collect();
    // The group of an entity from what the game knows it as: a player of the session, one of its
    // mobs, one of its dropped items, or anything else. A mod's entities need nothing here.
    let player = |id: u32| game.remote.as_ref().and_then(|remote| remote.player_uuid(id)).is_some();
    let group = |id: u32| if player(id) { radar::PLAYER } else if items.contains_key(&id) { radar::ITEM }
        else if mobs.contains_key(&id) { radar::MOB } else { radar::OTHER };
    if !radar::uses_categories() {
        radar::update(game, setting, group, |_,_,_| Facts::default());
        return;
    }
    let own_team = game.player_team().map(|team| team.name.as_str());
    let own_uuid = game.own_uuid();
    let cave = game.native_player().is_some_and(|player| {
        matches!(state().layers.get(&Dimension::from_id(player.dimension)),Some(Layer::Below(_)))
    });
    let tracked = LOCATOR_WAYPOINTS.lock().unwrap_or_else(|error| error.into_inner());
    radar::update(game,setting,group,|id,_kind,at| {
        let mob = mobs.get(&id).copied();
        let team = mob.and_then(|mob| game.mob_team_of(mob)).or_else(|| game.mob_team(id));
        let name = team.map(|team| team.name.as_str());
        let baby = mob.is_some_and(|mob| mob.is_baby());
        let is_tracked = game.remote.as_ref()
            .and_then(|remote| remote.player_uuid(id))
            .is_some_and(|uuid| tracked.iter().any(|point| point.id == WaypointId::Uuid(uuid)));
        Facts {
            item: items.get(&id).copied(),
            living: mob.is_some() || player(id) || game.armor_stands.iter().any(|stand| stand.id == id),
            custom_name: mob.and_then(|mob| mob.custom_name.as_deref()).map(std::borrow::Cow::Borrowed)
                .or_else(|| game.armor_stands.iter().find(|stand| stand.id==id)?.custom_name.as_ref()
                    .map(|name| std::borrow::Cow::Owned(name.plain()))),
            baby,
            hostile: game.difficulty_id()>0 && mob.is_some_and(|mob| mob.category()=="monster"),
            tamed: mob.is_some_and(|mob| {
                let owned=|owner:[i32;4]| owner.into_iter().fold(0u128,|id,word|(id<<32)|u128::from(word as u32))==own_uuid;
                if horizon_client::mobs::equine::is_equine(mob.species()) {
                    mob.extra.tame
                } else if mob.species() == horizon_client::mobs::Species::Fox {
                    horizon_client::mobs::fox::trusted_entities(mob).any(owned)
                } else {
                    mob.extra.owner.is_some_and(owned)
                }
            }),
            same_team: name==own_team,
            team: name,
            team_color: team.and_then(|team| team.color.as_deref()).and_then(horizon_client::text::named_color),
            light: game.map_light_if_loaded(at.map(|axis| axis.floor() as i32)),
            cave,
            tracked: is_tracked,
        }
    });
}

/// Team's players' skins by name, from the game's player list: the faces Map draws for
/// teammates whose body is out of sight. Refreshed from the list every second.
fn refresh_team_skins(game: &mut Game<'_>) {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|at| at.elapsed() < Duration::from_secs(1)) {
            return;
        }
        *last = Some(Instant::now());
    }
    let Some((players, _, _)) = game.remote_tab_players() else { return };
    let skins: BTreeMap<String, String> = players
        .into_iter()
        .map(|player| {
            let skin = game.face_skin(player.uuid, &player.skin);
            (player.name, skin)
        })
        .collect();
    state().team_skins = skins;
}

/// A teammate's face: their skin with its hat on a frame of their Team colour, so a teammate
/// reads apart from other players; the default skin's face until the game knows theirs.
fn draw_mate(game: &Game<'_>, batch: &mut HudBatch, mate: &::map::team_view::Mate, skin: Option<&str>, at: [f32; 2]) {
    let face = look::head_rect(at, [8., 8.], 1.);
    let frame = look::head_rect(at, [10., 10.], 1.);
    game.native_fill(batch, frame, 0xff00_0000 | (mate.colour & 0x00ff_ffff));
    let layers = [[8., 8., 8., 8.], [40., 8., 8., 8.]];
    let drawn = skin.is_some_and(|skin| layers.iter().all(|src| game.native_sprite(batch, skin, *src, face, None)));
    if !drawn {
        for src in layers {
            game.native_sprite(batch, "entity/player/wide/steve", src, face, None);
        }
    }
}

/// A ping's pennant in its owner's colour: a sword for danger, the block for loot, else the
/// owner's initial.
fn draw_ping(game: &Game<'_>, batch: &mut HudBatch, ping: &::map::team_view::Ping, at: [f32; 2], scale: f32) {
    let (body, [left, right, tip]) = look::pennant(at, scale);
    let o = look::PENNANT_SHADOW_OFFSET * scale;
    let shift = |p: [f32; 2]| [p[0] + o, p[1] + o];
    game.native_fill(batch, [body[0] + o, body[1] + o, body[2], body[3]], look::PENNANT_SHADOW);
    game.native_quad(batch, [shift(left), shift(right), shift(tip), shift(left)], look::PENNANT_SHADOW);
    let fill = 0xff00_0000 | (ping.colour & 0x00ff_ffff);
    game.native_fill(batch, body, fill);
    game.native_quad(batch, [left, right, tip, left], fill);
    let centre = [body[0] + body[2] * 0.5, body[1] + body[3] * 0.5];
    use ::map::team_view::Mark;
    if ping.mark == Mark::Danger {
        let rows = look::DANGER_GLYPH;
        let cell = [body[0], body[1], body[2], body[3]];
        draw_glyph(game, batch, rows, cell, look::GLYPH);
        return;
    }
    if ping.mark == Mark::Loot
        && let Some(block) = ping.block.as_deref()
    {
        let side = body[2] - 4. * scale;
        let stack = horizon_client::items::ItemStack::new(block, 1);
        if game.native_item_stack(batch, &stack, [centre[0] - side * 0.5, centre[1] - side * 0.5], side) {
            return;
        }
    }
    let letter = first_letter(&ping.owner).to_string();
    let width = game.map_text_width(&letter) * scale;
    let origin = [(centre[0] - width * 0.5).round(), (centre[1] - 4. * scale).round()];
    let first = batch.vertices.len();
    game.map_text(batch, &letter, origin, look::INITIAL_TEXT, true);
    batch.scale_since(first, origin[0], origin[1], scale, scale);
}

/// Team on a map: each teammate's face (on the minimap clamped onto the frame when out of
/// range, as waypoints are) and each live ping's pennant. A teammate the radar already shows
/// is not drawn twice.
fn draw_team(game: &Game<'_>, batch: &mut HudBatch, state: &State, view: &View, rect: [f32; 4], round: bool) {
    if !game.map_on("team") {
        return;
    }
    let dimension = view.dimension.id();
    let shown_by_radar: Vec<String> = ::map::radar::snapshot()
        .filter(|radar| radar.dimension == dimension && (!view.large || radar.world_map))
        .map(|radar| radar.dots.iter().filter_map(|dot| game.remote_player_name(dot.entity_id).map(str::to_owned)).collect())
        .unwrap_or_default();
    let place = |at: [f64; 3]| {
        let point = world_screen(view, rect, [at[0], at[2]]);
        if view.large {
            look::on_map(rect, false, point).then_some(point)
        } else {
            let square = look::waypoint_square(rect, round, point);
            Some([square[0] + square[2] * 0.5, square[1] + square[3] * 0.5])
        }
    };
    for ping in ::map::team_view::pings().iter().filter(|ping| Dimension::from_id(&ping.dimension) == view.dimension) {
        if let Some(at) = place(ping.at) {
            let scale = if view.large { 1. } else { 0.6 };
            // A pennant's tip marks the spot; on the minimap the clamped square's centre does.
            draw_ping(game, batch, ping, at, scale);
        }
    }
    for mate in ::map::team_view::mates().iter().filter(|mate| Dimension::from_id(&mate.dimension) == view.dimension) {
        let point = world_screen(view, rect, [mate.at[0], mate.at[2]]);
        if shown_by_radar.iter().any(|name| *name == mate.name) && look::on_map(rect, round, point) {
            continue;
        }
        if let Some(at) = place(mate.at) {
            draw_mate(game, batch, mate, state.team_skins.get(&mate.name).map(String::as_str), at);
        }
    }
}

/// What kind of thing a radar dot is, for the draw order.
fn radar_kind(dot: &::map::radar::Dot, mob: Option<&horizon_client::mobs::Mob>) -> look::Kind {
    match dot.group as usize {
        ::map::radar::PLAYER => look::Kind::Player,
        ::map::radar::ITEM => look::Kind::Item,
        _ => match mob {
            Some(mob) if mob.category() == "monster" => look::Kind::Hostile,
            Some(_) => look::Kind::Passive,
            None => look::Kind::Item,
        },
    }
}

/// A dropped item's head: its item sprite, as a slot draws it.
fn draw_item_head(game: &Game<'_>, batch: &mut HudBatch, dot: &::map::radar::Dot, at: [f32; 2], scale: f32, alpha: u32) -> Option<[f32; 4]> {
    let item = game.dropped.iter().find(|item| item.id == dot.entity_id)?;
    let side = look::ITEM_SIDE * scale;
    let dst = look::head_rect(at, [side, side], 1.);
    let start = batch.vertices.len();
    let drawn = game.native_item_stack(batch, &item.stack, [dst[0], dst[1]], side);
    set_opacity(batch, start, (alpha * 100 / 255) as u8);
    drawn.then_some(dst)
}

/// An entity's head icon, made by the game's own entity drawing: the quads it would draw for
/// the entity in a screen (its model or a mod's renderer, every render layer and every mod
/// layer: eyes, wool, saddles, armour, collars, variants, baby or adult), seen from the front
/// at one GUI pixel per model pixel, kept where the head is. Corners are relative to the
/// head's centre, UVs in the GUI atlas, colours ABGR, far to near.
struct Icon {
    quads: Vec<([[f32; 2]; 4], [[f32; 2]; 4], u32)>,
    size: [f32; 2],
}

/// Icons made so far, by what decides their look (`icon_key`), and how often they were used.
/// A look that gives no icon is kept too (`None`), so it is not tried again every frame.
static ICONS: Mutex<Option<std::collections::HashMap<String, Option<Arc<Icon>>>>> = Mutex::new(None);

/// The most icons kept: every kind and variant on screen at once fits many times over.
const ICONS_KEPT: usize = 256;

/// What decides how an entity's head looks: its type, the textures and tints of its body and
/// every layer the game would draw on it, its age, and a player's skin.
fn icon_key(game: &Game<'_>, kind: &str, mob: &horizon_client::mobs::Mob) -> String {
    let game_atlas_stamp = || format!("|atlas {}x{} {}", game.gui.atlas.width, game.gui.atlas.height, game.gui.atlas.sprites.len());
    use horizon_client::mobs::render;
    let (model, texture) = render::body(mob);
    let mut key = format!("{kind}|{model}|{texture}|{:08x}|{}|{}", render::body_color(mob, 1.), mob.is_baby(), mob.skin.as_deref().unwrap_or(""));
    // A resource reload builds a new atlas: icons (an override's, the generated quads' UVs) follow.
    let atlas = &game_atlas_stamp();
    key.push_str(atlas);
    for layer in render::layers(mob, 0.) {
        key.push_str(&format!("|{}:{}:{:08x}", layer.model, layer.texture, layer.color));
    }
    key
}

/// The part of `model` that is the head: the shallowest part whose name says head (`head`, a
/// horse's `head_parts`), so everything attached to it comes with it. None for a model with
/// no head (a slime): then the whole entity is its icon.
fn head_part(model: &horizon_client::mobs::Part) -> Option<&str> {
    let mut level = vec![model];
    while !level.is_empty() {
        if let Some(part) = level.iter().find(|part| part.name.contains("head")) {
            return Some(part.name.as_str());
        }
        level = level.iter().flat_map(|part| part.children.iter()).collect();
    }
    None
}

/// The entity's head icon, made once per look and kept (`ICONS`).
fn mob_icon(game: &Game<'_>, kind: &str, mob: &horizon_client::mobs::Mob) -> Option<Arc<Icon>> {
    let key = icon_key(game, kind, mob);
    let mut icons = ICONS.lock().unwrap_or_else(|error| error.into_inner());
    let icons = icons.get_or_insert_with(Default::default);
    if let Some(icon) = icons.get(&key) {
        release_atlas_pixels();
        ICON_COST.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        runtime::profiling::record("radar-icon-hit", Duration::ZERO, 1, 0);
        return icon.clone();
    }
    let started = Instant::now();
    let icon = make_icon(game, mob).map(Arc::new);
    runtime::profiling::record("radar-icon-made", started.elapsed(), 1, 0);
    log::debug!("[{}] head icon for {kind} made in {} us ({} kept, {} uses from the cache so far)",
        runtime::MOD_ID, started.elapsed().as_micros(), icons.len() + 1, ICON_COST.hits.load(std::sync::atomic::Ordering::Relaxed));
    if icons.len() >= ICONS_KEPT {
        icons.clear();
    }
    icons.insert(key, icon.clone());
    icon
}

/// Draws `mob` facing the viewer through the game's own entity drawing and keeps its head.
fn make_icon(game: &Game<'_>, mob: &horizon_client::mobs::Mob) -> Option<Icon> {
    use glam::{DVec3, Mat4, Vec3};
    use horizon_client::mobs::render;
    // Posed as a screen poses an entity turned to face the viewer (`GuiEntityLook::Turned`).
    let mut mob = mob.clone();
    mob.previous = mob.position;
    (mob.body_yaw, mob.previous_body_yaw, mob.head_yaw, mob.previous_head_yaw) = (180., 180., 180., 180.);
    (mob.head_pitch, mob.previous_head_pitch) = (0., 0.);
    mob.extra.model_scale = 0.;
    // One GUI pixel per model pixel (16 a block), seen as a screen sees an entity.
    let view = Mat4::from_scale(Vec3::new(16., 16., -16.))
        * Mat4::from_translation(Vec3::new(0., mob.height() / 2., 0.))
        * Mat4::from_rotation_z(std::f32::consts::PI);
    let seen = |verts: &[horizon_client::mesher::Vertex]| -> Vec<(f32, [[f32; 3]; 4], [[f32; 2]; 4], u32)> {
        verts.as_chunks::<4>().0.iter().filter_map(|q| {
            let p: [Vec3; 4] = std::array::from_fn(|i| view.transform_point3(Vec3::from_array(q[i].pos)));
            // The winding gives inward normals: a face with n.z >= 0 looks away.
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            if n.z >= 0. && horizon_client::mesher::culls_back(q[0].normal) {
                return None;
            }
            // A face seen edge-on (a hat brim) shows nothing, and so outlines nothing.
            let span = |axis: usize| p.iter().map(|v| v[axis]).fold(f32::NEG_INFINITY, f32::max) - p.iter().map(|v| v[axis]).fold(f32::INFINITY, f32::min);
            if span(0) < 0.05 || span(1) < 0.05 {
                return None;
            }
            let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.;
            Some((depth, p.map(|v| v.to_array()), std::array::from_fn(|i| q[i].uv), q[0].color))
        }).collect()
    };
    let at = DVec3::new(mob.position.x, mob.position.y, mob.position.z);
    let atlas = &game.gui.atlas;
    let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
    let vanilla = |hidden: mod_api::HiddenParts| {
        let mut out = Vec::new();
        render::render_with_hidden(&mob, 1., at, &lookup, 240 | 240 << 8, game.res.mob_data(), &[], hidden, None, &mut out);
        seen(&out)
    };
    // Where the head is: what the vanilla drawing loses when the head part is hidden.
    let (model, _) = render::body(&mob);
    let model = mob.kind.layer(&model).unwrap_or(&mob.kind.model);
    let whole = vanilla(Default::default());
    let key = |p: &[[f32; 3]; 4]| [p[0][0].to_bits(), p[0][1].to_bits(), p[2][0].to_bits()];
    // Quads by place: a layer drawn over the head (spots, eyes) repeats its places, so what the
    // hidden head loses is counted, not just looked up.
    let mut rest: std::collections::HashMap<[u32; 3], usize> = Default::default();
    let mut lost_keys: std::collections::HashSet<[u32; 3]> = Default::default();
    let mut whole_body = false;
    let head: Vec<_> = match head_part(model) {
        Some(name) => {
            let mut hidden = mod_api::HiddenParts::default();
            if let Err(error) = hidden.hide_named(name) {
                log::warn!("[{}] no head icon for {}: its head part {name} cannot be hidden ({error:?})", runtime::MOD_ID, mob.kind.id);
                return None;
            }
            for (_, p, ..) in vanilla(hidden).iter() {
                *rest.entry(key(p)).or_default() += 1;
            }
            let mut counts: std::collections::HashMap<[u32; 3], usize> = Default::default();
            for (_, p, ..) in &whole {
                *counts.entry(key(p)).or_default() += 1;
            }
            lost_keys = counts.into_iter().filter(|(k, n)| *n > rest.get(k).copied().unwrap_or(0)).map(|(k, _)| k).collect();
            let lost: Vec<_> = whole.iter().filter(|(_, p, ..)| lost_keys.contains(&key(p))).collect();
            // A renderer that draws its head whatever is hidden (a breeze): the whole entity,
            // as for a mob without a head part.
            if lost.is_empty() {
                rest.clear();
                whole_body = true;
                whole.iter().collect()
            } else {
                lost
            }
        }
        None => {
            whole_body = true;
            whole.iter().collect()
        }
    };
    // The face, in model pixels (the model's own scale taken out): everything of the head above
    // its chin, a neck below it never (`look::face_frame`).
    let unit = model.pose[6].abs().max(0.01);
    let rect = |p: &[[f32; 3]; 4]| {
        let (x0, x1) = p.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(v[0]), b.max(v[0])));
        let (y0, y1) = p.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(v[1]), b.max(v[1])));
        [x0 / unit, y0 / unit, (x1 - x0) / unit, (y1 - y0) / unit]
    };
    let Some(frame) = look::face_frame(&head.iter().map(|(z, p, ..)| (*z / unit, rect(p))).collect::<Vec<_>>()) else {
        log::warn!("[{}] no head icon for {}: {} head faces of {} drawn, none a face plate", runtime::MOD_ID, mob.kind.id, head.len(), whole.len());
        return None;
    };
    let chin = (frame[1] + frame[3]) * unit;
    // Everything the game draws for the entity (a mod's renderer and layers too) on that face:
    // its eyes, wool, hat, bridle; a layer may stand a pixel off it (wool).
    let mut all = Vec::new();
    game.gui_entity_geometry(&mob, &mut all);
    let mut quads = seen(&all);
    let margin = unit;
    // The rest of the body counts only where it stands within the head's width and above the
    // chin (wool, a mushroom on a mooshroom's head), never beside it (a sheep's body wool).
    let own = head.iter().flat_map(|(_, p, ..)| p.iter()).fold([f32::INFINITY, f32::NEG_INFINITY],
        |b, v| [b[0].min(v[0]), b[1].max(v[0])]);
    let on_head = |p: &[[f32; 3]; 4]| lost_keys.contains(&key(p)) || !rest.contains_key(&key(p))
        || p.iter().all(|v| v[0] >= own[0] - margin && v[0] <= own[1] + margin && v[1] <= chin + margin);
    // What starts above the chin stays; a neck or a body below it goes.
    let on_face = |p: &[[f32; 3]; 4]| on_head(p) && p.iter().any(|v| v[1] < chin - unit * 0.5);
    quads.retain(|(_, p, ..)| on_face(p));
    if quads.is_empty() {
        log::warn!("[{}] no head icon for {}: nothing drawn on its face", runtime::MOD_ID, mob.kind.id);
        return None;
    }
    quads.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Framed on what was kept, down to the frame's foot (a neck through it is cut there).
    let mut box_ = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    for (_, p, ..) in &quads {
        for v in p {
            box_ = [box_[0].min(v[0]), box_[1].min(v[1]), box_[2].max(v[0]), box_[3].max(v[1])];
        }
    }
    box_[3] = box_[3].min(chin);
    // A body drawn whole (a squid, a ghast) or a head far from square keeps its top square.
    let framed = look::icon_frame([box_[0], box_[1], box_[2] - box_[0], box_[3] - box_[1]], whole_body);
    box_[3] = box_[1] + framed[3];
    let bottom = box_[3];
    quads.retain(|(_, p, ..)| p.iter().any(|v| v[1] < bottom - unit * 0.5));
    // Trimmed to what shows: the opaque texels of every face, inside the frame (a bee's empty
    // wing planes, a model's transparent margins do not count).
    let opaque = opaque_bounds(game, &quads).map(|o| [o[0].max(box_[0]), o[1].max(box_[1]), o[2].min(box_[2]), o[3].min(box_[3])])
        .filter(|o| o[2] > o[0] && o[3] > o[1]);
    if let Some(o) = opaque {
        box_ = o;
    }
    // A wide face keeps its middle, so it fills the box as a square one does.
    let crop = look::icon_crop([box_[0], box_[1], box_[2] - box_[0], box_[3] - box_[1]]);
    box_ = [crop[0], crop[1], crop[0] + crop[2], crop[1] + crop[3]];
    let centre = [(box_[0] + box_[2]) * 0.5, (box_[1] + box_[3]) * 0.5];
    Some(Icon {
        // One texture pixel to one GUI pixel, whatever the model's own scale (a ghast, a villager), as Xaero draws.
        quads: quads.into_iter().map(|(_, p, uv, color)| (p.map(|v| [(v[0] - centre[0]) / unit, (v[1] - centre[1]) / unit]), uv, color)).collect(),
        size: [(box_[2] - box_[0]) / unit, (box_[3] - box_[1]) / unit],
    })
}

/// The GUI atlas's pixels while icons are being made: unpacked once, kept a few seconds, so a
/// burst of new looks unpacks them once (`opaque_bounds`).
static ATLAS_PIXELS: Mutex<Option<(String, Arc<Vec<u8>>, Instant)>> = Mutex::new(None);

/// The bounds `[x0, y0, x1, y1]` of what the quads show: each face's opaque texels mapped back
/// onto it. None when nothing is opaque.
fn opaque_bounds(game: &Game<'_>, quads: &[(f32, [[f32; 3]; 4], [[f32; 2]; 4], u32)]) -> Option<[f32; 4]> {
    let atlas = &game.gui.atlas;
    let stamp = format!("{}x{} {}", atlas.width, atlas.height, atlas.sprites.len());
    let pixels = {
        let mut kept = ATLAS_PIXELS.lock().unwrap_or_else(|error| error.into_inner());
        match kept.as_mut().filter(|(at, ..)| *at == stamp) {
            Some((_, pixels, used)) => {
                *used = Instant::now();
                pixels.clone()
            }
            None => {
                let pixels = Arc::new(atlas.pixels().into_owned());
                *kept = Some((stamp, pixels.clone(), Instant::now()));
                pixels
            }
        }
    };
    let [w, h] = [atlas.width as usize, atlas.height as usize];
    if pixels.len() < w * h * 4 {
        return None;
    }
    let alpha = |uv: [f32; 2]| {
        let x = ((uv[0] * w as f32) as usize).min(w.saturating_sub(1));
        let y = ((uv[1] * h as f32) as usize).min(h.saturating_sub(1));
        pixels[(y * w + x) * 4 + 3] > 25
    };
    let mut bounds: Option<[f32; 4]> = None;
    for (_, p, uv, color) in quads {
        if color >> 24 == 0 {
            continue;
        }
        let texels = |a: [f32; 2], b: [f32; 2]| (((b[0] - a[0]) * w as f32).hypot((b[1] - a[1]) * h as f32).round() as usize).clamp(1, 64);
        let cells = [texels(uv[0], uv[1]), texels(uv[0], uv[3])];
        let at = |s: f32, t: f32| [uv[0][0] + (uv[1][0] - uv[0][0]) * s + (uv[3][0] - uv[0][0]) * t,
            uv[0][1] + (uv[1][1] - uv[0][1]) * s + (uv[3][1] - uv[0][1]) * t];
        let Some([s0, t0, s1, t1]) = look::opaque_span(cells, |i, j| {
            alpha(at((i as f32 + 0.5) / cells[0] as f32, (j as f32 + 0.5) / cells[1] as f32))
        }) else { continue };
        let place = |s: f32, t: f32| [p[0][0] + (p[1][0] - p[0][0]) * s + (p[3][0] - p[0][0]) * t,
            p[0][1] + (p[1][1] - p[0][1]) * s + (p[3][1] - p[0][1]) * t];
        for [x, y] in [place(s0, t0), place(s1, t0), place(s0, t1), place(s1, t1)] {
            bounds = Some(match bounds {
                None => [x, y, x, y],
                Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
            });
        }
    }
    bounds
}

/// Lets go of the atlas's unpacked pixels once no icon has been made for a few seconds.
fn release_atlas_pixels() {
    let mut kept = ATLAS_PIXELS.lock().unwrap_or_else(|error| error.into_inner());
    if kept.as_ref().is_some_and(|(_, _, used)| used.elapsed() > Duration::from_secs(5)) {
        *kept = None;
    }
}

/// An icon at `at`: first its silhouette in black one screen pixel out on each side (the
/// outline follows the drawn alpha), then the icon darkened by `shade`. Returns where it landed.
fn draw_icon(batch: &mut HudBatch, icon: &Icon, at: [f32; 2], scale: f32, alpha: u32, shade: f32) -> [f32; 4] {
    let pixel = 1. / batch.scale.max(1.);
    let place = |p: &[[f32; 2]; 4], [dx, dy]: [f32; 2]| p.map(|v| [at[0] + v[0] * scale + dx, at[1] + v[1] * scale + dy, 0.]);
    let outline = alpha << 24;
    let start = batch.vertices.len();
    for offset in look::outline_offsets(pixel) {
        for (p, uv, _) in &icon.quads {
            batch.quad_3d(place(p, offset), *uv, outline);
        }
    }
    let level = shade.clamp(0., 1.);
    for (p, uv, color) in &icon.quads {
        let channel = |s: u32| ((((color >> s) & 255) as f32 * level).round() as u32) << s;
        let a = ((color >> 24) * alpha / 255) << 24;
        batch.quad_3d(place(p, [0., 0.]), *uv, a | channel(16) | channel(8) | channel(0));
    }
    // A neck that runs on below the face is cut at the frame's foot, outline included.
    let frame = look::head_rect(at, icon.size, scale);
    batch.clip_since(start, frame[0] - pixel, frame[1] - pixel, frame[0] + frame[2] + pixel, frame[1] + frame[3] + pixel);
    frame
}

/// An entity's map icon from a resource pack or a mod (`look::icon_override_names`), drawn in
/// the box of `side` at `at`: its silhouette in black one screen pixel out, then the image.
/// None when no pack gives one.
fn draw_override(game: &Game<'_>, batch: &mut HudBatch, names: &[String; 2], at: [f32; 2], side: f32, alpha: u32, shade: f32) -> Option<[f32; 4]> {
    let pixel = 1. / batch.scale.max(1.);
    let rect = look::head_rect(at, [side, side], 1.);
    let level = (shade.clamp(0., 1.) * 255.).round() as u32;
    let fill = (alpha << 24) | (level << 16) | (level << 8) | level;
    for name in names {
        let start = batch.vertices.len();
        let offsets = look::outline_offsets(pixel);
        let [dx, dy] = offsets[0];
        batch.picture(&game.gui.atlas, name, rect[0] + dx, rect[1] + dy, rect[2], rect[3], alpha << 24);
        if batch.vertices.len() == start {
            continue;
        }
        for [dx, dy] in &offsets[1..] {
            batch.picture(&game.gui.atlas, name, rect[0] + dx, rect[1] + dy, rect[2], rect[3], alpha << 24);
        }
        batch.picture(&game.gui.atlas, name, rect[0], rect[1], rect[2], rect[3], fill);
        return Some(rect);
    }
    None
}

/// The merged face under the pointer on the world map: its tooltip and the face's rectangle.
static RADAR_HOVER: Mutex<Option<(Vec<Vec<(String, u32)>>, [f32; 4])>> = Mutex::new(None);

/// A merged face's tooltip: the kind with its count, then each member's distance, nearest
/// first.
fn group_lines<'a>(game: &Game<'_>, members: impl Iterator<Item = &'a ::map::radar::Dot>) -> Vec<Vec<(String, u32)>> {
    let player = game.native_player();
    let mut members: Vec<(&::map::radar::Dot, f64)> = members.map(|dot| {
        let d = player.as_ref().map_or(0., |p| ((dot.at[0] - p.x).powi(2) + (dot.at[1] - p.y).powi(2) + (dot.at[2] - p.z).powi(2)).sqrt());
        (dot, d)
    }).collect();
    members.sort_by(|a, b| a.1.total_cmp(&b.1));
    let Some((first, _)) = members.first() else { return Vec::new() };
    let key = format!("entity.{}", first.kind.replace(':', "."));
    let name = game.translate(&key).filter(|name| *name != key)
        .unwrap_or_else(|| first.kind.rsplit(':').next().unwrap_or(&first.kind).replace('_', " "));
    let mut lines = vec![vec![(format!("{name} ×{}", members.len()), look::TOOLTIP_TEXT)]];
    lines.extend(members.iter().take(8).map(|(_, d)| vec![(waypoint_distance_text(game, *d), look::GREY_TEXT)]));
    if members.len() > 8 {
        lines.push(vec![(format!("+{}", members.len() - 8), look::GREY_TEXT)]);
    }
    lines
}

/// What the radar's faces cost: uses of a kept icon, and the time and count of radar draws
/// (logged every 3000 draws).
struct IconCost {
    hits: std::sync::atomic::AtomicU64,
    draws: std::sync::atomic::AtomicU64,
    micros: std::sync::atomic::AtomicU64,
}

static ICON_COST: IconCost = IconCost {
    hits: std::sync::atomic::AtomicU64::new(0),
    draws: std::sync::atomic::AtomicU64::new(0),
    micros: std::sync::atomic::AtomicU64::new(0),
};

fn draw_radar(game: &Game<'_>, batch: &mut HudBatch, view: &View, rect: [f32; 4], round: bool, after_frame: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    let started = Instant::now();
    draw_radar_faces(game, batch, view, rect, round, after_frame);
    let micros = ICON_COST.micros.fetch_add(started.elapsed().as_micros() as u64, Relaxed) + started.elapsed().as_micros() as u64;
    let draws = ICON_COST.draws.fetch_add(1, Relaxed) + 1;
    if draws % 3000 == 0 {
        log::debug!("[{}] radar draw: {} us on average over {draws} draws", runtime::MOD_ID, micros / draws);
    }
}

fn draw_radar_faces(game: &Game<'_>, batch: &mut HudBatch, view: &View, rect: [f32; 4], round: bool, after_frame: bool) {
    let Some(radar) = ::map::radar::snapshot() else { return };
    if radar.dimension != view.dimension.id() || (view.large && !radar.world_map) { return; }
    let mobs: horizon_client::fast_hash::FxHashMap<_,_> = game.mobs.iter().map(|mob| (mob.id,mob)).collect();
    // Nearest last within a kind; kinds in a fixed order so the important face is on top.
    let mut dots: Vec<_> = radar.dots.iter().rev()
        .filter(|dot| !view.large && dot.over_frame == after_frame || view.large && !after_frame)
        .map(|dot| (radar_kind(dot, mobs.get(&dot.entity_id).copied()), dot))
        .collect();
    dots.sort_by_key(|(kind, _)| *kind);
    let scale = radar.icon_size / 8.;
    // Where each head goes: its entity's spot, then nudged out of a crowd (`look::declutter`).
    let mut placed = Vec::new();
    for (_, dot) in dots {
        let shade = if dot.depth_shade { look::depth_shade(dot.dy) } else { Some(1.) };
        let Some(shade) = shade else { continue };
        let mut position = dot.at;
        if radar.smooth && let Some(mob) = mobs.get(&dot.entity_id) {
            let current = mob.position;
            let previous = mob.previous;
            let partial = game.clock.camera_fraction();
            position = [
                previous.x+(current.x-previous.x)*partial,
                previous.y+(current.y-previous.y)*partial,
                previous.z+(current.z-previous.z)*partial,
            ];
        }
        let [x,_,z] = position;
        let at = world_screen(view,rect,[x,z]);
        // A face whose box still reaches into the map is drawn, cut by the map's edge (Xaero).
        let item = dot.group as usize == ::map::radar::ITEM;
        let face = (dot.icon && !item).then(|| mobs.get(&dot.entity_id).map(|mob| {
            let (_, texture) = horizon_client::mobs::render::body(mob);
            let variant = texture.rsplit('/').next().unwrap_or(&texture).to_owned();
            (look::icon_override_names(&dot.kind, &variant), mob_icon(game, &dot.kind, mob))
        })).flatten();
        // Each face's side on screen with its outline: every face in the one box.
        let side = match &face {
            Some(_) => look::ICON_BOX * scale * dot.icon_scale + 1.,
            None if dot.icon && item => look::ITEM_SIDE * scale + 1.,
            None if dot.icon => 8. * scale * dot.icon_scale + 1.,
            None => dot.size + 2.,
        };
        if !look::reaches_map(rect, round, at, side * 0.5) { continue; }
        placed.push((dot, at, shade, face, side));
    }
    let anchors: Vec<[f32; 2]> = placed.iter().map(|(_, at, ..)| *at).collect();
    let sizes: Vec<f32> = placed.iter().map(|(.., side)| *side).collect();
    // Faces of one mob kind that would overlap become one face with a stack count
    // (`look::group`); players, items, named mobs and dots stay single. Then kinds part.
    let keys: Vec<Option<u64>> = placed.iter().map(|(dot, ..)| {
        (dot.icon && dot.label.is_none() && dot.group as usize == ::map::radar::MOB)
            .then(|| dot.kind.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)))
    }).collect();
    let groups = look::group(&anchors, &sizes, &keys);
    let centres: Vec<[f32; 2]> = groups.iter().map(|g| g.at).collect();
    let sides: Vec<f32> = groups.iter().map(|g| g.side).collect();
    let parted = look::declutter(&centres, &sides, look::CROWD_REACH * scale, look::CROWD_GAP);
    let mouse = view.large.then(|| game.cursor());
    let mut labels = Vec::new();
    let mut hover = None;
    for (group, [x, y]) in groups.iter().zip(parted) {
        let (dot, _, _, face, _) = &placed[group.members[0]];
        let dot = *dot;
        // The group's face: its nearest member's, the least shaded.
        let shade = group.members.iter().map(|&m| placed[m].2).fold(0f32, f32::max);
        let alpha = group.members.iter().map(|&m| u32::from(placed[m].0.alpha)).max().unwrap_or(255);
        // A face is cut by the map's edge, as Xaero's are.
        let start = batch.vertices.len();
        let head = match face {
            Some((names, icon)) => draw_override(game, batch, names, [x, y], look::ICON_BOX * scale * dot.icon_scale, alpha, shade)
                .or_else(|| icon.as_ref().map(|icon| draw_icon(batch, icon, [x, y], look::icon_fit(icon.size, scale * dot.icon_scale), alpha, shade))),
            None if dot.icon && dot.group as usize == ::map::radar::ITEM => draw_item_head(game, batch, dot, [x, y], scale, alpha),
            None => None,
        };
        if let Some(face) = head && group.members.len() > 1 {
            let count = group.members.len().to_string();
            let (at, size) = look::stack_count(face, game.map_text_width(&count));
            small_text(game, batch, &count, at, size, (alpha << 24) | 0x00ff_ffff, Some((alpha << 24) | look::text_shadow(0xff_ffff)));
            if let Some(mouse) = mouse && look::contains(face, [mouse[0], mouse[1], 0., 0.]) {
                hover = Some((group_lines(game, group.members.iter().map(|&m| placed[m].0)), face));
            }
        }
        batch.clip_since(start, rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]);
        let shape = head.unwrap_or_else(|| {
            let half = dot.size*0.5;
            let level = (shade * 255.) as u32;
            let rgb = |c: u32| [16,8,0].into_iter().map(|s| ((c >> s & 255) * level / 255) << s).sum::<u32>();
            for (radius,color) in [(half+1.,(alpha<<24)|0x0a1318),(half,(alpha<<24)|rgb(dot.color))] {
                game.native_quad(batch,[[x,y-radius],[x+radius,y],[x,y+radius],[x-radius,y]],color);
            }
            [x-half,y-half,dot.size,dot.size]
        });
        let label = dot.label.as_ref().or_else(|| if head.is_some() {None} else {dot.fallback_name.as_ref()});
        if let Some(label) = label {
            labels.push((label,alpha,shape));
        }
    }
    if view.large {
        *RADAR_HOVER.lock().unwrap_or_else(|error| error.into_inner()) = hover;
    }
    for (label,alpha,head) in labels.into_iter().rev().take(if view.large {48} else {24}).collect::<Vec<_>>().into_iter().rev() {
        let width = game.map_text_width(label);
        let (tag, text) = look::name_tag(head, width);
        if !look::contains(rect, tag) { continue; }
        game.native_fill(batch, tag, ((look::NAME_BOX >> 24) * alpha / 255) << 24);
        small_text(game, batch, label, text, look::NAME_FONT, (alpha << 24) | 0x00ff_ffff, None);
    }
}

fn draw_tracked_in_world(game:&Game<'_>,batch:&mut HudBatch) {
    if game.map_option("locator_world")!=Some(1) {return;}
    let Some(player)=game.native_player() else {return;};
    let size=[batch.width,batch.height];
    let camera=world_camera(game,&player,size);
    let points=LOCATOR_WAYPOINTS.lock().unwrap_or_else(|error|error.into_inner());
    for point in points.iter() {
        let WaypointTarget::Block(at)=point.target else {continue;};
        let (id,name)=match &point.id {
            WaypointId::Uuid(uuid) if *uuid==game.own_uuid()=>continue,
            WaypointId::Uuid(uuid)=>{
                let id=game.remote.as_ref().and_then(|remote|remote.player_entity(*uuid));
                (id,id.and_then(|id|game.remote_player_name(id)))
            }
            WaypointId::Name(name)=>(None,Some(name.as_str())),
        };
        let position=[f64::from(at[0])+0.5,f64::from(at[1])+1.8,f64::from(at[2])+0.5];
        let Some([x,y])=project(&camera,position,size) else {continue;};
        if x<0. || y<0. || x>size[0] || y>size[1] {continue;}
        let w=8.*waypoint_scale(game,"locator_world_scale");
        let color=0xff000000|point.color.unwrap_or(0x85e9f8) as u32&0xffffff;
        let skin=id.and_then(|id|game.mobs.iter().find(|mob|mob.id==id))
            .map(|mob|mob.skin.as_deref().unwrap_or(&mob.kind.texture));
        if !skin.is_some_and(|skin|game.native_sprite(batch,skin,[8.,8.,8.,8.],[x-w*0.5,y-w*0.5,w,w],None)) {
            game.native_fill(batch,[x-w*0.5,y-w*0.5,w,w],color);
        }
        if let Some(name)=name {
            let name=name.chars().filter(|c|!c.is_control()).take(40).collect::<String>();
            let scale=waypoint_scale(game,"locator_name_scale");
            let left=x-game.map_text_width(&name)*scale*0.5;
            let top=y-w*0.5-10.*scale;
            let first=batch.vertices.len();
            game.map_text(batch,&name,[left,top],color,true);
            batch.scale_since(first,left,top,scale,scale);
        }
    }
}

fn draw_locator_waypoints(game: &Game<'_>, batch: &mut HudBatch, view: &View, rect: [f32; 4], round: bool) {
    let mut enabled=game.map_option("radar_locator")==Some(1);
    if view.large {
        enabled=match game.map_option("world_locator_mode") {Some(1)=>false,Some(2)=>true,_=>enabled};
    }
    if !enabled || !game.native_player().is_some_and(|player| player.dimension == view.dimension.id()) {
        return;
    }
    let visible_radar = ::map::radar::snapshot()
        .filter(|radar| !view.large || radar.world_map);
    for point in LOCATOR_WAYPOINTS.lock().unwrap_or_else(|error| error.into_inner()).iter() {
        if let WaypointId::Uuid(uuid) = point.id
            && (uuid == game.own_uuid() || game.remote.as_ref().and_then(|remote| remote.player_entity(uuid))
                .is_some_and(|id| visible_radar.as_ref().is_some_and(|radar| radar.dots.iter().any(|dot| dot.entity_id == id))))
        {
            continue;
        }
        let (at, exact) = match point.target {
            WaypointTarget::Block([x,_,z]) => ([f64::from(x)+0.5,f64::from(z)+0.5],true),
            WaypointTarget::Chunk([x,z]) => ([f64::from(x)*16.+8.,f64::from(z)*16.+8.],false),
            _ => continue,
        };
        let [x,y] = world_screen(view,rect,at);
        let scale=if view.large {1.}else{waypoint_scale(game,"locator_mini_scale")};
        let margin=5.*scale;
        if !map_overlay_inside([x-margin,y-margin],rect,round) || !map_overlay_inside([x+margin,y+margin],rect,round) {
            continue;
        }
        let color = 0xff00_0000 | (point.color.unwrap_or(0x85e9f8) as u32 & 0xffffff);
        if exact {
            game.native_fill(batch,[x-2.*scale,y-2.*scale,4.*scale,4.*scale],color);
        } else {
            for [dx,dy,w,h] in [[-4.,-4.,8.,1.],[-4.,3.,8.,1.],[-4.,-3.,1.,6.],[3.,-3.,1.,6.]] {
                game.native_fill(batch,[x+dx*scale,y+dy*scale,w*scale,h*scale],color);
            }
        }
    }
}

fn map_overlay_inside(at: [f32; 2], rect: [f32; 4], round: bool) -> bool {
    if !inside(at, rect) { return false; }
    let x = (at[0] - rect[0]) / rect[2] - 0.5;
    let z = (at[1] - rect[1]) / rect[3] - 0.5;
    !round || x.hypot(z) <= 0.48
}

/// The minimap's frame: four bevelled one-GUI bands, light on the top and left, dark on the
/// bottom and right; the same colours make the ring of a round map.
fn draw_minimap_frame(game: &Game<'_>, batch: &mut HudBatch, map: [f32; 4], round: bool) {
    if game.map_option("minimap_frame") == Some(3) { return; }
    if round {
        let steps = (map[2] * 0.75).clamp(48., 192.) as usize;
        for (quad, color) in look::frame_ring(map, steps) {
            game.native_quad(batch, quad, color);
        }
    } else {
        for (band, color) in look::frame_bands(map) {
            game.native_fill(batch, band, color);
        }
    }
}

fn draw_slime_chunks(game: &Game<'_>, batch: &mut HudBatch, view: &View, rect: [f32; 4], seed: i64) {
    use horizon_core::random::{RandomSource, slime_chunk_random};
    if view.dimension != Dimension::Overworld || view.span[0].max(view.span[1]) > 1024.0 {
        return;
    }
    let [cx, cz] = view.center.map(|value| (value.floor() as i32).div_euclid(16));
    let radius = (view.span[0].hypot(view.span[1]) / 32.0).ceil() as i32 + 1;
    for z in (cz - radius)..=(cz + radius) {
        for x in (cx - radius)..=(cx + radius) {
            let at = world_screen(view, rect, [f64::from(x * 16 + 8), f64::from(z * 16 + 8)]);
            if map_overlay_inside([at[0] - 3., at[1] - 3.], rect, view.round)
                && map_overlay_inside([at[0] + 3., at[1] + 3.], rect, view.round)
                && slime_chunk_random(x, z, seed, 987_234_911).next_int_bound(10) == Ok(0)
            {
                game.native_fill(batch, [at[0] - 3., at[1] - 3., 6., 6.], 0xca33c168);
            }
        }
    }
}

fn draw_light_overlay(game: &Game<'_>, batch: &mut HudBatch, view: &View, rect: [f32; 4]) {
    let Some(player) = game.native_player() else { return; };
    if Dimension::from_id(player.dimension) != view.dimension
        || (f64::from(rect[2]) / view.span[0]).min(f64::from(rect[3]) / view.span[1]) < 2.0
    {
        return;
    }
    let source = game.map_option("light_source").unwrap_or(0);
    let min = game.map_option("light_min").unwrap_or(0);
    let max = game.map_option("light_max").unwrap_or(7);
    let color=game.stored_setting(runtime::MOD_ID,"light_overlay_color")
        .and_then(|value|value.parse::<u32>().ok()).filter(|rgb|*rgb<=0xffffff);
    let y = match view.layer { Layer::Surface | Layer::Below(maps::FULL_CAVE) => player.y.floor() as i32, Layer::Below(y) => y };
    let [cx, cz] = view.center.map(|value| value.floor() as i32);
    for z in (cz - 24)..=(cz + 24) {
        for x in (cx - 24)..=(cx + 24) {
            let at = world_screen(view, rect, [f64::from(x) + 0.5, f64::from(z) + 0.5]);
            if !map_overlay_inside(at, rect, view.round) { continue; }
            if let Some((block, sky)) = game.map_light_if_loaded([x, y, z]) {
                let light = if source == 0 { block } else { sky };
                if (min..=max).contains(&light) {
                    let alpha=if light==0 {0xef000000}else{0xdf000000};
                    game.native_fill(batch,[at[0],at[1],1.5,1.5],alpha|color.unwrap_or(if light==0 {0xfd645f}else{0xf7c15d}));
                }
            }
        }
    }
}

fn draw(
    game: &Game<'_>,
    batch: &mut HudBatch,
    state: &State,
    rect: [f32; 4],
    large: bool,
    mouse: Option<[f32; 2]>,
) {
    let _timing = runtime::profiling::measure("hud-map");
    let settings = runtime::initialize(game.native_data_dir(runtime::MOD_ID)).settings();
    let round = !large && settings.round;
    if large {
        game.native_fill(batch,rect,look::UNEXPLORED);
    } else if let Some(opacity)=game.map_option("minimap_undiscovered").filter(|v|*v>0) {
        let color=(u32::from(opacity)*255/100)<<24 | (look::UNEXPLORED & 0x00ff_ffff);
        if round {
            let center=[rect[0]+rect[2]*0.5,rect[1]+rect[3]*0.5];
            let point=|step| {
                let (s,c)=(step as f32*std::f32::consts::TAU/48.).sin_cos();
                [center[0]+c*rect[2]*0.5,center[1]+s*rect[3]*0.5]
            };
            for i in 0..48 {game.native_quad(batch,[center,point(i),point(i+1),center],color);}
        } else {game.native_fill(batch,rect,color);}
    }
    let valid = state.frame.as_ref().filter(|f| {
        f.view.dimension == state.selected
            && f.view.large == large
            && f.view.session == state.session
    });
    if let Some(frame) = valid {
        if let Some(mouse) = mouse {
            let at = frame.view.block_at(
                f64::from((mouse[0] - rect[0]) / rect[2]) * frame.view.width as f64,
                f64::from((mouse[1] - rect[1]) / rect[3]) * frame.view.height as f64,
            );
            runtime::initialize(game.native_data_dir(runtime::MOD_ID))
                .maps
                .inspect(
                    frame.view.session,
                    frame.view.dimension,
                    frame.view.layer,
                    at,
                );
        }
        let mut terrain_visible = frame.known > 0;
        if let Some(atlas) = &frame.atlas {
            terrain_visible = false;
            if !atlas.pages.is_empty() {
                terrain_visible = draw_paged_terrain(batch, state, frame, atlas, rect);
            } else {
                for coarse in &atlas.coarse {
                    for part in &coarse.fragments {
                        if large {
                            if let Some((src, dst)) =
                                atlas.coarse_quad(&frame.view, rect, coarse, *part)
                            {
                                game.map_sprite(
                                    batch,
                                    horizon_client::hud::MOD_IMAGE_SPRITE,
                                    src,
                                    dst,
                                );
                                terrain_visible = true;
                            }
                        } else {
                            let bounds = coarse.key.bounds();
                            let uv = [
                                coarse.uv_origin[0] as f32
                                    + ((part[0] - bounds[0]) as f64 * 16.0 / bounds[2] as f64)
                                        as f32,
                                coarse.uv_origin[1] as f32
                                    + ((part[1] - bounds[1]) as f64 * 16.0 / bounds[3] as f64)
                                        as f32,
                                (part[2] as f64 * 16.0 / bounds[2] as f64) as f32,
                                (part[3] as f64 * 16.0 / bounds[3] as f64) as f32,
                            ];
                            terrain_visible |=
                                draw_terrain_polygon(game, batch, &frame.view, rect, *part, uv);
                        }
                    }
                }
                for (slot, chunk) in atlas.chunks.iter().enumerate() {
                    if large {
                        if let Some((src, dst)) = atlas.quad(&frame.view, rect, *chunk, slot) {
                            game.map_sprite(batch, horizon_client::hud::MOD_IMAGE_SPRITE, src, dst);
                            terrain_visible = true;
                        }
                    } else {
                        let columns = atlas.width / 16;
                        let slot = atlas.slots[slot];
                        terrain_visible |= draw_terrain_polygon(
                            game,
                            batch,
                            &frame.view,
                            rect,
                            [i64::from(chunk[0]) * 16, i64::from(chunk[1]) * 16, 16, 16],
                            [
                                (slot % columns * 16) as f32,
                                (slot / columns * 16) as f32,
                                16.,
                                16.,
                            ],
                        );
                    }
                }
            }
        } else if let Some(source) = &frame.raster_source {
            if let Some((src, dst)) = maps::raster_quad(&source.view, &frame.view, rect) {
                game.map_sprite(batch, horizon_client::hud::MOD_IMAGE_SPRITE, src, dst);
            } else {
                terrain_visible = false;
            }
        } else {
            game.map_sprite(
                batch,
                horizon_client::hud::MOD_IMAGE_SPRITE,
                [0.0, 0.0, frame.view.width as f32, frame.view.height as f32],
                rect,
            );
        }
        draw_claims(game, batch, state, &frame.view, rect);
        if settings.chunk_grid {
            draw_chunk_grid(game, batch, &frame.view, rect);
        }
        if large && settings.footsteps {
            for &(dimension, [x, z]) in &state.footsteps {
                if dimension == frame.view.dimension {
                    let px = world_screen(&frame.view, rect, [f64::from(x) + 0.5, f64::from(z) + 0.5]);
                    if inside(px, [rect[0] + 2.0, rect[1] + 2.0, rect[2] - 4.0, rect[3] - 4.0]) {
                        game.native_fill(batch, [px[0] - 1.5, px[1] - 1.5, 3.0, 3.0], 0xd42a_1f11);
                        game.native_fill(batch, [px[0] - 0.5, px[1] - 0.5, 1.0, 1.0], 0xffff_d679);
                    }
                }
            }
        }
        draw_border(game, batch, &frame.view, rect, mouse);
        if (game.map_option("slime_chunks") == Some(1) || state.slime_overlay || large && state.slime_preview)
            && let Some(seed) = state.slime_seed {
            draw_slime_chunks(game, batch, &frame.view, rect, seed);
        }
        if game.map_option("light_overlay") == Some(1) {
            draw_light_overlay(game, batch, &frame.view, rect);
        }
        if large && game.map_option("map_server_radius")==Some(1)
            && let Some(remote)=&game.remote
            && let Some(player)=game.native_player()
            && frame.view.dimension==Dimension::from_id(player.dimension) {
            let radius=remote.sent_radius().saturating_sub(1);
            let x=(player.x.floor() as i32).div_euclid(16);
            let z=(player.z.floor() as i32).div_euclid(16);
            draw_world_box(game,batch,&frame.view,rect,[
                f64::from((x-radius)*16),f64::from((z-radius)*16),
                f64::from((x+radius+1)*16),f64::from((z+radius+1)*16),
            ],0x9f62d0ff);
        }
        draw_radar(game, batch, &frame.view, rect, round, false);
        draw_locator_waypoints(game,batch,&frame.view,rect,round);
        draw_waystones(game, batch, &frame.view, rect);
        let mut drawn: Vec<[i32; 2]> = Vec::new();
        for (dimension, at, kind) in &state.portals {
            if *dimension != frame.view.dimension {
                continue;
            }
            if drawn
                .iter()
                .any(|point| (point[0] - at[0]).abs() <= 2 && (point[1] - at[2]).abs() <= 2)
            {
                continue;
            }
            drawn.push([at[0], at[2]]);
            draw_portal_marker(game, batch, &frame.view, rect, *at, *kind);
        }
        if large {
            if let Some(block) = state.menu.map(|menu| menu.block) {
                highlight(game, batch, &frame.view, rect, block);
            }
        }
        for (index, waypoint) in state
            .waypoints
            .iter()
            .enumerate()
            .filter(|(_, waypoint)| waypoint.dimension == frame.view.dimension && map_waypoint_visible(game, state, waypoint))
            .filter(|_| large && game.map_option("world_map_waypoints") != Some(0)
                && f64::from(rect[2])/frame.view.span[0]>=game.map_number("world_waypoint_zoom_min").unwrap_or(0.))
        {
            draw_pennant(
                game,
                batch,
                &frame.view,
                rect,
                waypoint,
                &waypoint_text(state, index, waypoint),
                game.map_number("world_waypoint_symbol_scale").unwrap_or(1.) as f32,
            );
        }
        if large {
            draw_team(game, batch, state, &frame.view, rect, false);
        }
        if large && let Some(player) = game.native_player() {
            draw_player(game, batch, &frame.view, rect, state.marker, &player, false);
        }
        if terrain_visible {
            return;
        }
    }
    let text = if valid.is_some() {
        tr(
            "No data",
            "Нет данных",
            "Aucune donnée",
            "Sin datos",
            "Keine Daten",
        )
    } else {
        tr(
            "Preparing map…",
            "Подготовка карты…",
            "Préparation de la carte…",
            "Preparando mapa…",
            "Karte wird vorbereitet…",
        )
    };
    game.map_text(
        batch,
        text,
        [
            rect[0] + (rect[2] - game.map_text_width(text)) / 2.0,
            rect[1] + rect[3] / 2.0 - 4.0,
        ],
        0xcccccc,
        true,
    );
}

fn tab(width: f32, index: usize) -> [f32; 4] {
    let w = ((width - 16.0) / 4.0).clamp(1.0, 112.0);
    [8.0 + w * index as f32, 8.0, (w - 2.0).max(1.0), 24.0]
}

fn information_rect(width: f32, height: f32) -> [f32; 4] {
    let w = (width - 24.0).clamp(1.0, 560.0);
    let h = (height - 24.0).clamp(1.0, 260.0);
    [(width - w) / 2.0, (height - h) / 2.0, w, h]
}
fn information_lines(game: &Game<'_>, state: &State, menu: Menu, width: f32) -> Vec<(String, u32)> {
    let layer = state
        .frame
        .as_ref()
        .filter(|frame| frame.view.dimension == menu.dimension)
        .map(|f| f.view.layer)
        .or_else(|| state.layers.get(&menu.dimension).copied())
        .unwrap_or(Layer::Surface);
    let coverage = runtime::initialize(game.native_data_dir(runtime::MOD_ID))
        .maps
        .coverage();
    let (chunk, region) = maps::chunk_region(menu.block);
    let chunk_rect = [i64::from(chunk[0]) * 16, i64::from(chunk[1]) * 16, 16, 16];
    let region_rect = [
        i64::from(region[0]) * 512,
        i64::from(region[1]) * 512,
        512,
        512,
    ];
    let detail = coverage.detailed_at(menu.dimension, layer, menu.block);
    let lod_sample = coverage.lod_sample(menu.dimension, layer, menu.block);
    let lod = lod_sample.as_ref().map(|sample| sample.key);
    let source = if detail {
        tr(
            "Detailed client chunk (cached)",
            "Подробный клиентский чанк (кэш)",
            "Chunk client détaillé (en cache)",
            "Chunk detallado del cliente (caché)",
            "Detaillierter Client-Chunk (Cache)",
        )
    } else if lod.is_some() {
        tr(
            "LOD — approximate data",
            "LOD — приблизительные данные",
            "LOD — données approximatives",
            "LOD — datos aproximados",
            "LOD — ungefähre Daten",
        )
    } else {
        tr(
            "No data for this layer",
            "Нет данных для этого слоя",
            "Aucune donnée pour cette couche",
            "Sin datos para esta capa",
            "Keine Daten für diese Ebene",
        )
    };
    let layer_text = match layer {
        Layer::Surface => tr(
            "Surface",
            "Поверхность",
            "Surface",
            "Superficie",
            "Oberfläche",
        )
        .to_owned(),
        Layer::Below(maps::FULL_CAVE) => tr("Full caves", "Общий слой пещер", "Toutes les grottes", "Todas las cuevas", "Alle Höhlen").to_owned(),
        Layer::Below(y) => localized_format!(
            "Below Y {y}",
            "Ниже Y {y}",
            "Sous Y {y}",
            "Por debajo de Y {y}",
            "Unter Y {y}"
        ),
    };
    let normal = 0xffe1e7ed;
    let yellow = 0xffffd45c;
    let mut lines = vec![
        (
            format!("{} — {}", menu.dimension.label(), layer_text),
            normal,
        ),
        (
            format!("X: {}  Z: {}", menu.block[0], menu.block[1]),
            normal,
        ),
    ];
    if let (Some(y), Some(fluid)) = (menu.height, menu.fluid) {
        let kind = match fluid.kind {
            FluidKind::Water => tr("Water", "Вода", "Eau", "Agua", "Wasser"),
            FluidKind::Lava => tr("Lava", "Лава", "Lave", "Lava", "Lava"),
        };
        lines.push((
            localized_format!(
                "{}: surface Y {y}, bottom Y {}",
                "{}: поверхность Y {y}, дно Y {}",
                "{} : surface Y {y}, fond Y {}",
                "{}: superficie Y {y}, fondo Y {}",
                "{}: Oberfläche Y {y}, Grund Y {}",
                kind,
                fluid.bottom
            ),
            normal,
        ));
    }
    lines.extend([
        (
            localized_format!(
                "Chunk: {} / {}",
                "Чанк: {} / {}",
                "Chunk : {} / {}",
                "Chunk: {} / {}",
                "Chunk: {} / {}",
                chunk[0],
                chunk[1]
            ),
            normal,
        ),
        (
            format!(
                "X {}..{}   Z {}..{}",
                chunk_rect[0],
                chunk_rect[0] + 15,
                chunk_rect[1],
                chunk_rect[1] + 15
            ),
            normal,
        ),
        (
            source.to_owned(),
            if !detail && lod.is_some() {
                yellow
            } else {
                normal
            },
        ),
    ]);
    if let Some(key) = lod {
        let stride = 1_i64 << key.level;
        let level = key.level;
        lines.push((
            localized_format!(
                "LOD quality: level {level} (1 sample per {stride}×{stride} blocks)",
                "Качество LOD: уровень {level} (1 отсчёт на {stride}×{stride} блоков)",
                "Qualité LOD : niveau {level} (1 échantillon pour {stride}×{stride} blocs)",
                "Calidad LOD: nivel {level} (1 muestra por {stride}×{stride} bloques)",
                "LOD-Qualität: Stufe {level} (1 Abtastwert je {stride}×{stride} Blöcke)",
            ),
            yellow,
        ));
        let b = key.bounds();
        lines.push((
            format!(
                "LOD: X {}..{}  Z {}..{}",
                b[0],
                b[0] + b[2] - 1,
                b[1],
                b[1] + b[3] - 1
            ),
            normal,
        ));
    }
    if let Some(sample) = lod_sample {
        let biome = sample
            .biome
            .as_ref()
            .map(|id| localized_biome(id))
            .unwrap_or_else(|| {
                tr("Biome: —", "биом: —", "Biome : —", "Bioma: —", "Biom: —").into()
            });
        lines.push((format!("LOD: Y ≈{} | {biome}", sample.height), yellow));
    }
    for (is_region, bounds) in [(false, chunk_rect), (true, region_rect)] {
        let (d, _) = coverage.counts(menu.dimension, layer, bounds);
        let (_, l) = coverage.counts(menu.dimension, Layer::Surface, bounds);
        if is_region {
            lines.push((
                localized_format!(
                    "Region: {} / {} (32×32 chunks)",
                    "Регион: {} / {} (32×32 чанка)",
                    "Région : {} / {} (32×32 chunks)",
                    "Región: {} / {} (32×32 chunks)",
                    "Region: {} / {} (32×32 Chunks)",
                    region[0],
                    region[1]
                ),
                normal,
            ));
            lines.push((
                format!(
                    "X {}..{}   Z {}..{}",
                    bounds[0],
                    bounds[0] + 511,
                    bounds[1],
                    bounds[1] + 511
                ),
                normal,
            ));
        }
        if l > 0 {
            let area = if is_region {
                tr("Region", "Регион", "Région", "Región", "Region")
            } else {
                tr("Chunk", "Чанк", "Chunk", "Chunk", "Chunk")
            };
            let status = if d > 0 {
                tr(
                    "mixed coverage: detailed + LOD",
                    "смешанное покрытие: подробное + LOD",
                    "couverture mixte : détaillée + LOD",
                    "cobertura mixta: detallada + LOD",
                    "gemischte Abdeckung: detailliert + LOD",
                )
            } else {
                tr(
                    "LOD coverage — approximate",
                    "LOD-покрытие — приблизительно",
                    "couverture LOD — approximative",
                    "cobertura LOD — aproximada",
                    "LOD-Abdeckung — ungefähr",
                )
            };
            lines.push((format!("{area}: {status}"), yellow));
            if layer != Layer::Surface {
                lines.push((
                    tr(
                        "Surface LOD cannot describe this underground layer",
                        "Наружный LOD не описывает этот подземный слой",
                        "Le LOD de surface ne décrit pas cette couche souterraine",
                        "El LOD de superficie no describe esta capa subterránea",
                        "Oberflächen-LOD beschreibt diese unterirdische Ebene nicht",
                    )
                    .to_owned(),
                    yellow,
                ));
            }
        }
        if is_region {
            lines.push((
                localized_format!(
                    "Cached detailed chunks: {d}; LOD footprints: {l}",
                    "Подробных чанков в кэше: {d}; LOD-тайлов: {l}",
                    "Chunks détaillés en cache : {d} ; tuiles LOD : {l}",
                    "Chunks detallados en caché: {d}; teselas LOD: {l}",
                    "Detaillierte Chunks im Cache: {d}; LOD-Kacheln: {l}"
                ),
                normal,
            ));
        }
    }
    lines.push((
        tr(
            "Visit history is not recorded yet",
            "История посещений пока не записывается",
            "L’historique des visites n’est pas encore enregistré",
            "El historial de visitas aún no se registra",
            "Der Besuchsverlauf wird noch nicht aufgezeichnet",
        )
        .to_owned(),
        0xff9faebd,
    ));
    let mut wrapped = Vec::new();
    for (text, color) in lines {
        let mut line = String::new();
        for word in text.split_whitespace() {
            let next = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && game.map_text_width(&next) > width {
                wrapped.push((std::mem::take(&mut line), color));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            wrapped.push((line, color));
        }
    }
    wrapped
}
/// The information panel's Close button, at its reading end.
fn information_close(rect: [f32; 4]) -> [f32; 4] {
    let button = [rect[0] + rect[2] - 82.0, rect[1] + rect[3] - 24.0, 72.0, 18.0];
    if runtime::right_to_left() { [rect[0] + 10.0, button[1], button[2], button[3]] } else { button }
}

fn draw_information(game: &Game<'_>, batch: &mut HudBatch, state: &State, menu: Menu) {
    game.native_fill(batch, [0.0, 0.0, batch.width, batch.height], look::SCREEN_DIM);
    let rect = information_rect(batch.width, batch.height);
    game.native_fill(batch, rect, look::MENU_BORDER);
    game.native_fill(batch, [rect[0] + 1., rect[1] + 1., rect[2] - 2., rect[3] - 2.], look::MENU_ROW);
    let title = tr(
        "Chunk / region",
        "Чанк / регион",
        "Chunk / région",
        "Chunk / región",
        "Chunk / Region",
    );
    let width = game.map_text_width(title);
    game.map_text(batch, title, [(rect[0] + (rect[2] - width) * 0.5).round(), rect[1] + 8.0], look::SCREEN_TEXT, true);
    let lines = information_lines(game, state, menu, rect[2] - 24.0);
    let visible = ((rect[3] - 56.0) / 12.0).max(1.0) as usize;
    let start = state.info_scroll.min(lines.len().saturating_sub(visible));
    for (i, (text, color)) in lines.iter().skip(start).take(visible).enumerate() {
        text_in(game, batch, text, rect, 10., rect[1] + 25.0 + i as f32 * 12.0, 0xff00_0000 | *color);
    }
    if lines.len() > visible {
        let area = rect[3] - 56.0;
        let thumb = area * visible as f32 / lines.len() as f32;
        let y = rect[1]
            + 25.0
            + (area - thumb) * start as f32 / lines.len().saturating_sub(visible).max(1) as f32;
        let bar = if runtime::right_to_left() { rect[0] + 4.0 } else { rect[0] + rect[2] - 7.0 };
        game.native_fill(batch, [bar, y, 3.0, thumb], 0xffa2b2bf);
    }
    button(
        game,
        batch,
        information_close(rect),
        tr("Close", "Закрыть", "Fermer", "Cerrar", "Schließen"),
        game.cursor(),
        true,
    );
}

fn export_map_png(game: &Game<'_>, state: &mut State, dimension: Dimension) {
    if state.export.is_some() {
        return;
    }
    let layer = state.frame.as_ref()
        .filter(|frame| frame.view.session == state.session && frame.view.dimension == dimension)
        .map_or(Layer::Surface, |frame| frame.view.layer);
    let maps = &runtime::initialize(game.native_data_dir(runtime::MOD_ID)).maps;
    let text = match maps.export_png(state.session, dimension, layer, write_map_png) {
        Ok(reply) => {
            state.export = Some(reply);
            tr(
                "Exporting explored terrain…",
                "Экспортирую исследованную местность…",
                "Export de la carte…",
                "Exportando el mapa…",
                "Karte wird exportiert…",
            ).to_owned()
        }
        Err(error) => error,
    };
    state.notice = Some((text, Instant::now()));
}

const SETTING_CATEGORIES: usize = 5;

fn setting_category(key: &str) -> usize {
    if key.contains("radar") || key.starts_with("locator_") || key == "world_locator_mode" {
        3
    } else if key.contains("waypoint") || key.contains("waystone") {
        2
    } else if key.starts_with("world_") || key.starts_with("map_") || key == "footsteps" {
        1
    } else if key.starts_with("minimap_") || key.starts_with("quick_minimap") ||
        matches!(key,"enabled"|"corner"|"size"|"shape"|"rotate"|"zoom"|"mini_dimension"|"directions"|"direction_style")
    {
        0
    } else {
        4
    }
}

const CATEGORY_KEYS: [&str; SETTING_CATEGORIES] =
    ["minimap_settings", "world_map_settings", "waypoint_settings", "radar_settings", "other_settings"];

/// A category's settings, the main switches first (the minimap's own rows in their order),
/// then the rest as registered.
fn settings_catalog<'a>(game: &'a Game<'_>, category: usize) -> impl Iterator<Item=&'a horizon_client::mod_list::NativeSetting> {
    let mut rows: Vec<_> = game.settings.mods.installed.iter().find(|m|m.id==runtime::MOD_ID)
        .and_then(|m|m.native.as_ref()).into_iter()
        .flat_map(|page|page.settings.iter()).filter(move |row| setting_category(&row.key)==category)
        .collect();
    rows.sort_by_key(|row| runtime::SDK_SETTING_ROWS.iter().position(|(key, _)| *key == row.key).unwrap_or(usize::MAX));
    rows.into_iter()
}

fn setting_value(game: &Game<'_>, value:&horizon_client::mod_list::NativeSettingValue)->String {
    use horizon_client::mod_list::NativeSettingValue as Value;
    match value {
        Value::Toggle(on) => ui(if *on { "on" } else { "off" }),
        Value::Choice { options, index } => options.get(*index)
            .map(|option| game.translate(option).unwrap_or_else(||option.clone())).unwrap_or_default(),
        Value::Range { value, .. } => format!("{value:.3}").trim_end_matches('0').trim_end_matches('.').to_owned(),
        Value::Color(rgb) => format!("#{rgb:06X}"),
    }
}

/// What a settings button does.
#[derive(Clone, Copy, PartialEq)]
enum SettingsTarget {
    Category(usize),
    WaypointList,
    Caches,
    Action(MenuAction),
    /// The index of a setting in its category.
    Setting(usize),
}

struct SettingsItem {
    label: String,
    tooltip: Option<String>,
    /// A slider's position, for a range setting.
    slider: Option<f32>,
    /// A colour swatch, for a colour setting.
    swatch: Option<u32>,
    target: SettingsTarget,
}

/// Every button of the open settings page, before paging: the home page's categories and
/// tools, or one category's settings as `Label: Value`.
fn settings_items(game: &Game<'_>, state: &State) -> Vec<SettingsItem> {
    let plain = |key: &str, target: SettingsTarget, tip: bool| SettingsItem {
        label: ui(key),
        tooltip: tip.then(|| ui(&format!("tip_{key}"))),
        slider: None,
        swatch: None,
        target,
    };
    let Some(category) = state.settings_tab else {
        let mut items: Vec<_> = CATEGORY_KEYS.iter().enumerate()
            .map(|(i, key)| plain(key, SettingsTarget::Category(i), false))
            .collect();
        items.push(plain("waypoint_list", SettingsTarget::WaypointList, false));
        items.push(plain("map_caches", SettingsTarget::Caches, true));
        for (key, action) in [
            ("import_waypoints", MenuAction::ImportXaero),
            ("import_map_cache", MenuAction::ImportXaeroCache),
            ("import_settings", MenuAction::ImportXaeroSettings),
            ("next_profile", MenuAction::NextXaeroProfile),
            ("reload_radar_rules", MenuAction::ReloadRadarRules),
        ] {
            items.push(plain(key, SettingsTarget::Action(action), true));
        }
        return items;
    };
    use horizon_client::mod_list::NativeSettingValue as Value;
    settings_catalog(game, category).enumerate().map(|(i, setting)| {
        let name = game.translate(&setting.label).unwrap_or_else(|| setting.key.replace('_', " "));
        SettingsItem {
            label: format!("{name}: {}", setting_value(game, &setting.value)),
            tooltip: setting.tooltip.as_ref().and_then(|key| game.translate(key)),
            slider: matches!(setting.value, Value::Range { .. }).then(|| setting.value.fraction()),
            swatch: if let Value::Color(rgb) = setting.value { Some(rgb) } else { None },
            target: SettingsTarget::Setting(i),
        }
    }).collect()
}

const SETTINGS_PAGE: usize = look::PAGE_ROWS * 2;

fn settings_pages(items: usize) -> usize {
    items.div_ceil(SETTINGS_PAGE).max(1)
}

/// A vanilla GUI sprite (`gui/sprites/<name>`) drawn nine-slice into `dst`. `width` is the
/// sprite's size in GUI pixels as its `.mcmeta` gives it, so packs of any resolution fit.
fn gui_sprite(game: &Game<'_>, batch: &mut HudBatch, name: &str, dst: [f32; 4], width: f32, border: f32) -> bool {
    let key = format!("gui/sprites/{name}");
    let Some(sprite) = game.gui.atlas.sprites.get(key.as_str()).copied() else { return false };
    let size = [sprite.w as f32, sprite.h as f32];
    let texels = (size[0] / width).max(1e-3);
    for (src, piece) in look::nine_slice(dst, size, border * texels, 1. / texels) {
        game.native_sprite(batch, &key, src, piece, None);
    }
    true
}

/// A vanilla button: the game's own button sprite and its label, white with shadow, grey when
/// disabled.
fn button(game: &Game<'_>, batch: &mut HudBatch, rect: [f32; 4], label: &str, mouse: [f32; 2], enabled: bool) {
    let hovered = enabled && look::hit(mouse, rect);
    let sprite = if !enabled { "widget/button_disabled" } else if hovered { "widget/button_highlighted" } else { "widget/button" };
    if !gui_sprite(game, batch, sprite, rect, look::BUTTON[0], look::BUTTON_BORDER) {
        game.native_fill(batch, rect, if hovered { 0xff7a7a7a } else { 0xff5a5a5a });
    }
    let text = fit_text(game, label, rect[2] - 6.);
    let width = game.map_text_width(&text);
    game.map_text(batch, &text, [(rect[0] + (rect[2] - width) * 0.5).round(), (rect[1] + (rect[3] - 8.) * 0.5).round()],
        if enabled { look::SCREEN_TEXT } else { look::DISABLED_TEXT }, true);
}

/// A vanilla slider: its track, the handle at `fraction` and the centred label.
fn slider(game: &Game<'_>, batch: &mut HudBatch, rect: [f32; 4], label: &str, fraction: f32, mouse: [f32; 2]) {
    let hovered = look::hit(mouse, rect);
    if !gui_sprite(game, batch, if hovered { "widget/slider_highlighted" } else { "widget/slider" }, rect, look::BUTTON[0], 1.) {
        game.native_fill(batch, rect, 0xff000000);
    }
    let handle = [rect[0] + (rect[2] - 8.) * fraction.clamp(0., 1.), rect[1], 8., rect[3]];
    let name = if hovered { "gui/sprites/widget/slider_handle_highlighted" } else { "gui/sprites/widget/slider_handle" };
    if let Some(sprite) = game.gui.atlas.sprites.get(name).copied() {
        game.native_sprite(batch, name, [0., 0., sprite.w as f32, sprite.h as f32], handle, None);
    }
    let text = fit_text(game, label, rect[2] - 6.);
    let width = game.map_text_width(&text);
    game.map_text(batch, &text, [(rect[0] + (rect[2] - width) * 0.5).round(), (rect[1] + 6.).round()], look::SCREEN_TEXT, true);
}

/// A vanilla edit box: the text field sprite, the text (or a grey hint) and a cursor while
/// focused. Long text shows its end.
fn draw_field(game: &Game<'_>, batch: &mut HudBatch, rect: [f32; 4], text: &str, hint: &str, focused: bool) {
    let sprite = if focused { "widget/text_field_highlighted" } else { "widget/text_field" };
    if !gui_sprite(game, batch, sprite, rect, look::BUTTON[0], look::FIELD_BORDER) {
        game.native_fill(batch, rect, if focused { 0xffffffff } else { 0xffa0a0a0 });
        game.native_fill(batch, [rect[0] + 1., rect[1] + 1., rect[2] - 2., rect[3] - 2.], 0xff000000);
    }
    let y = (rect[1] + (rect[3] - 8.) * 0.5).round();
    let room = rect[2] - 10.;
    if text.is_empty() && !focused {
        let hint = fit_text(game, hint, room);
        let x = if runtime::right_to_left() { rect[0] + rect[2] - 4. - game.map_text_width(&hint) } else { rect[0] + 4. };
        game.map_text(batch, &hint, [x, y], look::HINT_TEXT, false);
        return;
    }
    let mut shown: String = text.to_owned();
    while game.map_text_width(&shown) > room && !shown.is_empty() {
        shown.remove(0);
    }
    if focused { shown.push('_'); }
    let x = if runtime::right_to_left() { rect[0] + rect[2] - 4. - game.map_text_width(&shown) } else { rect[0] + 4. };
    draw_text(game, batch, &shown, [x, y], 0xffe0e0e0, true);
}

/// The title line of a vanilla screen, centred.
fn screen_title(game: &Game<'_>, batch: &mut HudBatch, text: &str, y: f32) {
    let width = game.map_text_width(text);
    game.map_text(batch, text, [((batch.width - width) * 0.5).round(), y], look::SCREEN_TEXT, true);
}

/// Whether the world map lies behind the open screen.
fn map_behind(state: &State) -> bool {
    match state.mode {
        Mode::Map => true,
        Mode::Settings => state.settings_return == Some(Mode::Map),
        Mode::Name => state.name_form.as_ref().is_some_and(|form| form.background == Mode::Map),
        _ => false,
    }
}

/// The world map's terrain, darkened, behind a screen opened from it.
fn draw_map_backdrop(game: &Game<'_>, batch: &mut HudBatch, state: &State) {
    draw(game, batch, state, rectangle(batch.width, batch.height), true, None);
    game.native_fill(batch, [0., 0., batch.width, batch.height], look::SCREEN_DIM);
}

/// What lies behind a screen: the darkened world map when it was opened from the map, else
/// the blurred world darkened as vanilla darkens it.
fn draw_backdrop(game: &Game<'_>, batch: &mut HudBatch, state: &State) {
    if map_behind(state) {
        draw_map_backdrop(game, batch, state);
    } else {
        game.native_fill(batch, [0., 0., batch.width, batch.height], look::SCREEN_DIM);
    }
}

/// Whether the open screen shows the world behind it, blurred as vanilla screens do.
pub(super) fn blurs_world(game: &Game<'_>) -> bool {
    game.map_on(runtime::MOD_ID) && game.mod_screen() == Some(runtime::MOD_ID) && !map_behind(&state())
}

fn draw_settings(game: &Game<'_>, batch: &mut HudBatch, state: &State) {
    draw_backdrop(game, batch, state);
    let size = [batch.width, batch.height];
    let rtl = runtime::right_to_left();
    let mouse = game.cursor();
    let items = settings_items(game, state);
    let pages = settings_pages(items.len());
    let page = state.settings_scroll.min(pages - 1);
    let title = match state.settings_tab {
        None => ui("map_settings"),
        Some(category) if pages > 1 => format!("{} ({}/{pages})", ui(CATEGORY_KEYS[category]), page + 1),
        Some(category) => ui(CATEGORY_KEYS[category]),
    };
    screen_title(game, batch, &title, look::screen_title_y(batch.height));
    let shown: Vec<_> = items.iter().skip(page * SETTINGS_PAGE).take(SETTINGS_PAGE).collect();
    let (rects, back) = look::settings_grid(size, shown.len(), rtl);
    let mut tooltip = None;
    for (item, rect) in shown.iter().zip(rects) {
        match item.slider {
            Some(fraction) => slider(game, batch, rect, &item.label, fraction, mouse),
            None => button(game, batch, rect, &item.label, mouse, true),
        }
        if let Some(rgb) = item.swatch {
            let at = if rtl { rect[0] + 4. } else { rect[0] + rect[2] - 14. };
            game.native_fill(batch, [at, rect[1] + 5., 10., 10.], 0xff000000);
            game.native_fill(batch, [at + 1., rect[1] + 6., 8., 8.], waypoint_argb(rgb));
        }
        if look::hit(mouse, rect) {
            // A tooltip adds what the button cannot say: its description, or its whole label
            // when the button cuts it; never the label it already shows.
            let cut = fit_text(game, &item.label, rect[2] - 6.) != item.label;
            tooltip = item.tooltip.clone().or_else(|| cut.then(|| item.label.clone())).map(|text| (text, rect));
        }
    }
    if pages > 1 {
        let [prev, next] = look::pager(size, rtl);
        // The arrows point the way the pages turn: outward, mirrored right to left.
        let (back_arrow, on_arrow) = if rtl { (">>", "<<") } else { ("<<", ">>") };
        button(game, batch, prev, &format!("{back_arrow} {}", ui("previous")), mouse, page > 0);
        button(game, batch, next, &format!("{} {on_arrow}", ui("next")), mouse, page + 1 < pages);
    }
    button(game, batch, back, &ui(if state.settings_tab.is_none() { "done" } else { "back" }), mouse, true);
    draw_notice(game, batch, state);
    if let Some((text, hovered)) = tooltip {
        draw_tooltip(game, batch, &text, mouse, hovered);
    }
}

fn open_settings(state:&mut State, back:Option<Mode>, tab:Option<usize>) {
    state.settings_return=back;
    state.settings_tab=tab;
    state.settings_scroll=0;
    state.map_panel=MapPanel::None;
    state.dragging=false;
    state.menu=None;
    state.mode=Mode::Settings;
    runtime::set_text_input(false);
}

fn close_settings(state:&mut State) {
    if state.settings_tab.take().is_some() {
        state.settings_scroll = 0;
        return;
    }
    if let Some(mode)=state.settings_return.take() {state.mode=mode;} else {state.close=true;}
}

/// A menu with no point of its own, for the actions the settings home page runs.
fn home_menu(state: &State) -> Menu {
    Menu {
        height_approximate: false,
        block: [0, 0],
        height: None,
        fluid: None,
        dimension: state.selected,
        at: [0., 0.],
        portal: None,
        waypoint: None,
        radar_player: None,
    }
}

fn settings_press(game:&Game<'_>, state:&mut State, screen:&mod_api::ScreenView<'_>, button:i32) {
    if !matches!(button,0|1) {return;}
    let size = [screen.rect[2], screen.rect[3]];
    let rtl = runtime::right_to_left();
    let mouse=screen.mouse;
    let items = settings_items(game, state);
    let pages = settings_pages(items.len());
    let page = state.settings_scroll.min(pages - 1);
    let shown: Vec<_> = items.iter().skip(page * SETTINGS_PAGE).take(SETTINGS_PAGE).collect();
    let (rects, back) = look::settings_grid(size, shown.len(), rtl);
    if button == 0 && look::hit(mouse, back) { close_settings(state); return; }
    if button == 0 && pages > 1 {
        let [prev, next] = look::pager(size, rtl);
        if look::hit(mouse, prev) && page > 0 { state.settings_scroll = page - 1; return; }
        if look::hit(mouse, next) && page + 1 < pages { state.settings_scroll = page + 1; return; }
    }
    let Some((item, rect)) = shown.iter().zip(rects).find(|(_, rect)| look::hit(mouse, *rect)) else { return };
    match item.target {
        SettingsTarget::Category(i) if button == 0 => { state.settings_tab = Some(i); state.settings_scroll = 0; }
        SettingsTarget::WaypointList if button == 0 => { state.settings_return = None; state.mode = Mode::Waypoints; state.waypoint_scroll = 0; }
        SettingsTarget::Caches if button == 0 => show_cache_manager(game, state),
        SettingsTarget::Action(action) if button == 0 => {
            if game.native_player().is_some() {
                let menu = home_menu(state);
                perform_menu(game, state, menu, action);
            }
        }
        SettingsTarget::Setting(index) => {
            let Some(category) = state.settings_tab else { return };
            let Some(row) = settings_catalog(game, category).nth(index) else { return };
            use horizon_client::mod_list::NativeSettingValue as Value;
            let mut next = row.value.clone();
            match &mut next {
                Value::Range {..} => next.slide((mouse[0] - rect[0] - 4.) / (rect[2] - 8.).max(1.)),
                Value::Choice { options,index } if button==1 && !options.is_empty() => {
                    *index=(*index+options.len()-1)%options.len();
                }
                _ if button == 0 => next.click(),
                _ => return,
            }
            let value = match next {
                Value::Toggle(value)=>value.to_string(),
                Value::Choice {index,..}=>index.to_string(),
                Value::Range {value,..}=>value.to_string(),
                Value::Color(rgb)=>rgb.to_string(),
            };
            state.map_settings_change=Some((row.key.clone(),value));
        }
        _ => {}
    }
}

// Interface text from the lang files (`map.ui.<key>`); English is the fallback the en_us
/// file also holds.
fn ui(key: &str) -> String {
    let full = format!("map.ui.{key}");
    let text = horizon_client::lang::get_or(&full, "");
    if text.is_empty() { key.replace('_', " ") } else { text }
}

/// Text that keeps its left-to-right order: in a right-to-left language the renderer would
/// reverse a line without letters (`-32, 69, 0` becoming `0 ,69 ,32-`), so such a line is laid
/// out one character at a time.
fn draw_text(game: &Game<'_>, batch: &mut HudBatch, text: &str, at: [f32; 2], color: u32, shadow: bool) {
    if !runtime::right_to_left() {
        game.map_text(batch, text, at, color, shadow);
        return;
    }
    if text.chars().any(char::is_alphabetic) {
        let runs = runtime::arabic::number_runs(text);
        if runs.len() < 2 || !text.chars().any(|c| ('\u{0590}'..='\u{08ff}').contains(&c)) {
            game.map_text(batch, text, at, color, shadow);
            return;
        }
        // Numbers inside right-to-left words keep their own order (`-3 / 10`, not `3- / 10`):
        // the runs go right to left, each number drawn left to right.
        let mut x = at[0] + runs.iter().map(|(_, run)| game.map_text_width(run)).sum::<f32>();
        for (number, run) in runs {
            x -= game.map_text_width(run);
            if number {
                draw_text_ltr(game, batch, run, [x, at[1]], color, shadow);
            } else {
                game.map_text(batch, run, [x, at[1]], color, shadow);
            }
        }
        return;
    }
    draw_text_ltr(game, batch, text, at, color, shadow);
}

/// Text without letters laid out one character at a time, left to right.
fn draw_text_ltr(game: &Game<'_>, batch: &mut HudBatch, text: &str, at: [f32; 2], color: u32, shadow: bool) {
    let mut x = at[0];
    let mut buffer = [0u8; 4];
    for c in text.chars() {
        let glyph: &str = c.encode_utf8(&mut buffer);
        game.map_text(batch, glyph, [x, at[1]], color, shadow);
        x += game.map_text_width(glyph);
    }
}

/// A line's segments in reading order: reversed right to left, so a key name follows its
/// label there too.
fn reading_order<T: Clone>(parts: &[T]) -> Vec<T> {
    let mut parts = parts.to_vec();
    if runtime::right_to_left() { parts.reverse(); }
    parts
}

/// A tooltip: one or more lines of segments, each with its own colour (keys are green).
/// A tooltip and the rect of what it describes, which it must leave visible.
type Tip = (Vec<Vec<(String, u32)>>, [f32; 4]);

fn draw_tooltip_lines(game: &Game<'_>, batch: &mut HudBatch, lines: &[Vec<(String, u32)>], at: [f32; 2], hovered: [f32; 4]) {
    if lines.is_empty() {
        return;
    }
    let width = lines.iter()
        .map(|line| line.iter().map(|(text, _)| game.map_text_width(text)).sum::<f32>())
        .fold(0., f32::max);
    let height = lines.len() as f32 * look::INFO_PITCH - 1.;
    let rect = look::tooltip_box(at, [width, height], [batch.width, batch.height], hovered);
    game.native_fill(batch, rect, look::TOOLTIP_BOX);
    let rtl = runtime::right_to_left();
    for (row, line) in lines.iter().enumerate() {
        let line_width: f32 = line.iter().map(|(text, _)| game.map_text_width(text)).sum();
        let mut x = if rtl { rect[0] + rect[2] - 3. - line_width } else { rect[0] + 3. };
        for (text, color) in reading_order(line) {
            draw_text(game, batch, &text, [x, rect[1] + 2. + row as f32 * look::INFO_PITCH], color, true);
            x += game.map_text_width(&text);
        }
    }
}

fn draw_tooltip(game: &Game<'_>, batch: &mut HudBatch, text: &str, at: [f32; 2], hovered: [f32; 4]) {
    draw_tooltip_lines(game, batch, &[vec![(text.to_owned(), look::TOOLTIP_TEXT)]], at, hovered);
}

/// A bare glyph with its shadow one pixel down and right.
fn draw_glyph(game: &Game<'_>, batch: &mut HudBatch, rows: &[&str], cell: [f32; 4], color: u32) {
    let origin = look::glyph_origin(cell, rows);
    for run in look::glyph_runs(rows, [origin[0] + 1., origin[1] + 1.]) {
        game.native_fill(batch, run, look::GLYPH_SHADOW);
    }
    for run in look::glyph_runs(rows, origin) {
        game.native_fill(batch, run, color);
    }
}

/// A key's name for a tooltip, or nothing when it is unbound.
fn key_name(game: &Game<'_>, id: &str, default: &str) -> Option<String> {
    let key = game.map_key(runtime::MOD_ID, id, default);
    if key.ends_with(".unknown") || key.is_empty() {
        return None;
    }
    let name = horizon_client::lang::get_or(&key, "");
    Some(if name.is_empty() {
        key.rsplit('.').next().unwrap_or(&key).to_uppercase()
    } else { name })
}

/// Whether a world-map control's switch is on (`None` for a plain action).
fn control_on(game: &Game<'_>, state: &State, control: look::Control) -> Option<bool> {
    let flag = |key: &str, default: bool| game.map_option(key).map_or(default, |v| v == 1);
    match control {
        look::Control::WaypointMenu => Some(state.map_panel == MapPanel::Waypoints),
        look::Control::Waypoints => Some(flag("world_map_waypoints", true)),
        look::Control::TrackedPlayers => Some(flag("radar_locator", false)),
        look::Control::Radar => Some(flag("radar_world_map", true) && flag("radar", true)),
        look::Control::CaveMode => Some(flag("world_cave_enabled", true)),
        _ => None,
    }
}

/// The controls shown: the zoom pair follows its setting.
fn world_controls(game: &Game<'_>, size: [f32; 2]) -> Vec<(look::Control, [f32; 4])> {
    let zoom = game.map_option("world_zoom_buttons") != Some(0);
    look::world_controls(size, runtime::right_to_left()).into_iter()
        .filter(|(control, _)| zoom || !matches!(control, look::Control::ZoomIn | look::Control::ZoomOut))
        .collect()
}

fn control_tooltip(game: &Game<'_>, state: &State, control: look::Control) -> Vec<Vec<(String, u32)>> {
    let white = look::TOOLTIP_TEXT;
    let line = |text: String| vec![(text, white)];
    let switch = |key: &str| {
        let on = control_on(game, state, control).unwrap_or(false);
        line(format!("{}: {}", ui(key), ui(if on { "on" } else { "off" })))
    };
    match control {
        look::Control::Settings => vec![line(ui("settings"))],
        look::Control::MapSwitching => vec![line(ui("map_caches"))],
        look::Control::CaveMode => vec![switch("cave_mode")],
        look::Control::Dimension => vec![line(format!("{}: {}", ui("dimension"), state.selected.label()))],
        look::Control::HopTo => vec![line(ui("hop_to_waypoint"))],
        look::Control::Camera => vec![line(ui("centre_on_player"))],
        look::Control::WaypointMenu => vec![line(ui(if state.map_panel == MapPanel::Waypoints { "close_mini_menu" } else { "open_mini_menu" }))],
        look::Control::Waypoints => vec![switch("waypoints")],
        look::Control::TrackedPlayers => vec![switch("tracked_players")],
        look::Control::Radar => vec![switch("entity_radar")],
        look::Control::ExportPng => vec![line(ui("export_png"))],
        look::Control::ZoomOut => vec![line(ui("zoom_out"))],
        look::Control::ZoomIn => vec![line(ui("zoom_in"))],
        look::Control::Help => {
            let mut lines = vec![line(ui("help_drag")), line(ui("help_wheel")), line(ui("help_right_click"))];
            for (id, default, label) in [
                ("create_waypoint", "key.keyboard.r", "help_new_waypoint"),
                (runtime::WAYPOINTS_KEY, runtime::WAYPOINTS_DEFAULT, "help_waypoints"),
                (runtime::WORLD_MAP_KEY, runtime::WORLD_MAP_DEFAULT, "help_close_map"),
            ] {
                let key = key_name(game, id, default).unwrap_or_else(|| ui("unset"));
                lines.push(vec![(format!("{} ", ui(label)), white), (format!("({key})"), look::KEY_TEXT)]);
            }
            lines
        }
    }
}

fn draw_world_controls(game: &Game<'_>, batch: &mut HudBatch, state: &State, mouse: [f32; 2]) -> Option<Tip> {
    let mut tooltip = None;
    for (control, cell) in world_controls(game, [batch.width, batch.height]) {
        let hovered = look::hit(mouse, cell);
        let color = match control_on(game, state, control) {
            Some(false) => look::GLYPH_OFF,
            _ if hovered => look::GLYPH_HOVER,
            _ => look::GLYPH,
        };
        draw_glyph(game, batch, look::glyph(control), cell, color);
        if hovered {
            tooltip = Some((control_tooltip(game, state, control), cell));
        }
    }
    tooltip
}

fn toggle_setting(game: &Game<'_>, state: &mut State, key: &str, default: bool) {
    let on = game.map_option(key).map_or(default, |v| v == 1);
    state.map_settings_change = Some((key.to_owned(), (!on).to_string()));
}

fn press_world_control(game: &Game<'_>, state: &mut State, control: look::Control) {
    match control {
        look::Control::Settings => open_settings(state, Some(Mode::Map), None),
        look::Control::MapSwitching => show_cache_manager(game, state),
        look::Control::CaveMode => toggle_setting(game, state, "world_cave_enabled", true),
        look::Control::Dimension => {
            let i = Dimension::ALL.iter().position(|d| *d == state.selected).unwrap_or(0);
            let next = Dimension::ALL[(i % 3) + 1];
            runtime::initialize(game.native_data_dir(runtime::MOD_ID)).choose_dimension(true, next);
        }
        look::Control::HopTo => {
            let points: Vec<_> = state.waypoints.iter()
                .filter(|point| point.dimension == state.selected && map_waypoint_visible(game, state, point))
                .map(|point| [f64::from(point.x) + 0.5, f64::from(point.z) + 0.5])
                .collect();
            if !points.is_empty() {
                state.hop = (state.hop + 1) % points.len();
                state.camera.snap(points[state.hop]);
                state.elapsed = 1.;
            }
        }
        look::Control::Camera => {
            if let Some(player) = game.native_player() {
                let dimension = Dimension::from_id(player.dimension);
                state.camera.snap(dimension.convert(state.selected, [player.x, player.z]));
                state.elapsed = 1.;
            }
        }
        look::Control::WaypointMenu => {
            state.map_panel.toggle(MapPanel::Waypoints);
            state.map_panel_scroll = 0;
            state.mini_filter_focus = false;
            runtime::set_text_input(false);
        }
        look::Control::Waypoints => toggle_setting(game, state, "world_map_waypoints", true),
        look::Control::TrackedPlayers => toggle_setting(game, state, "radar_locator", false),
        look::Control::Radar => toggle_setting(game, state, "radar_world_map", true),
        look::Control::ExportPng => export_map_png(game, state, state.selected),
        look::Control::Help => {}
        look::Control::ZoomOut => map_zoom(game, state, -1., [0., 0.]),
        look::Control::ZoomIn => map_zoom(game, state, 1., [0., 0.]),
    }
}

/// The mini-menu's waypoints, filtered by its box.
fn mini_waypoints<'a>(state: &'a State) -> Vec<(usize, &'a Waypoint)> {
    let query = state.mini_filter.trim().to_lowercase();
    shown_waypoints(state)
        .filter(|(_, point)| point.dimension == state.selected)
        .filter(|(_, point)| query.is_empty() || point.name.to_lowercase().contains(&query))
        .collect()
}

fn mini_toggle_on(game: &Game<'_>, state: &State, toggle: look::MiniToggle) -> bool {
    match toggle {
        look::MiniToggle::FullMenu => true,
        look::MiniToggle::AllSets => game.map_option("all_waypoint_groups") == Some(1),
        look::MiniToggle::Backgrounds => game.map_option("world_waypoint_backgrounds") != Some(0),
        look::MiniToggle::Deathpoints => game.map_option("old_deathpoints") != Some(0),
        look::MiniToggle::CloseOnHop => state.close_on_hop,
    }
}

fn mini_toggle_label(toggle: look::MiniToggle) -> &'static str {
    match toggle {
        look::MiniToggle::FullMenu => "full_waypoint_menu",
        look::MiniToggle::AllSets => "all_sets",
        look::MiniToggle::Backgrounds => "waypoint_backgrounds",
        look::MiniToggle::Deathpoints => "old_deathpoints",
        look::MiniToggle::CloseOnHop => "close_on_hop",
    }
}

/// A black bar (a vanilla dropdown or value) with centred text.
fn draw_bar(game: &Game<'_>, batch: &mut HudBatch, rect: [f32; 4], text: &str, color: u32, hovered: bool) {
    game.native_fill(batch, rect, if hovered { 0xFFA0_A0A0 } else { look::BAR_EDGE });
    game.native_fill(batch, [rect[0] + 1., rect[1] + 1., rect[2] - 2., rect[3] - 2.], look::BAR);
    let text = fit_text(game, text, rect[2] - 6.);
    let width = game.map_text_width(&text);
    game.map_text(batch, &text, [(rect[0] + (rect[2] - width) * 0.5).round(), rect[1] + 2.], color, true);
}

/// A waypoint's initial square: its colour with the white initial, a skull for a death point.
fn draw_initial(game: &Game<'_>, batch: &mut HudBatch, square: [f32; 4], waypoint: &Waypoint, letter: &str) {
    let centre = [square[0] + square[2] * 0.5, square[1] + square[3] * 0.5];
    if waypoint.icon == runtime::WaypointIcon::Skull {
        game.native_fill(batch, square, look::DEATH_FILL);
        draw_skull(game, batch, centre, square[2] * 0.8);
        return;
    }
    let width = game.map_text_width(letter);
    let side = square[2].max(width + 2.);
    game.native_fill(batch, [centre[0] - side * 0.5, square[1], side, square[3]],
        waypoint_argb(if waypoint.color == 0 { 0xffffff } else { waypoint.color }));
    game.map_text(batch, letter, [(centre[0] - width * 0.5).round(), (centre[1] - 4.).round()], look::INITIAL_TEXT, true);
}

fn draw_mini_menu(game: &Game<'_>, batch: &mut HudBatch, state: &State, mouse: [f32; 2]) -> Option<Tip> {
    if state.map_panel != MapPanel::Waypoints {
        return None;
    }
    let rtl = runtime::right_to_left();
    let points = mini_waypoints(state);
    let fit = look::mini_menu_rows(batch.height);
    let paged = points.len() > fit;
    let room = if paged { fit.saturating_sub(2) } else { fit };
    let start = state.map_panel_scroll.min(points.len().saturating_sub(room));
    let shown = points.len().min(room) + if paged { 2 } else { 0 };
    let menu = look::mini_menu([batch.width, batch.height], shown, rtl);
    // Rows run bottom-up from the set bar; the first rect is the lowest.
    let mut rows = menu.rows.iter().copied();
    let mut tooltip = None;
    let row_text = |game: &Game<'_>, batch: &mut HudBatch, row: [f32; 4], text: &str, color: u32| {
        let text = fit_text(game, text, row[2] - 14.);
        let width = game.map_text_width(&text);
        let x = if rtl { row[0] + 12. } else { row[0] + row[2] - 12. - width };
        game.map_text(batch, &text, [x, row[1] + 1.], color, true);
    };
    if paged && let Some(row) = rows.next() {
        let color = if look::hit(mouse, row) { look::SCREEN_TEXT } else { look::GREY_TEXT };
        row_text(game, batch, row, &format!("[{}] ▽", ui("go_down")), color);
    }
    for ((index, point), row) in points.iter().skip(start).take(room).zip(rows.by_ref()) {
        let square = if rtl { [row[0], row[1], 9., 9.] } else { [row[0] + row[2] - 9., row[1], 9., 9.] };
        draw_initial(game, batch, square, point, &waypoint_text(state, *index, point));
        let hovered = look::hit(mouse, row);
        row_text(game, batch, row, &point.name, if hovered { 0xFFFF_FFA0 } else { look::SCREEN_TEXT });
        if hovered {
            tooltip = Some((vec![vec![(format!("{}, {}, {}", point.x, point.y, point.z), look::TOOLTIP_TEXT)]], row));
        }
    }
    if paged && let Some(row) = rows.next() {
        let color = if look::hit(mouse, row) { look::SCREEN_TEXT } else { look::GREY_TEXT };
        row_text(game, batch, row, &format!("[{}] △", ui("go_up")), color);
    }
    draw_bar(game, batch, menu.set, &group_label(state.waypoint_group.as_deref()), look::SCREEN_TEXT, look::hit(mouse, menu.set));
    draw_field(game, batch, menu.filter, &state.mini_filter, &ui("filter_waypoints"), state.mini_filter_focus);
    for (toggle, cell) in look::MINI_TOGGLES.into_iter().zip(menu.toggles) {
        let hovered = look::hit(mouse, cell);
        let color = if !mini_toggle_on(game, state, toggle) { look::GLYPH_OFF } else if hovered { look::GLYPH_HOVER } else { look::GLYPH };
        draw_glyph(game, batch, look::mini_glyph(toggle), cell, color);
        if hovered {
            let on = mini_toggle_on(game, state, toggle);
            let text = if toggle == look::MiniToggle::FullMenu { ui(mini_toggle_label(toggle)) }
                else { format!("{}: {}", ui(mini_toggle_label(toggle)), ui(if on { "on" } else { "off" })) };
            tooltip = Some((vec![vec![(text, look::TOOLTIP_TEXT)]], cell));
        }
    }
    tooltip
}

/// Handles a press on the mini-menu; true when it took it.
fn mini_menu_press(game: &Game<'_>, state: &mut State, mouse: [f32; 2], size: [f32; 2]) -> bool {
    if state.map_panel != MapPanel::Waypoints {
        return false;
    }
    let rtl = runtime::right_to_left();
    let points: Vec<_> = mini_waypoints(state).into_iter().map(|(_, p)| (p.key(), [f64::from(p.x) + 0.5, f64::from(p.z) + 0.5])).collect();
    let fit = look::mini_menu_rows(size[1]);
    let paged = points.len() > fit;
    let room = if paged { fit.saturating_sub(2) } else { fit };
    let start = state.map_panel_scroll.min(points.len().saturating_sub(room));
    let shown = points.len().min(room) + if paged { 2 } else { 0 };
    let menu = look::mini_menu(size, shown, rtl);
    state.mini_filter_focus = look::hit(mouse, menu.filter);
    runtime::set_text_input(state.mini_filter_focus);
    if state.mini_filter_focus {
        return true;
    }
    if look::hit(mouse, menu.set) {
        let groups = waypoint_groups(game, state);
        let i = groups.iter().position(|g| *g == state.waypoint_group).unwrap_or(0);
        state.waypoint_group = groups[(i + 1) % groups.len()].clone();
        runtime::initialize(game.native_data_dir(runtime::MOD_ID)).select_waypoint_group(&state.world, state.waypoint_group.as_deref());
        state.map_panel_scroll = 0;
        return true;
    }
    if let Some(toggle) = look::MINI_TOGGLES.into_iter().zip(menu.toggles).find(|(_, cell)| look::hit(mouse, *cell)).map(|(t, _)| t) {
        match toggle {
            look::MiniToggle::FullMenu => { state.mode = Mode::Waypoints; state.waypoint_scroll = 0; }
            look::MiniToggle::AllSets => toggle_setting(game, state, "all_waypoint_groups", false),
            look::MiniToggle::Backgrounds => toggle_setting(game, state, "world_waypoint_backgrounds", true),
            look::MiniToggle::Deathpoints => toggle_setting(game, state, "old_deathpoints", true),
            look::MiniToggle::CloseOnHop => state.close_on_hop = !state.close_on_hop,
        }
        return true;
    }
    let mut rows = menu.rows.iter().copied();
    if paged && let Some(row) = rows.next() {
        if look::hit(mouse, row) { state.map_panel_scroll = start + 1; return true; }
    }
    for ((key, at), row) in points.iter().skip(start).take(room).zip(rows.by_ref()) {
        if look::hit(mouse, row) {
            state.waypoint_selected = Some(*key);
            state.camera.snap(*at);
            state.elapsed = 1.;
            if state.close_on_hop { state.map_panel = MapPanel::None; }
            return true;
        }
    }
    if paged && let Some(row) = rows.next() {
        if look::hit(mouse, row) { state.map_panel_scroll = start.saturating_sub(1); return true; }
    }
    menu.rows.iter().any(|row| look::hit(mouse, *row))
}

fn map_zoom(game:&Game<'_>,state:&mut State,step:f64,offset:[f64;2]) {
    let min=if game.map_option("world_unlimited_zoom_in")==Some(1) {0.025}else{0.1};
    let max=if game.map_option("world_unlimited_zoom_out")==Some(1) {512.}else{64.};
    // `zoom_at_limit` doubles per unit; a wheel step multiplies by ZOOM_STEP.
    state.camera.zoom_at_limit(step*look::ZOOM_STEP.log2(),offset,min,max);
    state.elapsed=1.;
}

fn over_map_controls(game:&Game<'_>,state:&State,mouse: [f32; 2], width: f32, height: f32) -> bool {
    world_controls(game, [width, height]).into_iter().any(|(_, cell)| look::hit(mouse, cell))
        || state.map_panel == MapPanel::Waypoints && {
            let menu = look::mini_menu([width, height], look::mini_menu_rows(height), runtime::right_to_left());
            [menu.set, menu.filter].into_iter().chain(menu.toggles).chain(menu.rows).any(|r| look::hit(mouse, r))
        }
}

/// The zoom as Xaero writes it: up to three decimals, at least one (`3.0x`, `1.728x`).
fn zoom_text(zoom: f64) -> String {
    let text = format!("{zoom:.3}");
    let text = text.trim_end_matches('0');
    if text.ends_with('.') { format!("{text}0x") } else { format!("{text}x") }
}

fn draw_map_screen(game: &Game<'_>, batch: &mut HudBatch, state: &State) {
    let mouse = game.cursor();
    let rect = rectangle(batch.width, batch.height);
    let first_vertex=batch.vertices.len();
    draw(game,batch,state,rect,true,Some(mouse));
    if game.map_option("map_open_animation")==Some(1)
        && let Some(since)=state.map_opened {
        let percent=(since.elapsed().as_millis()*100/180).min(100) as u8;
        set_opacity(batch,first_vertex,percent);
    }
    let mut lines: Vec<(String, u32)> = Vec::new();
    let mut tooltip = None;
    if !over_map_controls(game,state,mouse, batch.width, batch.height)
        && let Some(frame) = state.frame.as_ref().filter(|f| {
            f.view.dimension == state.selected && f.view.large && f.view.session == state.session
        })
        && let Some(block) = hovered_block(&frame.view, rect, mouse)
    {
        let column = if state.dimension == frame.view.dimension {
            hovered_column(game, frame.view.layer, block)
        } else {
            match frame.view.layer {
                Layer::Below(maps::FULL_CAVE) => None,
                Layer::Below(y) => Some(ColumnInfo { height: y, fluid: None }),
                Layer::Surface => None,
            }
        };
        let coverage = runtime::initialize(game.native_data_dir(runtime::MOD_ID)).maps.coverage();
        let rough = if coverage.detailed_at(frame.view.dimension, frame.view.layer, block) {
            None
        } else {
            coverage.lod_sample(frame.view.dimension, frame.view.layer, block)
        };
        let detailed = coverage.detailed_biome(frame.view.dimension, frame.view.layer, block);
        let height = column
            .map(|column| column.height)
            .or_else(|| {
                (game.map_option("world_detect_ambiguous_y") == Some(0))
                    .then(|| coverage.detailed_height(frame.view.dimension, frame.view.layer, block))
                    .flatten()
            })
            .or_else(|| rough.as_ref().map(|p| p.height));
        if game.map_option("map_show_coordinates") != Some(0) {
            lines.push((match height {
                Some(y) => format!("X: {} Y: {} Z: {}", block[0], y, block[1]),
                None => format!("X: {} Z: {}", block[0], block[1]),
            }, look::SCREEN_TEXT));
        }
        if game.map_option("world_hover_biome") != Some(0) {
            let mut biome = None;
            if state.dimension == frame.view.dimension
                && let Some(y) = height.or_else(|| game.native_player().map(|p| p.y.floor() as i32)) {
                biome = known_biome(game, state.dimension, [block[0], y, block[1]]);
            }
            let biome = biome.or_else(|| detailed.as_ref()
                .or_else(|| rough.as_ref().and_then(|sample| sample.biome.as_ref()))
                .map(|name| name.to_string()));
            if let Some(biome) = biome {
                lines.push((localized_biome(&biome), look::SCREEN_TEXT));
            }
        }
        if !coverage.detailed_at(frame.view.dimension, frame.view.layer, block)
            && coverage.lod_at(frame.view.dimension, frame.view.layer, block).is_some()
        {
            lines.push((ui("approximate"), look::GREY_TEXT));
        }
        if let Some(waypoint)=waypoint_at_pointer(game,state,&frame.view,rect,mouse) {
            let at = world_screen(&frame.view, rect, [f64::from(waypoint.x) + 0.5, f64::from(waypoint.z) + 0.5]);
            let (body, _) = look::pennant(at, game.map_number("world_waypoint_symbol_scale").unwrap_or(1.) as f32);
            tooltip=Some((vec![vec![(if game.map_option("waypoint_hide_coords")==Some(1) {waypoint.name.clone()}
                else {format!("{}  ({} {} {})",waypoint.name,waypoint.x,waypoint.y,waypoint.z)}, look::TOOLTIP_TEXT)]],
                [body[0], body[1], body[2], at[1] - body[1]]));
        }
    }
    if tooltip.is_none() && !over_map_controls(game,state,mouse, batch.width, batch.height)
        && let Some(group) = RADAR_HOVER.lock().unwrap_or_else(|error| error.into_inner()).take() {
        tooltip = Some(group);
    }
    if game.map_option("world_cave_ceiling")==Some(1)
        && let Some(Layer::Below(y))=state.frame.as_ref().map(|frame|frame.view.layer) {
        lines.push((if y == maps::FULL_CAVE { ui("full_caves") } else { format!("Y ≤ {y}") }, look::GREY_TEXT));
    }
    let widths: Vec<f32> = lines.iter().map(|(text, _)| game.map_text_width(text)).collect();
    for ((text, color), at) in lines.iter().zip(look::cursor_lines([batch.width, batch.height], &widths)) {
        game.map_text(batch, text, [at[0], at[1]], *color, true);
    }
    if game.map_option("world_zoom_readout") != Some(0) {
        let text = zoom_text(1. / state.camera.zoom);
        let (zoom_box, at) = look::zoom_label([batch.width, batch.height], game.map_text_width(&text));
        game.native_fill(batch, zoom_box, look::ZOOM_BOX);
        draw_text(game, batch, &text, at, look::SCREEN_TEXT, true);
    }
    if state.info.is_none() && state.menu.is_none() {
        if let Some(lines) = draw_world_controls(game, batch, state, mouse) { tooltip = Some(lines); }
        if let Some(lines) = draw_mini_menu(game, batch, state, mouse) { tooltip = Some(lines); }
    }
    draw_menu(game, batch, state);
    draw_notice(game, batch, state);
    if let Some((lines, hovered)) = tooltip && state.info.is_none() && state.menu.is_none() {
        draw_tooltip_lines(game, batch, &lines, mouse, hovered);
    }
    if let Some(info) = state.info {
        draw_information(game, batch, state, info);
    }
}

fn cache_rows(height: f32) -> usize {
    (((height - 122.).max(0.) / 48.) as usize).clamp(1, 16)
}
fn cache_row(width: f32, index: usize) -> [f32; 4] {
    [12., 42. + index as f32 * 48., (width - 24.).max(1.), 46.]
}
fn cache_footer(width: f32, height: f32, index: usize) -> [f32; 4] {
    let w = ((width - 32.) / 3.).max(1.);
    let rect = [12. + index as f32 * (w + 4.), height - 55., w, 18.];
    if runtime::right_to_left() { look::mirror(rect, width) } else { rect }
}
fn cache_refresh(width: f32) -> [f32; 4] {
    let rect = [width - 92., 10., 80., 18.];
    if runtime::right_to_left() { look::mirror(rect, width) } else { rect }
}
/// Text at `inset` from the reading start of `row`: its left edge, or its right edge right to
/// left.
fn text_in(game: &Game<'_>, batch: &mut HudBatch, text: &str, row: [f32; 4], inset: f32, y: f32, color: u32) {
    let x = if runtime::right_to_left() { row[0] + row[2] - inset - game.map_text_width(text) } else { row[0] + inset };
    draw_text(game, batch, text, [x, y], color, false);
}
fn cache_status(status: &runtime::container::Compatibility) -> &'static str {
    use runtime::container::Compatibility::*;
    match status {
        Current => tr(
            "Compatible",
            "Совместим",
            "Compatible",
            "Compatible",
            "Kompatibel",
        ),
        RebuildImages => tr(
            "Update available: rebuild map images",
            "Доступно обновление: перестройте изображения карты",
            "Mise à jour : reconstruire les images",
            "Actualización: reconstruir imágenes",
            "Update: Kartenbilder neu aufbauen",
        ),
        Older(_) => tr(
            "Old format: conversion is not available yet",
            "Старый формат: конвертация пока недоступна",
            "Ancien format : conversion indisponible",
            "Formato antiguo: conversión no disponible",
            "Altes Format: Konvertierung noch nicht verfügbar",
        ),
        Newer(_) => tr(
            "Requires a newer mod version",
            "Нужна более новая версия мода",
            "Version plus récente du mod requise",
            "Requiere una versión más reciente del mod",
            "Neuere Mod-Version erforderlich",
        ),
        Damaged => tr(
            "Header unreadable; file preserved",
            "Заголовок не читается; файл сохранён",
            "En-tête illisible ; fichier conservé",
            "Cabecera ilegible; archivo conservado",
            "Header unlesbar; Datei erhalten",
        ),
    }
}
fn cache_text(game: &Game<'_>, value: &str, width: f32) -> String {
    let mut text: String = value
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect();
    if game.map_text_width(&text) <= width {
        return text;
    }
    while !text.is_empty() && game.map_text_width(&format!("{text}…")) > width {
        text.pop();
    }
    format!("{text}…")
}
fn cache_icon(bytes: &[u8]) -> Option<Arc<Vec<u8>>> {
    // Validate size before the PNG decoder allocates its output.
    if bytes.len() > 1024 * 1024
        || bytes.len() < 24
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || &bytes[12..16] != b"IHDR"
        || u32::from_be_bytes(bytes[16..20].try_into().ok()?) != 64
        || u32::from_be_bytes(bytes[20..24].try_into().ok()?) != 64
    {
        return None;
    }
    horizon_client::hud::decode_bytes(bytes)
        .ok()
        .filter(|(w, h, p)| *w == 64 && *h == 64 && p.len() == 64 * 64 * 4)
        .map(|(_, _, p)| Arc::new(p))
}
fn refresh_cache_catalog(game: &Game<'_>) {
    let controller = runtime::initialize(game.native_data_dir(runtime::MOD_ID));
    controller.caches.refresh(|rows| {
        use std::io::Read;
        let saves = std::env::current_dir()?.join("saves");
        let (servers, hidden) =
            horizon_protocol::servers::load_all(&horizon_client::multiplayer::servers_file(&saves));
        let servers: Vec<_> = servers.into_iter().chain(hidden).collect();
        for row in rows {
            let Some(world) = row.world.as_deref() else {
                continue;
            };
            if let Some(folder) = world.strip_prefix("save:") {
                let mut parts = std::path::Path::new(folder).components();
                if !matches!(parts.next(), Some(std::path::Component::Normal(_)))
                    || parts.next().is_some()
                    || folder.contains('\\')
                {
                    continue;
                }
                let root = saves.join(folder);
                if let Some(level) = horizon_client::save::Level::read_summary(&root) {
                    row.name = level.name;
                }
                if let Ok(file) = std::fs::File::open(root.join("icon.png")) {
                    let mut bytes = Vec::new();
                    if file.take(1024 * 1024 + 1).read_to_end(&mut bytes).is_ok() {
                        row.icon = cache_icon(&bytes);
                    }
                }
            } else if let Some(address) = world.strip_prefix("server:")
                && let Some(server) = servers.iter().find(|s| s.ip == address)
            {
                if !server.name.is_empty() {
                    row.name = server.name.clone();
                }
                row.icon = server
                    .icon
                    .as_deref()
                    .filter(|s| s.len() <= 1400000)
                    .and_then(horizon_client::skins::base64_decode)
                    .and_then(|png| cache_icon(&png));
            }
        }
        Ok(())
    });
}
fn show_cache_manager(game: &Game<'_>, state: &mut State) {
    state.mode = Mode::Caches;
    state.menu = None;
    state.info = None;
    state.name_form = None;
    runtime::set_text_input(false);
    refresh_cache_catalog(game);
}
pub(super) fn open_cache_manager(game: &mut Game<'_>) {
    if game.native_player().is_none() || game.title.is_some() {
        return;
    }
    if game.screen == super::Screen::Options {
        game.settings.save(&Game::options_path());
        game.screen = super::Screen::Playing;
    }
    if game.mod_screen_open(runtime::MOD_ID) {
        show_cache_manager(game, &mut state());
    }
}
fn update_cache_ui(game: &Game<'_>, state: &mut State) {
    let snapshot = runtime::initialize(game.native_data_dir(runtime::MOD_ID))
        .caches
        .snapshot();
    if state.caches.catalog_revision != snapshot.revision {
        state.caches.catalog_revision = snapshot.revision;
        let file = runtime::container::world_path(std::path::Path::new(""), &state.world);
        state.caches.attention = snapshot.entries.iter().any(|e| {
            (e.world.as_ref() == Some(&state.world)
                || file.file_name().is_some_and(|f| f == e.file.as_str()))
                && e.compatibility != runtime::container::Compatibility::Current
        });
    }
    if state.mode != Mode::Caches {
        return;
    }
    if let Some(rx) = &state.caches.operation {
        match rx.try_recv() {
            Ok(result) => {
                state.caches.operation = None;
                state.caches.message = Some(match result {
                    Ok(()) => tr(
                        "Rebuild scheduled; images update when this cache is viewed",
                        "Перестроение назначено; изображения обновятся при просмотре кеша",
                        "Reconstruction planifiée à la prochaine consultation",
                        "Reconstrucción programada al consultar la caché",
                        "Neuaufbau beim Anzeigen geplant",
                    )
                    .into(),
                    Err(error) => error,
                });
                refresh_cache_catalog(game);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                state.caches.operation = None;
                state.caches.message = Some(
                    tr(
                        "Cache worker stopped",
                        "Поток кеша остановлен",
                        "Gestion du cache arrêtée",
                        "Gestor detenido",
                        "Cache-Verarbeitung beendet",
                    )
                    .into(),
                );
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
    if state.caches.revision != snapshot.revision {
        state.caches.revision = snapshot.revision;
        state.caches.confirm = None;
        state.caches.icons.clear();
        if state
            .caches
            .selected
            .as_ref()
            .is_some_and(|name| !snapshot.entries.iter().any(|e| e.file == *name))
        {
            state.caches.selected = None;
        }
    }
    let height = game.map_client_state().map_or(480., |s| s.gui_size[1]);
    let start = state
        .caches
        .scroll
        .min(snapshot.entries.len().saturating_sub(cache_rows(height)));
    let visible: BTreeSet<_> = snapshot
        .entries
        .iter()
        .skip(start)
        .take(cache_rows(height))
        .map(|e| e.file.as_str())
        .collect();
    state
        .caches
        .icons
        .retain(|file, _| visible.contains(file.as_str()));
    for entry in snapshot.entries.iter().skip(start).take(cache_rows(height)) {
        if let Some(pixels) = &entry.icon {
            state
                .caches
                .icons
                .entry(entry.file.clone())
                .or_insert_with(|| super::minimap_pages::create(64, 64, (**pixels).clone()));
        }
    }
}
fn draw_cache_screen(game: &Game<'_>, batch: &mut HudBatch, state: &State) {
    let snapshot = runtime::initialize(game.native_data_dir(runtime::MOD_ID))
        .caches
        .snapshot();
    draw_backdrop(game, batch, state);
    screen_title(
        game,
        batch,
        tr(
            "Map caches",
            "Кеши карт",
            "Caches des cartes",
            "Cachés de mapas",
            "Karten-Caches",
        ),
        14.,
    );
    button(
        game,
        batch,
        cache_refresh(batch.width),
        tr(
            "Refresh",
            "Обновить",
            "Actualiser",
            "Actualizar",
            "Aktualisieren",
        ),
        game.cursor(),
        !snapshot.loading && state.caches.operation.is_none(),
    );
    let start = state.caches.scroll.min(
        snapshot
            .entries
            .len()
            .saturating_sub(cache_rows(batch.height)),
    );
    for (i, entry) in snapshot
        .entries
        .iter()
        .skip(start)
        .take(cache_rows(batch.height))
        .enumerate()
    {
        let row = cache_row(batch.width, i);
        if state.caches.selected.as_ref() == Some(&entry.file) {
            game.native_fill(batch, row, look::SELECTED_BORDER);
            game.native_fill(batch, [row[0] + 1., row[1] + 1., row[2] - 2., row[3] - 2.], look::SELECTED_ROW);
        } else {
            game.native_fill(batch, row, look::LIST_BAND);
        }
        let icon_x = if runtime::right_to_left() { row[0] + row[2] - 38. } else { row[0] + 6. };
        if let Some(icon) = state.caches.icons.get(&entry.file) {
            let x = f64::from(icon_x);
            let y = f64::from(row[1] + 6.);
            super::minimap_pages::paint(
                batch,
                icon,
                &[
                    [x, y, 0., 0.],
                    [x + 32., y, 64., 0.],
                    [x + 32., y + 32., 64., 64.],
                    [x, y + 32., 0., 64.],
                ],
            );
        } else {
            game.native_fill(batch, [icon_x, row[1] + 6., 32., 32.], 0xff506846);
            game.map_text(
                batch,
                if entry
                    .world
                    .as_ref()
                    .is_some_and(|w| w.starts_with("server:"))
                {
                    "S"
                } else {
                    "W"
                },
                [icon_x + 12., row[1] + 18.],
                0xffffff,
                true,
            );
        }
        let server = entry.world.as_deref().is_some_and(|world| world.starts_with("server:"));
        let mask = game.map_option("hide_world_names").unwrap_or(0);
        let hidden = mask >= 2 || (mask == 1 && server);
        let label = if hidden {
            if server { tr("Server", "Сервер", "Serveur", "Servidor", "Server") }
            else { tr("World", "Мир", "Monde", "Mundo", "Welt") }
        } else { &entry.name };
        let title = cache_text(game, label, row[2] - 52.);
        text_in(game, batch, &title, row, 44., row[1] + 4., look::SCREEN_TEXT);
        let size = entry.bytes as f64 / 1048576.;
        let description = if hidden { format!("{size:.2} MiB") } else { format!("{size:.2} MiB · {}", entry.file) };
        let detail = cache_text(game, &description, row[2] - 52.);
        text_in(game, batch, &detail, row, 44., row[1] + 17., look::GREY_TEXT);
        let label = cache_text(game, cache_status(&entry.compatibility), row[2] - 52.);
        text_in(game, batch, &label, row, 44., row[1] + 30.,
            if entry.compatibility == runtime::container::Compatibility::Current { 0xff9f_c792 } else { 0xffff_d24a });
    }
    if snapshot.entries.is_empty() {
        game.map_text(
            batch,
            tr(
                "No map cache files",
                "Файлов кеша карт нет",
                "Aucun cache",
                "No hay cachés",
                "Keine Karten-Caches",
            ),
            [if runtime::right_to_left() { batch.width - 12. - game.map_text_width(tr(
                "No map cache files", "Файлов кеша карт нет", "Aucun cache", "No hay cachés", "Keine Karten-Caches")) } else { 12. }, 46.],
            0xbac4d0,
            false,
        );
    }
    let busy = snapshot.loading || state.caches.operation.is_some();
    let selected = snapshot
        .entries
        .iter()
        .find(|e| Some(&e.file) == state.caches.selected.as_ref());
    if let Some(entry) = selected {
        let identity = cache_text(
            game,
            &format!(
                "{} · {}",
                entry.name,
                entry.world.as_deref().unwrap_or(&entry.file)
            ),
            batch.width - 24.,
        );
        text_in(game, batch, &identity, [0., 0., batch.width, batch.height], 12., batch.height - 73., look::GREY_TEXT);
    }
    let can_rebuild = selected.is_some_and(|e| {
        e.world.is_some()
            && matches!(
                e.compatibility,
                runtime::container::Compatibility::Current
                    | runtime::container::Compatibility::RebuildImages
            )
    });
    button(
        game,
        batch,
        cache_footer(batch.width, batch.height, 0),
        if state.caches.confirm.is_some() {
            tr(
                "Confirm rebuild",
                "Подтвердить",
                "Confirmer",
                "Confirmar",
                "Bestätigen",
            )
        } else {
            tr(
                "Rebuild images",
                "Перестроить",
                "Reconstruire",
                "Reconstruir",
                "Neu aufbauen",
            )
        },
        game.cursor(),
        can_rebuild && !busy,
    );
    button(
        game,
        batch,
        cache_footer(batch.width, batch.height, 1),
        tr(
            "Compress: later",
            "Сжать: позже",
            "Compresser : plus tard",
            "Comprimir: después",
            "Komprimieren: später",
        ),
        game.cursor(),
        false,
    );
    button(
        game,
        batch,
        cache_footer(batch.width, batch.height, 2),
        tr("Back to map", "К карте", "Carte", "Mapa", "Zur Karte"),
        game.cursor(),
        true,
    );
    let message = state
        .caches
        .message
        .as_deref()
        .or(snapshot.error.as_deref())
        .unwrap_or(if busy {
            tr(
                "Working…",
                "Обработка…",
                "En cours…",
                "Procesando…",
                "Verarbeitung…",
            )
        } else if state.caches.confirm.is_some() {
            tr(
                "Only images are rebuilt; saved terrain is retained",
                "Перестроятся только изображения; исходная карта сохранится",
                "Seules les images changent, le terrain est conservé",
                "Solo se reconstruyen imágenes; se conserva el terreno",
                "Nur Bilder werden neu aufgebaut; Gelände bleibt erhalten",
            )
        } else {
            tr(
                "Select a cache. Compression and file conversion are not available yet",
                "Выберите кеш. Сжатие и конвертация файлов пока недоступны",
                "Sélectionnez un cache. Compression et conversion à venir",
                "Selecciona una caché. Compresión y conversión pendientes",
                "Cache auswählen. Komprimierung und Konvertierung folgen",
            )
        });
    let message = cache_text(game, message, batch.width - 24.);
    let screen = [0., 0., batch.width, batch.height];
    text_in(game, batch, &message, screen, 12., batch.height - 29., 0xffff_d24a);
    if snapshot.truncated {
        text_in(game, batch, "4096+", screen, 12., batch.height - 15., 0xffff_d24a);
    }
}
fn cache_press(
    game: &Game<'_>,
    state: &mut State,
    screen: &mod_api::ScreenView<'_>,
    button_id: i32,
    canceled: &mut bool,
) {
    *canceled = true;
    if button_id != 0 {
        return;
    }
    let controller = runtime::initialize(game.native_data_dir(runtime::MOD_ID));
    let snapshot = controller.caches.snapshot();
    if inside(
        screen.mouse,
        cache_footer(screen.rect[2], screen.rect[3], 2),
    ) {
        state.mode = Mode::Map;
        state.caches.confirm = None;
        return;
    }
    if snapshot.loading || state.caches.operation.is_some() {
        return;
    }
    if inside(screen.mouse, cache_refresh(screen.rect[2])) {
        state.caches.confirm = None;
        refresh_cache_catalog(game);
        return;
    }
    if inside(
        screen.mouse,
        cache_footer(screen.rect[2], screen.rect[3], 0),
    ) {
        if let Some(entry) = snapshot
            .entries
            .iter()
            .find(|e| Some(&e.file) == state.caches.selected.as_ref())
            && matches!(
                entry.compatibility,
                runtime::container::Compatibility::Current
                    | runtime::container::Compatibility::RebuildImages
            )
            && let Some(world) = entry.world.as_deref()
        {
            if state.caches.confirm.as_deref() == Some(&entry.file) {
                state.caches.confirm = None;
                match controller.maps.rebuild_images(world, &entry.file) {
                    Ok(rx) => {
                        state.caches.operation = Some(rx);
                        state.caches.message = None;
                    }
                    Err(error) => state.caches.message = Some(error),
                }
            } else {
                state.caches.confirm = Some(entry.file.clone());
                state.caches.message = None;
            }
        }
        return;
    }
    let start = state.caches.scroll.min(
        snapshot
            .entries
            .len()
            .saturating_sub(cache_rows(screen.rect[3])),
    );
    for (i, entry) in snapshot
        .entries
        .iter()
        .skip(start)
        .take(cache_rows(screen.rect[3]))
        .enumerate()
    {
        if inside(screen.mouse, cache_row(screen.rect[2], i)) {
            state.caches.selected = Some(entry.file.clone());
            state.caches.confirm = None;
            state.caches.message = None;
            break;
        }
    }
}

// Only the map and its name prompt cover the world. Other screens keep
// Horizon's normal rendering and overlay priorities.
fn map_covers_world(state: &State) -> bool {
    state.mode == Mode::Map
        || (state.mode == Mode::Name
            && state
                .name_form
                .as_ref()
                .is_some_and(|form| form.background == Mode::Map))
}

pub(super) fn covers_world(game: &Game<'_>) -> bool {
    game.map_on(runtime::MOD_ID)
        && game.mod_screen() == Some(runtime::MOD_ID)
        && game.native_player().is_some()
        && map_covers_world(&state())
}

pub(super) fn screen(game: &Game<'_>, batch: &mut HudBatch) {
    if game.mod_screen() != Some(runtime::MOD_ID) || !game.map_on(runtime::MOD_ID) {
        return;
    }
    let state = state();
    match state.mode {
        Mode::Map => draw_map_screen(game, batch, &state),
        Mode::Waypoints => draw_waypoints_screen(game, batch, &state),
        Mode::Settings => draw_settings(game,batch,&state),
        Mode::Name => {
            if let Some(form) = state.name_form.as_ref() {
                draw_name_form(game, batch, &state, form);
            }
        }
        Mode::Caches => draw_cache_screen(game, batch, &state),
    }
}

pub(super) fn press(
    game: &Game<'_>,
    screen: &mod_api::ScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    if game.mod_screen() != Some(runtime::MOD_ID) || !game.map_on(runtime::MOD_ID) {
        return;
    }
    let chosen = game.map_key(
        runtime::MOD_ID,
        runtime::WORLD_MAP_KEY,
        runtime::WORLD_MAP_DEFAULT,
    );
    let mouse = match button {
        0 => "key.mouse.left",
        1 => "key.mouse.right",
        2 => "key.mouse.middle",
        _ => "",
    };
    if chosen == mouse {
        state().close = true;
        *canceled = true;
        return;
    }
    let mut state = state();
    if state.info.is_some() {
        if button == 0 {
            let rect = information_rect(screen.rect[2], screen.rect[3]);
            let close = information_close(rect);
            if !inside(screen.mouse, rect) || inside(screen.mouse, close) {
                state.info = None;
            }
        }
        *canceled = true;
        return;
    }
    match state.mode {
        Mode::Map => map_press(game, &mut state, screen, button, canceled),
        Mode::Waypoints => waypoint_press(game, &mut state, screen, button, canceled),
        Mode::Settings => { settings_press(game,&mut state,screen,button); *canceled=true; }
        Mode::Name => name_press(game, &mut state, screen, button, canceled),
        Mode::Caches => cache_press(game, &mut state, screen, button, canceled),
    }
}

fn map_press(
    game: &Game<'_>,
    state: &mut State,
    screen: &mod_api::ScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    let size = [screen.rect[2], screen.rect[3]];
    let mouse = screen.mouse;
    if state.menu.is_none() {
        if let Some((control, _)) = world_controls(game, size).into_iter().find(|(_, cell)| look::hit(mouse, *cell)) {
            if button == 0 {
                press_world_control(game, state, control);
            }
            state.dragging = false;
            *canceled = true;
            return;
        }
        if button == 0 && mini_menu_press(game, state, mouse, size) {
            state.dragging = false;
            *canceled = true;
            return;
        }
    }
    if button == 1 {
        let menu = menu_target_at(game, state, screen, mouse);
        state.menu = menu;
        *canceled = true;
        return;
    }
    if button != 0 {
        return;
    }
    let width = screen.rect[2];
    state.dragging = false;
    if let Some(menu) = state.menu {
        let Some(player) = game.native_player() else {
            state.menu = None;
            return;
        };
        let rows = menu_rows(game, state, menu, &player);
        let (outer, _, rects) = menu_layout(size, menu, rows.len());
        if look::hit(mouse, outer) {
            if let Some(action) = rows.iter().zip(rects).find(|(_, rect)| look::hit(mouse, *rect)).and_then(|(row, _)| row.action) {
                perform_menu(game, state, menu, action);
            }
            *canceled = true;
            return;
        }
        state.menu = None;
    }
    let portal_click = state
        .frame
        .as_ref()
        .filter(|frame| {
            frame.view.dimension == state.selected
                && frame.view.large
                && frame.view.session == state.session
        })
        .and_then(|frame| {
            hovered_block(&frame.view, rectangle(width, screen.rect[3]), mouse)
        })
        .is_some_and(|block| portal_near(state, state.selected, block).is_some());
    if portal_click {
        let menu = menu_target_at(game, state, screen, mouse);
        state.menu = menu;
        *canceled = true;
        return;
    }
    if inside(mouse, rectangle(width, screen.rect[3])) {
        state.dragging = true;
        *canceled = true;
    }
}

fn menu_target_at(
    game: &Game<'_>,
    state: &State,
    screen: &mod_api::ScreenView<'_>,
    at: [f32; 2],
) -> Option<Menu> {
    let rect = rectangle(screen.rect[2], screen.rect[3]);
    let frame = state.frame.as_ref().filter(|frame| {
        frame.view.dimension == state.selected
            && frame.view.large
            && frame.view.session == state.session
    })?;
    let block = hovered_block(&frame.view, rect, at)?;
    let portal = portal_near(state, state.selected, block);
    let column = if state.dimension == frame.view.dimension {
        hovered_column(game, frame.view.layer, block)
    } else {
        match frame.view.layer {
            Layer::Below(maps::FULL_CAVE) => None,
            Layer::Below(y) => Some(ColumnInfo {
                height: y,
                fluid: None,
            }),
            Layer::Surface => None,
        }
    };
    let height = if let Some((_, at)) = portal {
        Some(at[1])
    } else {
        column.map(|column| column.height)
    };
    let fluid = if portal.is_some() {
        None
    } else {
        column.and_then(|column| column.fluid)
    };
    let coverage = runtime::initialize(game.native_data_dir(runtime::MOD_ID)).maps.coverage();
    let height = height.or_else(|| {
        (game.map_option("world_detect_ambiguous_y") == Some(0))
            .then(|| coverage.detailed_height(state.selected, frame.view.layer, block))
            .flatten()
    });
    let rough = coverage.lod_sample(state.selected, frame.view.layer, block);
    let height_approximate = height.is_none() && rough.is_some();
    let height = height.or_else(|| rough.map(|p| p.height));
    let radar_player = ::map::radar::snapshot()
        .filter(|radar| radar.world_map && radar.dimension == frame.view.dimension.id())
        .and_then(|radar| radar.dots.iter().filter_map(|dot| {
            game.remote_player_name(dot.entity_id)?;
            let point = world_screen(&frame.view, rect, [dot.at[0], dot.at[2]]);
            let distance = ((point[0] - at[0]).powi(2) + (point[1] - at[1]).powi(2)).sqrt();
            (distance <= 9.).then_some((distance, dot.entity_id))
        }).min_by(|a,b| a.0.total_cmp(&b.0)).map(|(_,id)| id));
    Some(Menu {
        height_approximate,
        block,
        height,
        fluid,
        dimension: state.selected,
        at,
        portal: portal.map(|(kind, _)| kind),
        radar_player,
        waypoint: waypoint_at_pointer(game,state,&frame.view,rect,at)
            .map(|waypoint| Spot {
                dimension: waypoint.dimension,
                x: waypoint.x,
                z: waypoint.z,
                key: waypoint.key(),
            }),
    })
}

fn edit_waypoint(state: &mut State, key: u64) {
    let Some(waypoint) = state.waypoints.iter().find(|waypoint| waypoint.key() == key) else {
        return;
    };
    let mut form = NameForm::new(state, MarkerMode::Saved, state.mode);
    form.text = waypoint.name.clone();
    form.color = waypoint.color;
    form.rename = Some(Spot { dimension: waypoint.dimension, x: waypoint.x, z: waypoint.z, key });
    form.group = waypoint.options.group.clone();
    form.dimension = waypoint.dimension;
    form.coords = [
        waypoint.x.to_string(),
        waypoint.y.to_string(),
        waypoint.z.to_string(),
        if waypoint.options.rotation { waypoint.options.yaw.to_string() } else { String::new() },
    ];
    form.initials = waypoint.initials.clone().unwrap_or_default();
    form.show_3d = waypoint.show_3d;
    form.disabled = waypoint.options.disabled;
    state.name_form = Some(form);
    state.mode = Mode::Name;
    runtime::set_text_input(true);
}

fn create_waypoint_here(game: &Game<'_>, state: &mut State) {
    let Some(player) = game.native_player() else { return; };
    let form = NameForm::new(state, MarkerMode::Saved, state.mode).at(Target {
        dimension: Dimension::from_id(player.dimension),
        x: player.x.floor() as i32,
        y: player.y.floor() as i32,
        z: player.z.floor() as i32,
    });
    state.name_form = Some(form);
    state.mode = Mode::Name;
    runtime::set_text_input(true);
}

fn teleport_to_waypoint(game: &Game<'_>, state: &mut State, key: u64) {
    let Some(player) = game.native_player() else { return };
    let Some(point) = state.waypoints.iter().find(|point| point.key() == key) else { return };
    let y = if point.options.unspecified_height { player.y.floor() as i32 } else { point.y };
    state.pending_command = configured_teleport(game, point.dimension, Dimension::from_id(player.dimension),
        Some(point), [point.x, y, point.z], point.options.unspecified_height);
    state.close = state.pending_command.is_some();
}

fn paste_waypoint(game: &Game<'_>, state: &mut State) {
    let clipboard = game.clipboard.text();
    let dimension = game.native_player().map_or(Dimension::Overworld, |player| Dimension::from_id(player.dimension));
    let controller = runtime::initialize(game.native_data_dir(runtime::MOD_ID));
    let message = match runtime::receive_shared_waypoint(&clipboard, &state.world, dimension) {
        Some(point) => match controller.merge_waypoints(&state.world, vec![point]) {
            Ok(_) => ui("waypoint_imported"),
            Err(error) => error,
        },
        None => ui("no_waypoint_link"),
    };
    state.notice = Some((message, Instant::now()));
}

fn waypoint_press(
    game: &Game<'_>,
    state: &mut State,
    screen: &mod_api::ScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    if button != 0 {
        return;
    }
    *canceled = true;
    let size = [screen.rect[2], screen.rect[3]];
    let mouse = screen.mouse;
    let controller = runtime::initialize(game.native_data_dir(runtime::MOD_ID));
    let (list, rows) = waypoint_screen(game, state, size);
    let rows: Vec<(u64, [f32; 4])> = rows.into_iter().map(|(_, point, row)| (point.key(), row)).collect();
    if state.waypoint_groups_open {
        state.waypoint_groups_open = false;
        if let Some((row, _)) = set_rows(game, state, list.set, list.actions[0][1]).into_iter().find(|(_, rect)| look::hit(mouse, *rect)) {
            match row {
                SetRow::Select(group) => {
                    state.waypoint_group = group;
                    state.waypoint_scroll = 0;
                    state.waypoint_selected = None;
                    controller.select_waypoint_group(&state.world, state.waypoint_group.as_deref());
                }
                SetRow::Rename(name) => {
                    let mut form = NameForm::new(state, MarkerMode::Saved, Mode::Waypoints);
                    form.text = name.clone();
                    form.group_edit = Some(GroupEdit::Rename(name));
                    state.name_form = Some(form);
                    state.mode = Mode::Name;
                    runtime::set_text_input(true);
                }
                SetRow::DeleteEmpty(name) => {
                    if controller.remove_empty_waypoint_group(&state.world, &name) {
                        state.waypoint_group = None;
                        state.waypoint_scroll = 0;
                        state.waypoint_selected = None;
                    }
                }
            }
        }
        return;
    }
    state.waypoint_search_focus = look::hit(mouse, list.filter);
    runtime::set_text_input(state.waypoint_search_focus);
    if state.waypoint_search_focus {
        return;
    }
    if look::hit(mouse, list.bars[0]) {
        let i = Dimension::ALL.iter().position(|d| *d == state.waypoint_dimension).unwrap_or(0);
        state.waypoint_dimension = Dimension::ALL[(i + 1) % Dimension::ALL.len()];
        state.waypoint_scroll = 0;
        state.waypoint_selected = None;
        return;
    }
    if look::hit(mouse, list.bars[1]) {
        state.waypoint_sort = (state.waypoint_sort + 1) % 3;
        state.waypoint_scroll = 0;
        return;
    }
    if look::hit(mouse, list.options) {
        open_settings(state, Some(Mode::Waypoints), Some(2));
        return;
    }
    if look::hit(mouse, list.set) {
        state.waypoint_groups_open = true;
        state.waypoint_groups_scroll = 0;
        return;
    }
    if look::hit(mouse, list.new_set) {
        let mut form = NameForm::new(state, MarkerMode::Saved, Mode::Waypoints);
        form.text = String::new();
        form.group_edit = Some(GroupEdit::Create);
        state.name_form = Some(form);
        state.mode = Mode::Name;
        runtime::set_text_input(true);
        return;
    }
    if look::hit(mouse, list.done) {
        state.close = true;
        return;
    }
    if let Some((key, _)) = rows.iter().find(|(_, row)| look::hit(mouse, *row)) {
        // A second click on the selected row shows it on the map.
        if state.waypoint_selected == Some(*key) {
            if let Some(dimension) = show_selected_waypoint(state) {
                controller.choose_dimension(true, dimension);
            }
        } else {
            state.waypoint_selected = Some(*key);
        }
        return;
    }
    let [edit, teleport, share, enable, delete] = list.actions;
    let selected = selected_waypoint(state).map(|point| (point.key(), point.share_link()));
    if look::hit(mouse, edit) {
        match selected {
            Some((key, _)) => edit_waypoint(state, key),
            None => create_waypoint_here(game, state),
        }
        return;
    }
    let Some((key, link)) = selected else { return };
    if look::hit(mouse, teleport) && teleport_allowed(game) {
        teleport_to_waypoint(game, state, key);
    } else if look::hit(mouse, share) {
        state.pending_copy = Some(link);
        state.notice = Some((ui("link_copied"), Instant::now()));
    } else if look::hit(mouse, enable) {
        controller.change_waypoint(&state.world, key, |point| point.options.disabled = !point.options.disabled);
    } else if look::hit(mouse, delete) {
        controller.delete_waypoint(&state.world, key);
        state.notice = Some((ui("waypoint_deleted"), Instant::now()));
        state.waypoint_selected = None;
    }
}

fn name_press(
    game: &Game<'_>,
    state: &mut State,
    screen: &mod_api::ScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) {
    if !matches!(button, 0 | 1) {
        return;
    }
    *canceled = true;
    let layout = look::waypoint_form([screen.rect[2], screen.rect[3]], runtime::right_to_left());
    let mouse = screen.mouse;
    let groups = waypoint_groups(game, state);
    let step = |len: usize, i: usize| if button == 0 { (i + 1) % len } else { (i + len - 1) % len };
    let Some(valid) = state.name_form.as_ref().map(NameForm::valid) else { return };
    if button == 0 && look::hit(mouse, layout.confirm) {
        if valid { save_name(game, state); }
        return;
    }
    if button == 0 && look::hit(mouse, layout.cancel) {
        cancel_name(state);
        return;
    }
    let Some(form) = state.name_form.as_mut() else { return };
    if button == 0 {
        if let Some(field) = FIELDS.into_iter()
            .filter(|field| form.group_edit.is_none() || *field == Field::Name)
            .find(|field| look::hit(mouse, form_field(&layout, *field)))
        {
            form.focus = field;
            return;
        }
    }
    if form.group_edit.is_some() {
        return;
    }
    if look::hit(mouse, layout.color) {
        let colors = runtime::WAYPOINT_COLORS;
        let i = colors.iter().position(|c| *c == form.color).unwrap_or(colors.len() - 1);
        form.color = colors[step(colors.len(), i)];
    } else if look::hit(mouse, layout.dimension) {
        let dimensions = [Dimension::Overworld, Dimension::Nether, Dimension::End];
        let i = dimensions.iter().position(|d| *d == form.dimension).unwrap_or(0);
        form.dimension = dimensions[step(3, i)];
    } else if look::hit(mouse, layout.set) {
        let names: Vec<String> = groups.into_iter().flatten().chain(std::iter::once(String::new())).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let i = names.iter().position(|g| *g == form.group).unwrap_or(0);
        form.group = names[step(names.len(), i)].clone();
    } else if button == 0 && look::hit(mouse, layout.world_3d) {
        form.show_3d = !form.show_3d;
    } else if button == 0 && look::hit(mouse, layout.enabled) {
        form.disabled = !form.disabled;
    }
}

fn save_name(game: &Game<'_>, state: &mut State) {
    let Some(mut form) = state.name_form.take() else {
        return;
    };
    let controller = runtime::initialize(game.native_data_dir(runtime::MOD_ID));
    let initials = Some(form.initials.trim().to_uppercase()).filter(|text| !text.is_empty());
    let yaw = form.yaw();
    if let Some(edit) = form.group_edit.take() {
        let name = form.text.trim();
        let saved = match edit {
            GroupEdit::Create => controller.add_waypoint_groups(&state.world,[name.to_owned()]),
            GroupEdit::Rename(old) => controller.rename_waypoint_group(&state.world,&old,name),
        };
        if saved {
            controller.select_waypoint_group(&state.world,Some(name));
            state.waypoint_group = Some(name.to_owned());
            state.waypoint_scroll = 0;
            state.waypoint_selected = None;
        }
        state.notice = Some((ui(if saved { "set_saved" } else { "set_unchanged" }), Instant::now()));
    } else if let Some(rename) = form.rename {
        let at = form.position();
        controller.change_waypoint(&state.world, rename.key, |point| {
            point.name = Waypoint::clean_name(&form.text);
            point.options.group = form.group.trim().to_owned();
            point.color = if point.initials.is_some() || point.icon == runtime::WaypointIcon::Skull {
                form.color & 0x00ff_ffff
            } else {
                Waypoint::clean_color(form.color)
            };
            if let Some([x, y, z]) = at {
                point.x = x;
                point.y = y;
                point.z = z;
            }
            point.dimension = form.dimension;
            point.initials = initials.clone();
            point.show_3d = form.show_3d;
            point.options.disabled = form.disabled;
            point.options.rotation = yaw.is_some();
            point.options.yaw = yaw.unwrap_or(0);
        });
        state.waypoint_selected = None;
        state.notice = Some((ui("waypoint_updated"), Instant::now()));
    } else if let Some([x, y, z]) = form.position() {
        let biome = known_biome(game, form.dimension, [x, y, z]);
        let saved = controller.add_waypoint(Waypoint {
            world: state.world.clone(),
            dimension: form.dimension,
            x,
            y,
            z,
            name: Waypoint::clean_name(&form.text),
            initials,
            options: runtime::WaypointOptions {
                group: form.group.trim().to_owned(),
                temporary: !matches!(form.kind, MarkerMode::Saved),
                destination: matches!(form.kind, MarkerMode::Destination),
                visibility: if !matches!(form.kind,MarkerMode::Saved) && game.map_option("temporary_waypoints_global")==Some(1) {1} else {0},
                disabled: form.disabled,
                rotation: yaw.is_some(),
                yaw: yaw.unwrap_or(0),
                ..Default::default()
            },
            color: form.color,
            icon: runtime::WaypointIcon::Letter,
            biome,
            show_3d: form.show_3d,
        });
        state.notice = Some((ui(if saved { "waypoint_saved" } else { "waypoint_unchanged" }), Instant::now()));
    }
    state.mode = form.background;
    runtime::set_text_input(false);
}

fn cancel_name(state: &mut State) {
    if let Some(form) = state.name_form.take() {
        state.mode = form.background;
    }
    runtime::set_text_input(false);
}

pub(super) fn drag(game: &Game<'_>, delta: [f32; 2], button: i32, canceled: &mut bool) {
    if game.mod_screen() != Some(runtime::MOD_ID) || !game.map_on(runtime::MOD_ID) || button != 0 {
        return;
    }
    let mut state = state();
    if state.info.is_some() {
        *canceled = true;
        return;
    }
    if state.mode != Mode::Map || !state.dragging {
        return;
    }
    state.camera.pan([f64::from(delta[0]), f64::from(delta[1])]);
    state.elapsed = 1.0;
    *canceled = true;
}

pub(super) fn scroll(
    game: &Game<'_>,
    screen: &mod_api::ScreenView<'_>,
    scroll: [f64; 2],
    canceled: &mut bool,
) {
    if game.mod_screen() != Some(runtime::MOD_ID) || !game.map_on(runtime::MOD_ID) {
        return;
    }
    let mut state = state();
    if state.mode == Mode::Settings {
        let max=settings_pages(settings_items(game,&state).len()) - 1;
        state.settings_scroll=(state.settings_scroll as i64-scroll[1].signum() as i64).clamp(0,max as i64) as usize;
        *canceled=true;
        return;
    }
    if state.mode == Mode::Caches {
        let snapshot = runtime::initialize(game.native_data_dir(runtime::MOD_ID))
            .caches
            .snapshot();
        let max = snapshot
            .entries
            .len()
            .saturating_sub(cache_rows(screen.rect[3]));
        state.caches.scroll =
            (state.caches.scroll.min(max) as i64 - scroll[1] as i64).clamp(0, max as i64) as usize;
        *canceled = true;
        return;
    }
    if state.info.is_some() {
        let rect = information_rect(screen.rect[2], screen.rect[3]);
        let visible = ((rect[3] - 56.0) / 12.0).max(1.0) as usize;
        let lines = information_lines(game, &state, state.info.unwrap(), rect[2] - 24.0);
        let max = lines.len().saturating_sub(visible);
        state.info_scroll =
            (state.info_scroll.min(max) as i32 - scroll[1] as i32).clamp(0, max as i32) as usize;
        *canceled = true;
        return;
    }
    if state.mode == Mode::Waypoints {
        if state.waypoint_groups_open {
            let list=look::waypoint_list([screen.rect[2],screen.rect[3]],0,runtime::right_to_left());
            let fit=((list.actions[0][1]-list.set[1]-list.set[3])/list.set[3]).floor().max(1.) as usize;
            let groups=(waypoint_groups(game,&state).len()+2).saturating_sub(fit);
            state.waypoint_groups_scroll=(state.waypoint_groups_scroll as i64-scroll[1] as i64).clamp(0,groups as i64) as usize;
            *canceled=true;
            return;
        }
        let rows = listed_waypoints(game,&state).len().saturating_sub(waypoint_visible_rows(screen.rect[3]));
        let next = (state.waypoint_scroll as i32 - scroll[1] as i32).clamp(0, rows as i32);
        state.waypoint_scroll = next as usize;
        *canceled = true;
        return;
    }
    if state.mode != Mode::Map {
        return;
    }
    if state.map_panel == MapPanel::Waypoints
        && look::mini_menu([screen.rect[2], screen.rect[3]], look::mini_menu_rows(screen.rect[3]), runtime::right_to_left())
            .rows.iter().any(|row| look::hit(screen.mouse, *row)) {
        let max = mini_waypoints(&state).len().saturating_sub(look::mini_menu_rows(screen.rect[3]).saturating_sub(2));
        state.map_panel_scroll = (state.map_panel_scroll as i64 - scroll[1] as i64).clamp(0,max as i64) as usize;
        *canceled = true;
        return;
    }
    if over_map_controls(game,&state,screen.mouse, screen.rect[2], screen.rect[3]) {
        *canceled = true;
        return;
    }
    let rect = rectangle(screen.rect[2], screen.rect[3]);
    if !inside(screen.mouse, rect) {
        return;
    }
    let offset = [
        f64::from(screen.mouse[0] - rect[0] - rect[2] / 2.0),
        f64::from(screen.mouse[1] - rect[1] - rect[3] / 2.0),
    ];
    map_zoom(game,&mut state,scroll[1],offset);
    *canceled = true;
}

/// GLFW's Control modifier bit.
const CONTROL: u32 = 0x2;

pub(super) fn screen_key(game: &Game<'_>, key: &str, modifiers: u32, canceled: &mut bool) {
    if game.mod_screen() != Some(runtime::MOD_ID) || !game.map_on(runtime::MOD_ID) {
        return;
    }
    let mut state = state();
    if state.info.is_some() {
        if key == "key.keyboard.escape" {
            state.info = None;
        }
        *canceled = true;
        return;
    }
    if state.mode == Mode::Settings {
        if key == "key.keyboard.escape" { close_settings(&mut state); }
        *canceled=true;
        return;
    }
    if state.mode == Mode::Waypoints && state.waypoint_search_focus {
        match key {
            "key.keyboard.escape" | "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
                state.waypoint_search_focus=false;
                runtime::set_text_input(false);
            }
            "key.keyboard.backspace" => {
                state.waypoint_search.pop();
                state.waypoint_scroll=0;
                state.waypoint_selected=None;
            }
            _=>{}
        }
        *canceled=true;
        return;
    }
    if state.mode == Mode::Waypoints && key == "key.keyboard.v" && modifiers & CONTROL != 0 {
        paste_waypoint(game, &mut state);
        *canceled = true;
        return;
    }
    if state.mode == Mode::Waypoints && state.waypoint_groups_open && key=="key.keyboard.escape" {
        state.waypoint_groups_open=false;
        *canceled=true;
        return;
    }
    if state.mode==Mode::Waypoints && matches!(key,"key.keyboard.up"|"key.keyboard.down") {
        let step=if key=="key.keyboard.up" {-1} else {1};
        let (_,h)=game.gui_size();
        let count=waypoint_visible_rows(h);
        let points=listed_waypoints(game,&state);
        let next=points.iter().position(|(_,p)|Some(p.key())==state.waypoint_selected)
            .map_or(0,|i|(i as i64+step).clamp(0,points.len().saturating_sub(1) as i64) as usize);
        let key=points.get(next).map(|(_,p)|p.key());
        if key.is_some() {
            state.waypoint_selected=key;
            state.waypoint_scroll=state.waypoint_scroll.clamp(next.saturating_sub(count-1),next);
        }
        *canceled=true;
        return;
    }
    if state.mode==Mode::Waypoints && matches!(key,"key.keyboard.enter"|"key.keyboard.keypad.enter") {
        if let Some(dimension)=show_selected_waypoint(&mut state) {
            runtime::initialize(game.native_data_dir(runtime::MOD_ID)).choose_dimension(true,dimension);
        }
        *canceled=true;
        return;
    }
    if state.mode == Mode::Name {
        match key {
            "key.keyboard.escape" => {
                cancel_name(&mut state);
                *canceled = true;
            }
            "key.keyboard.backspace" => {
                if let Some(form) = state.name_form.as_mut() {
                    let focus = form.focus;
                    form.field(focus).pop();
                }
                *canceled = true;
            }
            "key.keyboard.tab" => {
                if let Some(form) = state.name_form.as_mut() && form.group_edit.is_none() {
                    let i = FIELDS.iter().position(|field| *field == form.focus).unwrap_or(0);
                    form.focus = FIELDS[(i + 1) % FIELDS.len()];
                }
                *canceled = true;
            }
            "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
                if state.name_form.as_ref().is_some_and(NameForm::valid) {
                    save_name(game, &mut state);
                }
                *canceled = true;
            }
            _ => {
                *canceled = true;
            }
        }
        return;
    }
    if state.mode == Mode::Map && state.mini_filter_focus {
        match key {
            "key.keyboard.escape" | "key.keyboard.enter" | "key.keyboard.keypad.enter" => {
                state.mini_filter_focus = false;
                runtime::set_text_input(false);
            }
            "key.keyboard.backspace" => {
                state.mini_filter.pop();
                state.map_panel_scroll = 0;
            }
            _ => {}
        }
        *canceled = true;
        return;
    }
    if key == "key.keyboard.escape" {
        if state.map_panel != MapPanel::None && state.mode == Mode::Map {
            state.map_panel = MapPanel::None;
            *canceled = true;
            return;
        }
        runtime::set_text_input(false);
        state.close = true;
        *canceled = true;
    }
}

pub(super) fn screen_char(game: &Game<'_>, c: char, canceled: &mut bool) {
    if game.mod_screen() != Some(runtime::MOD_ID) || !game.map_on(runtime::MOD_ID) {
        return;
    }
    let mut state = state();
    if state.mode == Mode::Map && state.mini_filter_focus {
        if !c.is_control() && state.mini_filter.len() + c.len_utf8() <= 64 {
            state.mini_filter.push(c);
            state.map_panel_scroll = 0;
        }
        *canceled = true;
        return;
    }
    if state.mode == Mode::Waypoints && state.waypoint_search_focus {
        if !c.is_control() && state.waypoint_search.len()+c.len_utf8()<=128 {
            state.waypoint_search.push(c);
            state.waypoint_scroll=0;
            state.waypoint_selected=None;
        }
        *canceled=true;
        return;
    }
    if state.mode != Mode::Name {
        return;
    }
    if c.is_control() {
        return;
    }
    if let Some(form) = state.name_form.as_mut() {
        let focus = form.focus;
        if form.accepts(focus, c) {
            form.field(focus).push(c);
        }
    }
    *canceled = true;
}

pub(super) fn disconnect(game: &Game<'_>) {
    if !game.native_window() {
        return;
    }
    runtime::set_text_input(false);
    QUICK_MINIMAP.store(0,std::sync::atomic::Ordering::Relaxed);
    LOCATOR_WAYPOINTS.lock().unwrap_or_else(|error| error.into_inner()).clear();
    WAYSTONES.lock().unwrap_or_else(|error| error.into_inner()).clear();
    let mut s = state();
    let controller = runtime::initialize(game.native_data_dir(runtime::MOD_ID));
    controller.clear_temporary_waypoints(&s.world);
    let next = s.session + 1;
    *s = State {
        session: next,
        ..State::default()
    };
    controller.maps.reset(next);
}

pub(super) fn atlas_invalidated() {
    state().frame = None;
    state().page_images.clear();
}
