// SPDX-License-Identifier: GPL-3.0-only
use mod_api::{ClientHost, Registrar, SettingSpec, Text};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::Path,
};

const GROUPS: [&str; 4] = ["radar_players", "radar_mobs", "radar_items", "radar_other"];
const MAX_MARKERS: usize = 96;

pub struct Dot {
    pub entity_id: u32,
    pub kind: String,
    pub at: [f64; 3],
    pub color: u32,
    pub label: Option<String>,
    pub icon: bool,
    pub size: f32,
    pub icon_scale: f32,
    pub alpha: u8,
    pub order: u16,
    pub over_frame: bool,
    pub fallback_name: Option<String>,
    /// Blocks above (positive) or below the player.
    pub dy: f32,
    /// Whether the face darkens with depth below the player.
    pub depth_shade: bool,
}

pub struct View {
    pub dimension: String,
    pub dots: Vec<Dot>,
    pub world_map: bool,
    pub icon_size: f32,
    pub name_scale: f32,
    pub smooth: bool,
    pub legacy: bool,
}

static VIEW: Mutex<Option<Arc<View>>> = Mutex::new(None);
static COUNTDOWN: AtomicU8 = AtomicU8::new(0);
static RULES: Mutex<Option<Arc<Rules>>> = Mutex::new(None);
static IMPORTED: Mutex<Option<Arc<ImportedCategory>>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rule {
    pub visible: Option<bool>,
    pub rgb: Option<u32>,
    pub icon: Option<u8>,
    pub label: Option<u8>,
    pub size: Option<f32>,
    pub height: Option<f64>,
    pub priority: bool,
    pub team_color: Option<bool>,
    pub icon_scale: Option<f32>,
    pub height_fade: Option<bool>,
    pub height_label: Option<u8>,
    pub fade_from: Option<f64>,
    pub limit: Option<u16>,
    pub nametag: Option<bool>,
    pub icon_fallback: Option<bool>,
    pub over_frame: Option<u8>,
    pub order: Option<u16>,
}

impl Rule {
    fn over(self, base: Self) -> Self {
        Self {
            visible: self.visible.or(base.visible),
            rgb: self.rgb.or(base.rgb),
            icon: self.icon.or(base.icon),
            label: self.label.or(base.label),
            size: self.size.or(base.size),
            height: self.height.or(base.height),
            priority: self.priority || base.priority,
            team_color: self.team_color.or(base.team_color),
            icon_scale: self.icon_scale.or(base.icon_scale),
            height_fade: self.height_fade.or(base.height_fade),
            height_label: self.height_label.or(base.height_label),
            fade_from: self.fade_from.or(base.fade_from),
            limit: self.limit.or(base.limit),
            nametag: self.nametag.or(base.nametag),
            icon_fallback: self.icon_fallback.or(base.icon_fallback),
            over_frame: self.over_frame.or(base.over_frame),
            order: self.order.or(base.order),
        }
    }
}

#[derive(Default)]
pub struct Facts<'a> {
    pub item: Option<&'a str>,
    pub custom_name: Option<std::borrow::Cow<'a, str>>,
    pub hostile: bool,
    pub baby: bool,
    pub tamed: bool,
    pub same_team: bool,
    pub team: Option<&'a str>,
    pub team_color: Option<u32>,
    pub light: Option<(u8, u8)>,
    pub cave: bool,
    pub tracked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Property {
    Hostile,
    Baby,
    Tamed,
    SameTeam,
    InTeam,
    Above,
    SameGround,
    Lit,
    Named,
    Tracked,
}

pub enum Selector {
    All,
    Never,
    Group(usize),
    Living(bool),
    Namespace(bool),
    Kind(String),
    Player(String),
    Item(String),
    CustomName(String),
    State(Property, bool),
}

impl Selector {
    pub fn hard_rule(value: &str) -> Option<Self> {
        use Property::*;
        Some(match value {
            "anything" => Self::All,
            "nothing" => Self::Never,
            "players" => Self::Group(0),
            "living" => Self::Living(true),
            "items" => Self::Group(2),
            "vanilla" => Self::Namespace(true),
            "modded" => Self::Namespace(false),
            "hostile" => Self::State(Hostile, true),
            "friendly" => Self::State(Hostile, false),
            "baby" => Self::State(Baby, true),
            "tamed" => Self::State(Tamed, true),
            "same-team" => Self::State(SameTeam, true),
            "other-teams" => Self::State(SameTeam, false),
            "in-a-team" => Self::State(InTeam, true),
            "teamless" => Self::State(InTeam, false),
            "above-ground" => Self::State(Above, true),
            "below-ground" => Self::State(Above, false),
            "my-ground" => Self::State(SameGround, true),
            "not-my-ground" => Self::State(SameGround, false),
            "block-lit" => Self::State(Lit, true),
            "block-unlit" => Self::State(Lit, false),
            "has-custom-name" => Self::State(Named, true),
            "no-custom-name" => Self::State(Named, false),
            "tracked" => Self::State(Tracked, true),
            _ => return None,
        })
    }

