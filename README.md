# Map

A minimap and a world map for [Horizon](https://playhorizon.dev) (Minecraft: Java Edition 26.3 in Rust), in Minecraft's own look: a bevelled minimap that turns with you, every creature as its own face, waypoints and death points, a full-screen world map of everywhere you have been. English and Arabic; Arabic mirrors every screen.

## What it shows
- **Minimap** (hold `H` to enlarge it): square or circle, rotating or north-locked, zoom 1–6, compass letters, the faces of nearby players, mobs and items (darker below you), waypoints (clamped to the edge when far), coordinates and optional biome, dimension, time and light.
- **World map** (`M`): explored ground on black, cursor coordinates and biome, zoom, waypoints as pennants, a right-click menu (create waypoint, teleport, share, export PNG) and a waypoint mini-menu.
- **Waypoints** (`N` for the list, `R` to create one here): list, create and edit screens, sets, sharing, death points.
- **With [Team](https://github.com/ameliiom/team)** (0.3 or later, optional): each teammate's face on both maps (on the edge of the minimap when far) and every ping as a pennant in its owner's colour, with a sword for danger and the block for loot.
- Cave view, chunk grid, slime chunks, light overlay, footsteps, PNG export, and import of Xaero's waypoints and settings.

## Install
1. Copy `map.hmod` into `%APPDATA%\horizon\horizon-native-mods\`.
2. Add the line `mod.map:true` to `%APPDATA%\horizon\horizon-mods.txt`.
3. Build: in the launcher's Mods page press **Build**, or run `horizon-launcher native-build --instance horizon --trust-native-code`. Needs launcher 0.2.82 or later. A native mod runs with your rights: install it only from a source you trust.

Built for Horizon Nightly `f037744` (2026-11-21).

## Build
```powershell
cargo horizon-mod build
```
The source package is `target\horizon-mod\map.hmod`; the launcher compiles it (step 3 above).

## Licence
GPL-3.0-only, see [LICENSE](LICENSE) and [NOTICE](NOTICE). Adapted from [trail](https://github.com/groznen95/trailgrid) by groznen95, itself derived from Minimap, Worldmap and waypoints reworked by daniliammo and Gurov08ruS. Its look follows Xaero's Minimap and World Map, studied from the running mods; no code or assets from them are included. Not affiliated with Xaero or the Horizon project.
