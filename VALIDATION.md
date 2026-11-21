# Trailgrid Maps 0.1.0 release checks

| Item | Result |
| --- | --- |
| Package | `trailgrid_maps-0.1.0.hmod`, 309,784 bytes, unsigned native source package |
| SHA-256 | `0ec0f3cc57e1d1565e76b67ba787a7b5fc45827a2cf728676c17b175b71b8b93` |
| Target | Minecraft Java Edition 26.3, `nightly-2026-11-20-0a875da` |
| Horizon revision | `0a875da91dbb0fdcb7ba8c42c11341aa229af51a` |
| Mixins revision | `e59da9094f0c2fdda6c627fe9eff22dec6a906f0` |
| Exact-SDK build | Horizon Launcher 0.2.93 built the packaged mod in a clean Apple Silicon macOS test instance and selected `native-0a875da-d59fed1a454b`. |
| Package checks | `cargo horizon-mod check` accepted the package as `native-mod`. Packaged source and assets match the release source. |
| English strings | The `en_us` table has 285 entries. All statically named SDK labels were matched against the table. |
| Local play | English settings, waypoint creation, case-insensitive search, selection, map navigation, and return with Escape passed in a disposable world. A setting changed in-game persisted to disk. |
| Existing data | Copied a previous build's test cache and data byte-for-byte to the new mod directory. Horizon opened a copy of the original world after backing it up. The new mod loaded its saved setting and both previous waypoints and opened the map using the existing world identity. |
| Rust tests | 132 passed, 0 failed, 3 ignored. |

Windows and Linux runs, live Horizon SMP, and the catalog review are outside these checks.
