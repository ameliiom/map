// SPDX-License-Identifier: GPL-3.0-only
fn map_hud_background(game: &crate::game::Game<'_>, layer: &str, draw: &mut horizon_client::hud::HudBatch) {
    if layer == "map:background" {
        crate::game::reworked_maps::hud_under(game, draw);
    }
}
