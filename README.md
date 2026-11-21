# Trailgrid Maps

Trailgrid Maps is a client-side Horizon mod with a configurable minimap, a world map that remembers explored terrain, a searchable waypoint list, and entity radar. Map data stays in your local Horizon profile, with separate caches for each world and server.

| Default key | Action |
| --- | --- |
| `M` | Open the world map. Drag to pan and scroll to zoom. |
| `N` | Open the waypoint list. Filter by group or dimension, search by name, and sort the results. Press Enter to show the selected waypoint on the map. |
| `B` | Add a waypoint at your current position. |
| `Y` | Open map settings. The same settings are available from the map and waypoint list. |
| `Z` | Enlarge the minimap. |

The interface follows your game language and defaults to English. To use the release in English, choose **English (US)** in Minecraft's language settings. Saved waypoint names stay as entered.

## Install

Version 0.1.0 targets **Horizon Nightly 2026-11-20 (`0a875da`)** and **Minecraft Java Edition 26.3**. It is a native source `.hmod`; Horizon Launcher builds it with the signed SDK for that exact Nightly. To install the release manually, put the package at `horizon-native-mods/trailgrid_maps.hmod` inside the instance directory. Enable Trailgrid Maps and build native mods for that instance.

Radar shows entities the client receives. Map teleport requires server permissions. On Horizon SMP the claim overlay can query `/claims` and `/claim info`; Waystones and Open Parties and Claims overlays depend on compatible data from the server. The first release was tested in a local world on Apple Silicon macOS. Live Horizon SMP and optional server integrations were outside that release test.

## Copy maps and waypoints from a previous build

Close the game and use a profile with only one mapping mod enabled. Trailgrid Maps has its own mod ID and data directory. Before opening a world with it, copy the contents of `horizon-mods/.horizon-mod-data/minimap_worldmap_waypoints_reworked/` to `horizon-mods/.horizon-mod-data/trailgrid_maps/` in the target profile. Keep the original directory as a backup. For an existing Trailgrid profile, back up both directories and copy selected files explicitly to avoid replacing newer data. Use the same save folder name or server address so the copied map and waypoints resolve to the same world. If Horizon reports the old mod ID as missing in a single-player save, choose **Back Up and Open** after checking that the list only names the old map mod. Rebind custom keys in the new profile as needed.

The world map's import tools can also read supported waypoint, settings and cache formats from existing Xaero's Minimap and Xaero's World Map data. The Java mods themselves are separate products.

## Source and license

Trailgrid Maps is licensed under [GPL-3.0-only](LICENSE). It is based on [Minimap, Worldmap and waypoints reworked](https://github.com/Gurov08ruS/Minimap-Worldmap-and-waypoints-reworked), created by daniliammo and continued by Gurov08ruS. See [NOTICE](NOTICE) and [LICENSE-MIT](LICENSE-MIT) for attribution and the original template's license. The icon is original to Trailgrid Maps. The release package contains the Rust source and assets used by the launcher.

Compatibility with existing data refers to [Xaero's Minimap](https://www.curseforge.com/minecraft/mc-mods/xaeros-minimap) and [Xaero's World Map](https://www.curseforge.com/minecraft/mc-mods/xaeros-world-map) by Xaero. Trailgrid Maps is an independent project.
