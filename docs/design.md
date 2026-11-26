# Map: design

The target is Xaero's art direction as observed in the running mods (record: `.study/xaero-look.md`, local, with the
pictures it cites), raised wherever a clearer, more visual form exists. "Xaero" below means what the record shows;
"Better" marks where Map goes past it. Every size is in GUI pixels at the game's GUI scale unless it says px.

## Minimap
- **Place and size**: top-left by default (5 GUI from the edges), map area 135 GUI square (Xaero "Size: Auto"), resizable.
  In a right-to-left language the default corner is top-right.
- **Frame**: four 1-GUI bands, bevelled, the same on square and circle (record §1 Frame): outer #000000; light side
  #939393, #5D5D5D, #101010; dark side #474747, #7E7E7E, #2B2B2B. No shadow outside it.
- **Map**: block colours with terrain slopes and lighting, rotating with the player by default; zoom levels 1–6, N
  screen px per block; icons, waypoints and letters keep their size at every zoom.
- **Compass**: N E S W in the vanilla font at half GUI size, white with #5555FF shadow, on a 9 px #5555FF box at 56%
  alpha, riding the inside of the frame; at 45° they move to the corners; always upright.
- **Player**: a crosshair on a rotating map; the red #CC1919 notched arrow when north is locked.
- **Entities, Better**: heads are the default for every kind (Xaero's default is dots). Each mob is its own flat face
  from its texture, 1 texel = 1 GUI px (8×8 face = 16 px + a 1 px #000000 outline following the silhouette), upright,
  fixed size, anchored at the entity's x/z; dropped items draw their item sprite. Players draw their skin's face (with
  the hat layer). Depth: below the player the face darkens (−10 → 58%, −20 → 25%), beyond ±20 blocks nothing. Better
  than Xaero: a fixed draw order so the important face is on top (players, then hostile, then passive, then items), and
  every mob gets a face (Xaero shows a grey silhouette for the horse).
- **Names** (off by default): the type name under the icon, half-GUI font, white, on #000000 at 35%.
- **Waypoints**: an 18 px square of the waypoint colour at 90% with its initial centred (GUI font, white, #3F3F3F
  shadow), above entities; out of range it is clamped onto the frame on the line towards it. Death point: black square
  with a white skull.
- **Info lines**: centred under the frame, 4 GUI below it, white with the vanilla #3F3F3F shadow, no box, one fact per
  line, 10 GUI pitch; default is coordinates only (`-32, 69, 0`); optional biome, dimension, time, light, angles.

## World map (M)
- The map fills the whole window; unexplored ground is #000000; no frame, no panel, no tabs.
- Top centre: cursor coordinates `X: 87 Y: 81 Z: -76` and the biome under it, white with shadow.
- Bottom centre: the zoom (`3.0x`) on a small translucent black box; wheel steps ×1.2.
- Controls are bare white glyph icons (#FCFCFC, #3E3E3E shadow 1 GUI down-right, #E6E6E6 hovered) in 20 GUI cells on
  the two side edges: gear 30 GUI top-left; left column bottom-up: map switching, cave mode, dimension, hop to
  coordinates, camera; right column bottom-up: waypoint mini-menu, waypoints on/off, tracked players, entity radar,
  export PNG, controls help, zoom out, zoom in. Every icon has a tooltip (dark translucent box, white text, keys green).
  In a right-to-left language the two columns swap sides.
- Player: the red arrow. Entities: the same heads. Waypoints: an opaque pennant of the colour with its initial and a
  dark drop shadow; death point a black pennant with a skull.
- Right-click menu at the cursor: 2 px white border, title row on #606060, rows on black at 80% with 2 px #323232
  separators, centred white text, keys green: chunk and block coordinates, Create Waypoint, Temporary Waypoint, Teleport
  Here, Share Location, Waypoints, Export PNG, Settings, distance.
- Waypoint mini-menu at the bottom right beside the icon column, no panel: a right-aligned list with initials, a set
  dropdown, a filter box and a row of five icon toggles.

## Screens (settings, waypoints, create waypoint)
- Vanilla screens over the blurred, darkened game, built from the game's own widgets (button and edit-box sprites, the
  vanilla font): two columns of 200×20 buttons 10 GUI apart, rows every 24 GUI, Back 200×20 under them, pages with
  `<< Prev` / `Next >>`. Tooltips on hover.
- Waypoint list: rows with the coloured initial square and the name, the selected row black with a 1 GUI white border,
  action buttons 75×20 under it (Add/Edit, Teleport, Share, Enable, Delete) and Done 200×20.
- Create waypoint: world and dimension, colour, name, x/y/z/yaw, initials, Confirm/Cancel 150×20.

## Language
- English and Arabic; Arabic mirrors every screen (columns, lists, the map's icon columns, the default corner); digits
  stay Latin.

## Team
- With Team installed, teammates show their skin face on both maps (also clamped to the frame when out of range) and
  Team's pings show as pennants in the pinger's colour.

## Checks
- One layout function per screen; a test asserts that every part stays inside its container and none overlap, for
  short, long and empty names, many entities and every corner.

## Light marks in the world (0.3.0, 2026-11-26)

The light overlay's rule (`light.rs`, one `Rule` for both views: block or sky light between
`light_min` and `light_max`, red at 0 and yellow above, or the chosen colour) is also drawn in the
world (`marks.rs`): while a torch is in either hand, a cross on every opaque full block within 16
blocks (8 up and down) with two airs above whose spot the rule marks. Found again only on a move,
a change of the rule's settings, or a block or light packet (2 and 12 ticks later); drawn as lines
each frame. Moved here from Hands 0.3 (its own spawn marks repeated this overlay).

- Earned with Map's node **Spawn Sight** (`data/map/skills/mining.json`, extends Skills' Mining,
  tag `map.marks`); the client asks Skills (`skills::has_tag`, optional dependency `skills ^0.2.1`).
  Without Skills the marks never show.
- Switch: `light_marks` (on by default).

| Check (in the game) | Result |
|---|---|
| Mining tab | Spawn Sight at row 4, beside Lucky; Gathering's and Hands' nodes on their own spots |
| Node unlocked, torch held in a dark stone room | a red cross on every floor block |
| A torch placed in the room (room lit to 8+) | the crosses gone |
| Pickaxe in hand | no crosses |