    pub fn from_xaero(value: &str) -> Option<Self> {
        let (prefix, value) = value.split_once(';').unwrap_or(("entity", value));
        match prefix {
            "entity" => resource_id(value).map(Self::Kind),
            "player" => Some(Self::Player(value.to_owned())),
            "item" => resource_id(value).map(Self::Item),
            "custom-name" => Some(Self::CustomName(value.to_owned())),
            _ => match value {
                "true" | "1" => Self::hard_rule(prefix),
                "false" | "0" => match Self::hard_rule(prefix)? {
                    Self::State(property, expect) => Some(Self::State(property, !expect)),
                    Self::Namespace(expect) => Some(Self::Namespace(!expect)),
                    Self::Living(expect) => Some(Self::Living(!expect)),
                    _ => None,
                },
                _ => None,
            },
        }
    }

    fn matches(&self, kind: &str, group: usize, player: Option<&str>, facts: &Facts<'_>) -> bool {
        use Property::*;
        match self {
            Self::All => true,
            Self::Never => false,
            Self::Group(g) => group == *g,
            Self::Living(wanted) => (group <= 1 || kind == "minecraft:armor_stand") == *wanted,
            Self::Namespace(vanilla) => kind.starts_with("minecraft:") == *vanilla,
            Self::Kind(id) => id == kind,
            Self::Player(name) => player.is_some_and(|p| p.eq_ignore_ascii_case(name)),
            Self::Item(item) => facts.item == Some(item),
            Self::CustomName(name) => facts.custom_name.as_deref() == Some(name),
            Self::State(property, expected) => {
                let actual = match property {
                    Hostile => Some(facts.hostile),
                    Baby => Some(facts.baby),
                    Tamed => Some(facts.tamed),
                    SameTeam => Some(facts.same_team),
                    InTeam => Some(facts.team.is_some()),
                    Above => facts.light.map(|(_, sky)| sky == 15),
                    SameGround => facts.light.map(|(_, sky)| (sky == 15) != facts.cave),
                    Lit => facts.light.map(|(block, _)| block > 0),
                    Named => Some(facts.custom_name.is_some()),
                    Tracked => Some(facts.tracked),
                };
                actual == Some(*expected)
            }
        }
    }
}

#[derive(Default)]
pub struct ImportedCategory {
    pub id: usize,
    pub base: Option<Selector>,
    pub includes: Vec<Selector>,
    pub excludes: Vec<Selector>,
    pub reversed: bool,
    pub inherit_includes: bool,
    pub children: Vec<ImportedCategory>,
    pub rule: Rule,
}

impl ImportedCategory {
    fn listed(&self, kind: &str, group: usize, player: Option<&str>, facts: &Facts<'_>) -> bool {
        self.includes
            .iter()
            .any(|rule| rule.matches(kind, group, player, facts))
            || self
                .children
                .iter()
                .any(|child| child.inherit_includes && child.listed(kind, group, player, facts))
    }

    fn applies(&self, kind: &str, group: usize, player: Option<&str>, facts: &Facts<'_>) -> bool {
        let matched = |rule: &Selector| rule.matches(kind, group, player, facts);
        (self.base.as_ref().is_some_and(matched) || self.listed(kind, group, player, facts))
            && self.excludes.iter().any(matched) == self.reversed
    }

    fn resolve(
        &self,
        kind: &str,
        group: usize,
        player: Option<&str>,
        facts: &Facts<'_>,
        base: Rule,
    ) -> (Rule, usize, bool) {
        let here = self.rule.over(base);
        self.children
            .iter()
            .find(|child| child.applies(kind, group, player, facts))
            .map_or_else(
                || (here, self.id, self.listed(kind, group, player, facts)),
                |child| child.resolve(kind, group, player, facts, here),
            )
    }

    fn max_height(&self) -> f64 {
        self.children
            .iter()
            .map(Self::max_height)
            .fold(self.rule.height.unwrap_or(0.), f64::max)
    }
}

