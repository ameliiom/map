# trail 0.1.1 release checks

| Item | Result |
| --- | --- |
| Package | `trailgrid_maps-0.1.1.hmod`, 309,770 bytes, unsigned native source package |
| SHA-256 | `7739fb61259e1bb8717c60e33e2d6233ca7549e0522b1e130781cd87791bb670` |
| Target | Minecraft Java Edition 26.3, `nightly-2026-11-21-8f83e5c` |
| Horizon revision | `8f83e5c85ae3b9a117f742e8d91170c5c9080a06` |
| Mixins revision | `e59da9094f0c2fdda6c627fe9eff22dec6a906f0` |
| Exact-SDK build | Horizon Launcher 0.2.93 built the release `.hmod` in an Apple Silicon macOS test profile and selected `native-8f83e5c-73a2a4d7a242`. |
| Package check | `cargo horizon-mod check` accepted the package as `native-mod`. |
| Rust tests | 132 passed, 0 failed, 3 ignored using the verified API source for the Nov 21 SDK. |
| Local play | Through the launcher, opened a disposable copy of the previous test world. Existing waypoints and the saved map setting loaded. The settings menu, persistence, waypoint creation and search, and navigation to the selected waypoint passed. |
| Existing data | Copied the v0.1.0 test cache and data byte-for-byte to the new test profile before starting. The mod uses the same data path for both versions. |

Live Horizon SMP, Windows, and Linux were outside these checks. See [v0.1.0 validation](https://github.com/groznen95/TailgridMaps/blob/v0.1.0/VALIDATION.md) for the November 20 build.
