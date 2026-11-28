# Map

A minimap and a world map for [Horizon](https://playhorizon.dev) (Minecraft: Java Edition 26.3 in Rust), in Minecraft's own look: a bevelled minimap that turns with you, every creature as its own face, waypoints and death points, a full-screen world map of everywhere you have been. English and Arabic; Arabic mirrors every screen.

## What it shows
- **Minimap** (hold `H` to enlarge it): square or circle, rotating or north-locked, zoom 1–6, compass letters, the faces of nearby players, mobs and items (darker below you), waypoints (clamped to the edge when far), coordinates and optional biome, dimension, time and light.
- **World map** (`M`): explored ground on black, cursor coordinates and biome, zoom, waypoints as pennants, a right-click menu (create waypoint, teleport, share, export PNG) and a waypoint mini-menu.
- **Waypoints** (`N` for the list, `R` to create one here): list, create and edit screens, sets, death points.
- **With [Team](https://github.com/ameliiom/team)** (0.3 or later, optional): each teammate's face on both maps (on the edge of the minimap when far) and every ping as a pennant in its owner's colour, with a sword for danger and the block for loot. Players are then Team's alone: Map's own locator points for other players (their face and name over them in the world, their dots on the maps) step aside so nothing shows twice.
- **Shared ground** (0.4): what either of you explored shows on both maps. Each surface chunk your map sees goes to the server, which keeps it with the world (`map-shared-ground.bin` in the world folder, at most 16,384 chunks, the oldest go first) and sends each player only the chunks others explored and they haven't had yet, in the dimension they are in. A friend's chunk only fills ground your own map has never seen; what you saw yourself stays. Needs Map on the host of the shared world and on both players; ground explored before 0.4 isn't shared.
- **Shared waypoints** (0.5): the waypoints you make and your death points show on your friends' minimap and world map, in your colour (Team's colour for you when Team is installed, light blue otherwise), with nobody pressing anything: sharing is automatic (0.5.1 removed the old Share button, chat link words and Ctrl+V paste). Edits and deletes follow. Only you change your waypoints: a friend sees them but can't edit them. Each set has its own switch in the set list (**Shared With Friends: On/Off**, on by default), and the setting **Share waypoints with friends** turns the whole thing off. The host's world keeps them (`map-shared-points.txt`, at most 256 waypoints a player), so a friend who joins later or rejoins sees them all, and changes travel as deltas.
- Cave view, chunk grid, slime chunks, light overlay, footsteps, PNG export, and import of Xaero's waypoints and settings.
- **Light marks in the world** (with [Skills](https://github.com/ameliiom/skills) 0.2.1 or later, optional): unlock **Spawn Sight** in the Mining tree, and while you hold a torch the light overlay's rule is drawn on the ground around you too: a red cross where the light is 0 (monsters spawn), yellow where it is still low. Same settings as the overlay (light source, min, max, colour); its own switch **Light marks in the world**.

![Light marks](docs/light-marks.png)

## Map icons
Every creature on the maps has an icon of one size. Map draws it from the creature's own model: the front of its head with whatever stands on it. A resource pack or a mod can supply its own icon instead, for any entity of any namespace:

```
assets/<namespace>/textures/map/icons/<entity>.png
assets/<namespace>/textures/map/icons/<entity>/<variant>.png
```

`<entity>` is the entity type's path (`pig` for `minecraft:pig`, `frost_wolf` for `acme:frost_wolf`). `<variant>` is the name of the texture the entity is drawn with (`cold_cow`), and it is used first when it exists. Use a square image such as 8×8 or 16×16. Map scales it to the icon box and draws the black outline itself.

## Install
Needs [Kit](https://github.com/ameliiom/kit) 0.1 or later, the shared code of these mods, installed the same way (`kit.hmod`, `mod.kit:true`).

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