pub fn uses_categories() -> bool {
    IMPORTED.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

pub fn use_imported(tree: Option<ImportedCategory>) {
    *IMPORTED.lock().unwrap_or_else(|e| e.into_inner()) = tree.map(Arc::new);
}

#[derive(Default)]
pub struct Rules {
    kinds: BTreeMap<String, Rule>,
    players: BTreeMap<String, Rule>,
}

const RULES_EXAMPLE: &str = "# category <name> <show|hide|default> <RRGGBB|default> <type>,<type>\n# track <username> [RRGGBB]\n# type <namespaced-id> <show|hide|default> <RRGGBB|default>\n";

fn color(value: &str) -> Option<Option<u32>> {
    if value == "default" {
        return Some(None);
    }
    (value.len() == 6)
        .then(|| u32::from_str_radix(value, 16).ok())
        .flatten()
        .map(Some)
}

fn visibility(value: &str) -> Option<Option<bool>> {
    match value {
        "show" => Some(Some(true)),
        "hide" => Some(Some(false)),
        "default" => Some(None),
        _ => None,
    }
}

pub fn resource_id(value: &str) -> Option<String> {
    let (namespace, path) = value.split_once(':').unwrap_or(("minecraft", value));
    fn plain(c: u8) -> bool {
        c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c)
    }
    (!namespace.is_empty()
        && !path.is_empty()
        && value.len() <= 160
        && namespace.bytes().all(plain)
        && path.bytes().all(|c| plain(c) || c == b'/'))
    .then(|| format!("{namespace}:{path}"))
}

fn parse_rule(rules: &mut Rules, line: &str) -> Option<()> {
    let fields: Vec<_> = line.split_whitespace().collect();
    match fields.as_slice() {
        ["category", _, shown, rgb, types] => {
            let rule = Rule {
                visible: visibility(shown)?,
                rgb: color(rgb)?,
                ..Rule::default()
            };
            let ids = types
                .split(',')
                .map(resource_id)
                .collect::<Option<Vec<_>>>()?;
            for id in ids {
                rules.kinds.insert(id, rule);
            }
        }
        ["type", id, shown, rgb] => {
            rules.kinds.insert(
                resource_id(id)?,
                Rule {
                    visible: visibility(shown)?,
                    rgb: color(rgb)?,
                    ..Rule::default()
                },
            );
        }
        ["track", name] | ["track", name, _]
            if !name.is_empty()
                && name.len() <= 16
                && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') =>
        {
            let rgb = match fields.get(2) {
                Some(input) => color(input)?,
                None => None,
            };
            rules.players.insert(
                name.to_ascii_lowercase(),
                Rule {
                    visible: Some(true),
                    rgb: rgb.or(Some(0x67_f4_a6)),
                    icon: Some(2),
                    label: Some(2),
                    priority: true,
                    ..Rule::default()
                },
            );
        }
        _ => return None,
    }
    Some(())
}

pub fn load_rules(directory: &Path, create_example: bool) -> io::Result<(usize, usize)> {
    let path = directory.join("radar-rules.txt");
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if create_example {
                std::fs::create_dir_all(directory)?;
                use std::io::Write;
                File::create_new(path)?.write_all(RULES_EXAMPLE.as_bytes())?;
            }
            *RULES.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return Ok((0, 0));
        }
        Err(error) => return Err(error),
    };
    const LIMIT: u64 = 64 * 1024;
    let mut source = String::new();
    file.take(LIMIT + 1).read_to_string(&mut source)?;
    if source.len() as u64 > LIMIT {
        return Err(io::Error::other("radar-rules.txt exceeds 64 KiB"));
    }
    let mut rules = Rules::default();
    let mut count = (0, 0);
    for line in source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        if parse_rule(&mut rules, line).is_some() {
            count.0 += 1;
        } else {
            count.1 += 1;
        }
    }
    *RULES.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(rules));
    Ok(count)
}

