// SPDX-License-Identifier: GPL-3.0-only
// The waypoint and settings screens show the world blurred behind them, as vanilla screens do.
fn map_screen_blur(game: &crate::game::Game<'_>, radius: &mut Option<f32>) {
    if radius.is_none() && crate::game::reworked_maps::blurs_world(game) {
        *radius = Some(game.settings.menu_blur as f32);
    }
}
