// SPDX-License-Identifier: GPL-3.0-only
//! The light overlay in the world: while a torch is in either hand, a small cross lies on every
//! floor around you that the map's light rule (`light.rs`) marks, in the overlay's colours: an
//! opaque full block with two blocks of air above, whose spot has the light the rule asks for
//! (by default block light 0 to 7: red where monsters spawn, yellow where they nearly could).
//!
//! Earned with the Skills node that gives the tag `map.marks` (Skills on the server and in this
//! client), and switched off with the setting `light_marks`. The spots are found again only when
//! you move a block, the rule's settings change, or a block or light packet arrives; each frame
//! only draws them.
#![allow(unexpected_cfgs)]
use crate::light::{self, Rule};
use mod_api::{
    BlockMetadata, ClientHost, ClientPacket, EquipmentSlot, Host, HostBlockStates, LevelRef, LightLayer, RenderLevelStage,
    RenderLevelStageEvent, SettingSpec, SettingValue, WorldDepth, WorldLine,
};
#[allow(unused_imports)]
use mod_api::prelude::*;
use std::sync::Mutex;

/// How far around the player floors are marked: 16 blocks across, 8 up and down.
pub const REACH: i32 = 16;
pub const HEIGHT: i32 = 8;
const WIDTH: f32 = 2.0;
/// Half the cross's arm, in blocks.
const ARM: f64 = 0.18;
/// Ticks after a world change before the floors are looked at again, so the client's own light
/// update has run; once more a little later for light that spread slowly.
const SETTLE: [u32; 2] = [2, 12];

/// A block as the marks see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// Air (and cave air): room to stand.
    Air,
    /// An opaque full block a monster may stand on.
    Floor,
    Other,
}

/// The floors of a column (bottom up) with two airs above them, by index.
pub fn floors(column: &[Cell]) -> impl Iterator<Item = usize> + '_ {
    column.windows(3).enumerate().filter(|(_, w)| w == &[Cell::Floor, Cell::Air, Cell::Air]).map(|(i, _)| i)
}

/// Blocks with a full opaque top where monsters still never spawn.
pub fn never_spawn_on(block: &str) -> bool {
    matches!(block, "minecraft:bedrock" | "minecraft:barrier" | "minecraft:magma_block" | "minecraft:packed_ice" | "minecraft:blue_ice")
}

struct Marks {
    /// The marked floors and their colours (ARGB).
    spots: Vec<([i32; 3], u32)>,
    /// What they were found for: the player's block and the rule.
    found_for: Option<([i32; 3], Rule)>,
    /// Ticks since the last world change, while a recount waits.
    changed: Option<u32>,
    shown: bool,
}

static MARKS: Mutex<Marks> = Mutex::new(Marks { spots: Vec::new(), found_for: None, changed: None, shown: false });