pub fn register(registrar: &mut dyn Registrar) {
    // Heads for every kind on both maps by default; names off.
    for key in ["radar", "radar_names", "radar_world_map", "radar_icons"]
        .into_iter()
        .chain(GROUPS)
    {
        registrar
            .register_setting(SettingSpec::toggle(
                key,
                format!("mwwr.setting.{key}"),
                key != "radar_names",
            ))
            .expect("valid radar setting");
    }
    for (key, default) in [("radar_style_legacy", false), ("radar_smooth", true)] {
        registrar
            .register_setting(SettingSpec::toggle(
                key,
                format!("mwwr.setting.{key}"),
                default,
            ))
            .expect("valid radar display flag");
    }
    registrar
        .register_setting(SettingSpec::toggle(
            "radar_hide_invisible",
            "mwwr.setting.radar_hide_invisible",
            true,
        ))
        .expect("valid radar visibility setting");
    registrar
        .register_setting(SettingSpec::range(
            "radar_range",
            "mwwr.setting.radar_range",
            16.,
            128.,
            8.,
            64.,
        ))
        .expect("valid radar range");
    for (key, min, max, step, default) in [
        ("radar_height", 4., 128., 4., 20.),
        ("radar_dot_size", 1., 10., 1., 2.),
        ("radar_name_scale", 1., 3., 0.5, 1.),
        ("radar_icon_size", 4., 24., 1., 8.),
    ] {
        registrar
            .register_setting(SettingSpec::range(
                key,
                format!("mwwr.setting.{key}"),
                min,
                max,
                step,
                default,
            ))
            .expect("valid radar display setting");
    }
}

pub fn clear() {
    *VIEW.lock().unwrap_or_else(|e| e.into_inner()) = None;
    COUNTDOWN.store(0, Ordering::Relaxed);
}

pub fn snapshot() -> Option<Arc<View>> {
    VIEW.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn category(id: &str) -> usize {
    match id {
        "minecraft:player" => 0,
        "minecraft:item" => 2,
        _ if id.starts_with("minecraft:") && is_utility(&id[10..]) => 3,
        _ => 1,
    }
}

fn is_utility(id: &str) -> bool {
    id.ends_with("_boat")
        || id.ends_with("_raft")
        || id.ends_with("minecart")
        || matches!(
            id,
            "armor_stand"
                | "area_effect_cloud"
                | "arrow"
                | "block_display"
                | "breeze_wind_charge"
                | "dragon_fireball"
                | "egg"
                | "end_crystal"
                | "ender_pearl"
                | "evoker_fangs"
                | "experience_bottle"
                | "experience_orb"
                | "eye_of_ender"
                | "falling_block"
                | "fireball"
                | "firework_rocket"
                | "fishing_bobber"
                | "glow_item_frame"
                | "interaction"
                | "item_display"
                | "item_frame"
                | "leash_knot"
                | "lightning_bolt"
                | "llama_spit"
                | "ominous_item_spawner"
                | "painting"
                | "potion"
                | "shulker_bullet"
                | "small_fireball"
                | "snowball"
                | "spectral_arrow"
                | "text_display"
                | "tnt"
                | "trident"
                | "wind_charge"
                | "wither_skull"
        )
}

fn default_name(host: &dyn ClientHost, id: &str) -> String {
    let key = format!("entity.{}", id.replace(':', "."));
    let name = host
        .text_string(&Text::translate(&key, vec![]))
        .unwrap_or_default();
    if name.is_empty() || name == key {
        id.rsplit(':').next().unwrap_or(id).replace('_', " ")
    } else {
        name
    }
}

pub fn sample_due(enabled: bool) -> bool {
    if !enabled {
        clear();
        return false;
    }
    COUNTDOWN.fetch_add(1, Ordering::Relaxed) % 4 == 0
}

pub fn update<'a>(
    host: &dyn ClientHost,
    setting: impl Fn(&str) -> f64,
    facts: impl Fn(u32, &str, [f64; 3]) -> Facts<'a>,
) {
    let view = gather(host, &setting, facts).map(Arc::new);
    *VIEW.lock().unwrap_or_else(|e| e.into_inner()) = view;
}

fn mode(value: Option<u8>, default: bool, listed: bool) -> bool {
    match value {
        Some(0) => false,
        Some(1) => listed,
        Some(2) => true,
        _ => default,
    }
}

