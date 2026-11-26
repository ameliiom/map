// SPDX-License-Identifier: GPL-3.0-only
//! One light rule for the map's light overlay and for its in-world marks: which light level is
//! marked (block light or sky light, between `light_min` and `light_max`) and in which colour.
//! Both views ask [`Rule::colour`]; the settings that shape the rule are registered here only.
use mod_api::{Host, SettingSpec, SettingValue};

pub const SOURCE: &str = "light_source";
pub const MIN: &str = "light_min";
pub const MAX: &str = "light_max";
pub const COLOUR: &str = "light_overlay_color";
/// The in-world marks' own switch (they also need the Skills node that gives [`MARKS_TAG`]).
pub const MARKS: crate::kit::setting::Toggle =
    crate::kit::setting::Toggle { key: "light_marks", default: true, label: Some("mwwr.setting.light_marks") };
/// The Skills tag that earns the in-world marks (node `skills:mining` / `spawn_sight`).
pub const MARKS_TAG: &str = "map.marks";

/// Light 0 and the lit levels inside the range, as the overlay has always drawn them.
const DARK: u32 = 0xfd645f;
const DIM: u32 = 0xf7c15d;

/// Every setting the rule reads, with its default.
pub fn specs() -> Vec<SettingSpec> {
    vec![
        SettingSpec::choice(SOURCE, "mwwr.setting.light_source", ["mwwr.value.light_source.0", "mwwr.value.light_source.1"], 0),
        SettingSpec::color(COLOUR, "mwwr.setting.light_overlay_color", DIM),
        SettingSpec::range(MIN, "mwwr.setting.light_min", 0., 15., 1., 0.),
        SettingSpec::range(MAX, "mwwr.setting.light_max", 0., 15., 1., 7.),
        MARKS.spec(crate::runtime::MOD_ID),
    ]
}

/// The rule as the settings shape it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Sky light instead of block light.
    pub sky: bool,
    pub min: u8,
    pub max: u8,
    /// A colour chosen for the lit levels (`0xRRGGBB`); None keeps the overlay's own.
    pub colour: Option<u32>,
}

impl Default for Rule {
    fn default() -> Self {
        Rule { sky: false, min: 0, max: 7, colour: None }
    }
}

impl Rule {
    /// The rule from this client's settings, through any host.
    pub fn read(host: &dyn Host) -> Self {
        let value = |key: &str| {
            let spec = specs().into_iter().find(|s| s.key == key)?;
            Some(spec.read(host.setting(crate::runtime::MOD_ID, key).as_deref()))
        };
        let level = |key: &str, fallback: u8| match value(key) {
            Some(SettingValue::Range(v)) => v.clamp(0.0, 15.0).round() as u8,
            _ => fallback,
        };
        Rule {
            sky: matches!(value(SOURCE), Some(SettingValue::Choice(1))),
            min: level(MIN, 0),
            max: level(MAX, 7),
            colour: match value(COLOUR) {
                Some(SettingValue::Color(rgb)) if rgb != DIM => Some(rgb),
                _ => None,
            },
        }
    }

    /// The colour (ARGB) a spot with this block and sky light is marked in, None when unmarked.
    pub fn colour(&self, block: u8, sky: u8) -> Option<u32> {
        let light = if self.sky { sky } else { block };
        if !(self.min..=self.max).contains(&light) {
            return None;
        }
        let (alpha, own) = if light == 0 { (0xef00_0000, DARK) } else { (0xdf00_0000, DIM) };
        Some(alpha | self.colour.unwrap_or(own))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_marks_only_its_range_in_its_colours() {
        let r = Rule::default();
        assert_eq!(r.colour(0, 15), Some(0xeffd645f), "dark: red");
        assert_eq!(r.colour(7, 15), Some(0xdff7c15d), "dim: yellow");
        assert_eq!(r.colour(8, 0), None, "lit enough");
        let sky = Rule { sky: true, min: 0, max: 3, colour: Some(0x0000ff) };
        assert_eq!(sky.colour(15, 2), Some(0xdf0000ff));
        assert_eq!(sky.colour(0, 4), None);
    }

    #[test]
    fn every_rule_setting_is_registered_once() {
        let keys: Vec<String> = specs().iter().map(|s| s.key.clone()).collect();
        assert_eq!(keys, [SOURCE, COLOUR, MIN, MAX, MARKS.key]);
    }
}
