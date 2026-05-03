# cs2-gsi-webui

[![CI](https://github.com/ccc007ccc/cs2-gsi-webui/actions/workflows/ci.yml/badge.svg)](https://github.com/ccc007ccc/cs2-gsi-webui/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.86-orange.svg)](https://blog.rust-lang.org/)
[![Built on cs2-gsi](https://img.shields.io/badge/built%20on-cs2--gsi-success)](https://github.com/ccc007ccc/cs2-gsi)

**English** · [简体中文](README.zh-CN.md)

A single-binary live web dashboard for **Counter-Strike 2 Game State Integration**, built on top of [`cs2-gsi`](https://github.com/ccc007ccc/cs2-gsi).

Plug it in, launch CS2, open `http://localhost:3000` — and watch the match live: scoreboard, killfeed (with headshot / knife / grenade tags), round timeline, bomb events, full player roster (in spectator).

---

## Features

- **Drop-in**: writes `gamestate_integration_*.cfg` into the CS2 cfg directory automatically (Steam path discovery).
- **Multi-mode**: works with competitive, wingman, casual, deathmatch — `map.mode` is shown in the top bar.
- **Live killfeed**:
  - ☠ headshot tag
  - 💥 grenade-kill tag (HE / molotov / inferno)
  - 🔪 knife-kill tag
  - ⭐ "YOU" tag for the local player's kills
  - 💀 dedicated banner for the local player's death
- **Scoreboard**: map · mode · round phase · CT/T scores.
- **Aggregate stats**: total kills · headshots · grenade kills · knife kills.
- **Round timeline**: round phases, bomb lifecycle, score changes, match start/end.
- **Player table**: HP bar, money, K/A/D, MVPs (full roster only when spectating).
- **Bilingual UI**: English / Simplified Chinese, auto-detected, manual switch in the top bar.
- **Diagnostics panel** (`--raw`): per-tick GSI summary to verify what CS2 is actually pushing.
- **Single binary**: HTML / CSS / JS embedded via `include_str!`. No Node, no bundler.

## Quick start

Requires Rust 1.86+ and CS2 installed via Steam.

```bash
# clone alongside cs2-gsi (the binary references it via path)
git clone https://github.com/ccc007ccc/cs2-gsi.git
git clone https://github.com/ccc007ccc/cs2-gsi-webui.git
cd cs2-gsi-webui

# default ports: GSI 57530 / Web 3000
cargo run --release
```

Open `http://localhost:3000`, launch CS2, join a match.

### CLI

```text
Usage: cs2-gsi-webui [OPTIONS]

Options:
      --gsi-port <GSI_PORT>    Port CS2 will POST GSI payloads to [default: 57530]
      --web-port <WEB_PORT>    Port the web UI listens on [default: 3000]
      --no-cfg                 Skip auto-writing gamestate_integration_*.cfg
      --print-cfg              Print the rendered cfg to stdout and exit
      --app-name <APP_NAME>    Service name (cfg filename + KV key) [default: GsiWebUi]
      --raw                    Stream raw GSI summaries to the diagnostics panel
  -h, --help                   Print help
  -V, --version                Print version
```

## Architecture

```text
┌────────────┐   POST JSON    ┌──────────────────┐
│ CS2 client │───────────────▶│   cs2-gsi crate  │
└────────────┘                │  • parse → State │
                              │  • diff → events │
                              └────────┬─────────┘
                                       │ typed events
                                       ▼
                              ┌──────────────────┐
                              │ tokio::broadcast │
                              └────────┬─────────┘
                                       │ JSON
                                       ▼
                              ┌──────────────────┐
                              │  axum WebSocket  │ ───▶ http://localhost:3000
                              │  + static HTML   │
                              └──────────────────┘
```

## Matchmaking vs spectator

Per [Valve's GSI docs](https://developer.valvesoftware.com/wiki/Counter-Strike:_Global_Offensive_Game_State_Integration), the `allplayers_*`, root-level `bomb`, and `allgrenades` blocks are **only sent to GOTV / HLTV / observer roles**. Exposing them to a live player would effectively be a sanctioned wallhack.

| Data | Matchmaking / Casual | Spectator / GOTV |
|---|:---:|:---:|
| Map · Mode · Round phase | ✅ | ✅ |
| CT / T score | ✅ | ✅ |
| Local player HP / money / kills | ✅ | ✅ |
| All players' HP / position / weapons | ❌ | ✅ |
| Synthetic killfeed (`KillFeed` event) | ❌ | ✅ |
| Bomb position / countdown | ❌ | ✅ |
| Live grenades | ❌ | ✅ |

In matchmaking the dashboard transparently falls back to **local-only mode** and shows a notice explaining the restriction. Self-kills (with weapon + headshot) and self-deaths still render via `PlayerGotKill` / `PlayerDied`.

## Troubleshooting

```bash
# 1. Verify the cfg renders correctly without launching the listener:
cargo run --release -- --print-cfg

# 2. Run with the diagnostics panel enabled, then expand it in the browser:
cargo run --release -- --raw
```

The diagnostics panel shows per-tick:

- `has_map / has_round / has_player` (should all be `true` once you join a match)
- `allplayers_len` (will be `0` outside spectator — that is expected)
- `player.match_kills`, `map.ct_score`, etc.

## License

Licensed under either of:

- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- **MIT license** ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms
or conditions.

## Acknowledgements

Built on [`cs2-gsi`](https://github.com/ccc007ccc/cs2-gsi), itself a port of [`antonpup/CounterStrike2GSI`](https://github.com/antonpup/CounterStrike2GSI).