fn gather<'a>(
    host: &dyn ClientHost,
    setting: &impl Fn(&str) -> f64,
    facts: impl Fn(u32, &str, [f64; 3]) -> Facts<'a>,
) -> Option<View> {
    let player = host.local_player()?.entity();
    let [x, y, z] = player.position(host)?;
    let dimension = host.dimension();
    let level = host.level(dimension)?;
    let radius = setting("radar_range").clamp(16., 128.);
    let groups = GROUPS.map(|key| setting(key) > 0.);
    let names = setting("radar_names") > 0.;
    let hide_invisible = setting("radar_hide_invisible") > 0.;
    let height = setting("radar_height").clamp(4., 128.);
    let size = setting("radar_dot_size").clamp(1., 10.) as f32;
    let icons = setting("radar_icons") > 0.;
    let rules = RULES.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let imported = IMPORTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let query_height = imported
        .as_ref()
        .map_or(height, |tree| height.max(tree.max_height()));
    let mut dots = Vec::new();
    for entity in level.entities_in(
        host,
        // One block more than the limit so an entity exactly at the limit is found (the limit
        // itself is inclusive below).
        [x - radius, y - query_height - 1., z - radius],
        [x + radius, y + query_height + 1., z + radius],
    )? {
        if entity == player || (hide_invisible && entity.is_invisible(host) == Some(true)) {
            continue;
        }
        let Some(at) = entity.position(host) else {
            continue;
        };
        let distance2 = (at[0] - x).powi(2) + (at[2] - z).powi(2);
        if distance2 > radius.powi(2) {
            continue;
        }
        let Some(kind) = entity.kind(host) else {
            continue;
        };
        let id = kind.id(host);
        let group = category(&id);
        let mut name = if group == 0
            && (imported.is_some() || rules.as_ref().is_some_and(|r| !r.players.is_empty()))
        {
            entity.name(host)
        } else {
            None
        };
        let personal = rules
            .as_ref()
            .and_then(|rules| {
                name.as_ref()
                    .and_then(|name| rules.players.get(&name.to_ascii_lowercase()))
                    .or_else(|| rules.kinds.get(&id))
            })
            .copied()
            .unwrap_or_default();
        let mut properties = imported
            .as_ref()
            .map_or_else(Facts::default, |_| facts(entity.id(), &id, at));
        properties.tracked |= personal.priority;
        if let Some(tree) = &imported
            && tree.base.is_some()
            && !tree.applies(&id, group, name.as_deref(), &properties)
            && personal.visible != Some(true)
        {
            continue;
        }
        let (category_rule, category_id, listed) = imported
            .as_ref()
            .map_or((Rule::default(), 0, false), |tree| {
                tree.resolve(&id, group, name.as_deref(), &properties, Rule::default())
            });
        let rule = personal.over(category_rule);
        let vertical = (at[1] - y).abs();
        if !rule.visible.unwrap_or(groups[group]) || vertical > rule.height.unwrap_or(height) {
            continue;
        }
        let defaults = [0x00D4_F5FF, 0x00F3_CD66, 0x00F8_6868, 0x00CD_9FF5];
        let team_rgb = if rule.team_color == Some(true) {
            properties.team_color
        } else {
            None
        };
        let color = team_rgb.or(rule.rgb).unwrap_or(defaults[group]);
        let icon = mode(rule.icon, icons, listed);
        let show_name = rule.nametag == Some(true) || mode(rule.label, names, listed);
        let name_if_needed = (show_name || (icon && rule.icon_fallback == Some(true))).then(|| {
            name.take()
                .or_else(|| entity.name(host))
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| default_name(host, &id))
                .chars()
                .take(40)
                .collect::<String>()
        });
        let relative = at[1].floor() as i32 - y.floor() as i32;
        let elevation = match rule.height_label.unwrap_or(0) {
            1 => Some(format!("{}", at[1].floor() as i32)),
            2 => Some(format!("{relative:+}")),
            3 if relative != 0 => Some(if relative > 0 { "↑" } else { "↓" }.to_owned()),
            _ => None,
        };
        let label = match (&name_if_needed, show_name, elevation) {
            (Some(name), true, Some(y)) => Some(format!("{name} {y}")),
            (Some(name), true, None) => Some(name.clone()),
            (_, _, y) => y,
        };
        let dist = distance2.sqrt();
        let fade = rule
            .fade_from
            .filter(|start| *start > 0. && *start < radius)
            .map_or(1., |start| {
                (1. - (dist - start).max(0.) / (radius - start)).clamp(0.15, 1.)
            });
        let dot = Dot {
            entity_id: entity.id(),
            kind: id,
            at,
            color,
            label,
            icon,
            size: rule.size.unwrap_or(size),
            icon_scale: rule.icon_scale.unwrap_or(1.),
            alpha: (255. * fade).round() as u8,
            order: rule.order.unwrap_or(0),
            over_frame: mode(rule.over_frame, false, listed),
            fallback_name: name_if_needed.filter(|_| !show_name),
            dy: (at[1] - y) as f32,
            depth_shade: rule.height_fade.unwrap_or(true),
        };
        dots.push((
            !rule.priority,
            distance2,
            category_id,
            rule.limit.unwrap_or(0),
            dot,
        ));
    }
    dots.sort_unstable_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.total_cmp(&b.1))
            .then_with(|| a.4.entity_id.cmp(&b.4.entity_id))
    });
    let mut counts = BTreeMap::<usize, u16>::new();
    let mut dots = dots
        .into_iter()
        .filter_map(|(_, _, category, limit, dot)| {
            if limit != 0 {
                let used = counts.entry(category).or_default();
                if *used >= limit {
                    return None;
                }
                *used += 1;
            }
            Some(dot)
        })
        .take(if imported.is_some() {
            4096
        } else {
            MAX_MARKERS
        })
        .collect::<Vec<_>>();
    dots.sort_by_key(|dot| std::cmp::Reverse(dot.order));
    Some(View {
        dimension: dimension.into(),
        dots,
        world_map: setting("radar_world_map") > 0.,
        icon_size: setting("radar_icon_size").clamp(4., 24.) as f32,
        name_scale: setting("radar_name_scale").clamp(1., 3.) as f32,
        smooth: setting("radar_smooth") > 0.,
        legacy: setting("radar_style_legacy") > 0.,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differentiates_players_mobs_items_and_projectiles() {
        assert_eq!(category("minecraft:player"), 0);
        assert_eq!(category("minecraft:camel_husk"), 1);
        assert_eq!(category("minecraft:item"), 2);
        assert_eq!(category("minecraft:spectral_arrow"), 3);
        assert_eq!(category("minecraft:cherry_chest_boat"), 3);
    }

    #[test]
    fn radar_rules_validate_a_category_before_applying_it() {
        let mut rules = Rules::default();
        assert!(parse_rule(&mut rules, "category animals hide ffffff cow,minecraft:pig").is_some());
        assert_eq!(rules.kinds["minecraft:pig"].visible, Some(false));
        assert!(parse_rule(&mut rules, "category broken show 00cf00 cow,INVALID").is_none());
        assert_eq!(rules.kinds["minecraft:cow"].visible, Some(false));
        assert!(parse_rule(&mut rules, "type cow show default").is_some());
        assert_eq!(rules.kinds["minecraft:cow"].visible, Some(true));
        assert!(parse_rule(&mut rules, "track MyFriend").is_some());
        assert!(rules.players["myfriend"].priority);
    }

    #[test]
    fn imported_category_inherits_color_and_excludes_named_players() {
        let tree = ImportedCategory {
            rule: Rule {
                rgb: Some(0xaabbcc),
                ..Rule::default()
            },
            children: vec![ImportedCategory {
                includes: vec![Selector::Group(0)],
                excludes: vec![Selector::Player("Friend".to_owned())],
                rule: Rule {
                    visible: Some(false),
                    ..Rule::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        let other = tree
            .resolve(
                "minecraft:player",
                0,
                Some("Stranger"),
                &Facts::default(),
                Rule::default(),
            )
            .0;
        assert_eq!((other.visible, other.rgb), (Some(false), Some(0xaabbcc)));
        assert_eq!(
            tree.resolve(
                "minecraft:player",
                0,
                Some("friend"),
                &Facts::default(),
                Rule::default()
            )
            .0
            .visible,
            None
        );
    }

    #[test]
    fn inherited_list_and_compound_excludes_use_received_entity_facts() {
        let tree = ImportedCategory {
            id: 1,
            children: vec![ImportedCategory {
                id: 2,
                base: Selector::hard_rule("hostile"),
                children: vec![ImportedCategory {
                    id: 3,
                    inherit_includes: true,
                    includes: vec![Selector::from_xaero("item;minecraft:diamond").unwrap()],
                    excludes: vec![Selector::from_xaero("block-lit;true").unwrap()],
                    rule: Rule {
                        icon: Some(1),
                        ..Rule::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let facts = Facts {
            item: Some("minecraft:diamond"),
            light: Some((0, 15)),
            ..Facts::default()
        };
        let (rule, id, listed) = tree.resolve("minecraft:item", 2, None, &facts, Rule::default());
        assert_eq!(id, 3);
        assert!(mode(rule.icon, false, listed));
        let lit = Facts {
            light: Some((10, 15)),
            ..facts
        };
        assert_eq!(
            tree.resolve("minecraft:item", 2, None, &lit, Rule::default())
                .1,
            2
        );
    }
}
