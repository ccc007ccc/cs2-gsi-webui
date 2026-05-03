# Changelog

All notable changes to **cs2-gsi-webui** are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-05-04

### Added
- Initial public release.
- Axum-based HTTP + WebSocket server bridging [`cs2-gsi`](https://crates.io/crates/cs2-gsi) to a single-file browser UI.
- Auto-writes `gamestate_integration_*.cfg` into the CS2 cfg directory via Steam path discovery.
- Live killfeed with weapon classification (knife / grenade / sniper / rifle / pistol / smg / shotgun / mg).
- Special tags rendered for headshots, knife kills, grenade kills, and the local player ("YOU").
- Scoreboard with map name, mode, round number, CT/T scores.
- Per-match aggregate stats: total kills, headshots, grenade kills, knife kills.
- Round timeline (round phases, bomb lifecycle, score changes, match start/end).
- Player table with HP bar, money, K/A/D, MVP awards.
- Graceful degradation in matchmaking / casual mode (Valve restricts `allplayers_*`
  to spectators / GOTV); the dashboard falls back to local-only data and
  surfaces a notice explaining the limitation.
- Self-kill / self-death rendering using `PlayerGotKill` / `PlayerDied`
  derived from the local `player.round_kills` / `health` deltas.
- Diagnostics panel (`--raw`) streaming a per-tick GSI summary so users can
  verify what CS2 is actually pushing.
- Bilingual UI (English / Simplified Chinese) with auto-detection from
  `navigator.language` and a manual switcher in the top bar.
- CLI: `--gsi-port`, `--web-port`, `--no-cfg`, `--print-cfg`, `--app-name`, `--raw`.

[Unreleased]: https://github.com/ccc007ccc/cs2-gsi-webui/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ccc007ccc/cs2-gsi-webui/releases/tag/v0.1.0