fn with<R>(f: impl FnOnce(&mut Marks) -> R) -> R {
    f(&mut MARKS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Whether this player earned the marks in Skills.
fn earned() -> bool {
    #[cfg(feature = "skills")]
    {
        return skills::has_tag(light::MARKS_TAG);
    }
    #[allow(unreachable_code)]
    false
}

fn switched_on(host: &dyn Host) -> bool {
    let spec: SettingSpec = light::specs().into_iter().find(|s| s.key == light::MARKS).expect("the marks switch is a rule setting");
    !matches!(spec.read(host.setting(crate::runtime::MOD_ID, light::MARKS).as_deref()), SettingValue::Toggle(false))
}

/// A packet that changes blocks or light around the player: the floors are looked at again soon.
pub fn packet(packet: &ClientPacket) {
    let id = packet.id.to_string();
    if matches!(
        id.as_str(),
        "minecraft:block_update" | "minecraft:section_blocks_update" | "minecraft:light_update" | "minecraft:level_chunk_with_light" | "minecraft:forget_level_chunk"
    ) {
        with(|m| m.changed = Some(0));
    }
}

/// The world left: nothing to show.
pub fn reset() {
    with(|m| {
        m.spots.clear();
        m.found_for = None;
        m.changed = None;
    });
}

fn holds_torch(host: &dyn Host, entity: mod_api::EntityRef) -> bool {
    let Some(living) = host.living(entity) else { return false };
    [EquipmentSlot::MainHand, EquipmentSlot::OffHand]
        .into_iter()
        .filter_map(|slot| living.equipment(host, slot))
        .any(|s| s.count() > 0 && s.item().id(host) == "minecraft:torch")
}

/// Each client tick: whether the marks show, and the floors found again when needed.
pub fn tick(host: &dyn ClientHost) {
    let read: &dyn Host = host;
    let Some(p) = host.local_player() else { return };
    if !(earned() && switched_on(read) && holds_torch(read, p.entity())) {
        with(|m| {
            m.shown = false;
            m.spots.clear();
            m.found_for = None;
        });
        return;
    }
    let (Some(level), Some(at)) = (p.level(read), p.entity().block_position(read)) else { return };
    let rule = Rule::read(read);
    let recount = with(|m| {
        let settled = match m.changed {
            Some(n) => {
                m.changed = (n < SETTLE[1]).then_some(n + 1);
                SETTLE.contains(&n)
            }
            None => false,
        };
        let moved = m.found_for != Some((at, rule));
        let started = !m.shown;
        m.shown = true;
        settled || moved || started
    });
    if recount {
        let spots = find(read, level, at, rule);
        with(|m| {
            m.spots = spots;
            m.found_for = Some((at, rule));
        });
    }
}

/// The floors around `at` the rule marks, with their colours.
fn find(host: &dyn Host, level: LevelRef, at: [i32; 3], rule: Rule) -> Vec<([i32; 3], u32)> {
    let states = host.block_states();
    let (lo, hi) = (at[1] - HEIGHT, at[1] + HEIGHT);
    let mut spots = Vec::new();
    let mut column = Vec::with_capacity((hi - lo + 3) as usize);
    for x in at[0] - REACH..=at[0] + REACH {
        for z in at[2] - REACH..=at[2] + REACH {
            column.clear();
            // Two more on top so the highest floor sees its two airs.
            for y in lo..=hi + 2 {
                let cell = match level.block_state(host, [x, y, z]) {
                    None => Cell::Other,
                    Some(state) => match states.name(state.block()) {
                        Ok("minecraft:air" | "minecraft:cave_air") => Cell::Air,
                        Ok(name) if !never_spawn_on(name) && states.is_solid_render(state) == Some(true) => Cell::Floor,
                        _ => Cell::Other,
                    },
                };
                column.push(cell);
            }
            for i in floors(&column) {
                let floor = [x, lo + i as i32, z];
                let above = [x, floor[1] + 1, z];
                let light = |layer| level.brightness(host, layer, above).map(|v| v.clamp(0, 15) as u8);
                if floor[1] <= hi
                    && let (Some(block), Some(sky)) = (light(LightLayer::Block), light(LightLayer::Sky))
                    && let Some(colour) = rule.colour(block, sky)
                {
                    spots.push((floor, colour));
                }
            }
        }
    }
    spots
}

/// A small cross lying on the top of the block at `p`, in the rule's colour, a little see-through.
fn cross([x, y, z]: [i32; 3], colour: u32) -> [WorldLine; 2] {
    let colour = 0xD000_0000 | (colour & 0x00FF_FFFF);
    let (cx, cy, cz) = (f64::from(x) + 0.5, f64::from(y) + 1.01, f64::from(z) + 0.5);
    [
        WorldLine::new([cx - ARM, cy, cz - ARM], [cx + ARM, cy, cz + ARM], colour, WIDTH, WorldDepth::Tested),
        WorldLine::new([cx - ARM, cy, cz + ARM], [cx + ARM, cy, cz - ARM], colour, WIDTH, WorldDepth::Tested),
    ]
}

pub fn draw(event: &mut RenderLevelStageEvent<'_>) {
    if !matches!(event.stage, RenderLevelStage::AfterTranslucentBlocks) {
        return;
    }
    let lines: Vec<WorldLine> = with(|m| m.spots.iter().flat_map(|(p, c)| cross(*p, *c)).collect());
    if !lines.is_empty()
        && let Err(e) = event.draw.lines(&lines)
    {
        eprintln!("[map] drawing the light marks: {e:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_floor_needs_two_airs_above() {
        use Cell::*;
        let col = [Floor, Air, Air, Floor, Air, Other, Floor, Air, Air, Air];
        assert_eq!(floors(&col).collect::<Vec<_>>(), vec![0, 6]);
        assert_eq!(floors(&[Floor, Air]).count(), 0, "the top of the scan");
        assert!(never_spawn_on("minecraft:bedrock") && !never_spawn_on("minecraft:stone"));
    }
}
