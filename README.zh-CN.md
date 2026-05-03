# cs2-gsi-webui

[![CI](https://github.com/ccc007ccc/cs2-gsi-webui/actions/workflows/ci.yml/badge.svg)](https://github.com/ccc007ccc/cs2-gsi-webui/actions)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#许可)
[![MSRV](https://img.shields.io/badge/MSRV-1.86-orange.svg)](https://blog.rust-lang.org/)
[![Built on cs2-gsi](https://img.shields.io/badge/built%20on-cs2--gsi-success)](https://github.com/ccc007ccc/cs2-gsi)

[English](README.md) · **简体中文**

基于 [`cs2-gsi`](https://github.com/ccc007ccc/cs2-gsi) 的单二进制 **CS2 实时对局 Web 仪表盘**。

启动后浏览器打开 `http://localhost:3000`，进入 CS2 加入对局，即可实时看到比分板、击杀播报（爆头 / 刀杀 / 道具杀标签）、回合时间线、炸弹事件，以及完整玩家列表（观战模式）。

---

## 功能

- **零配置部署**：自动通过 Steam 库定位 CS2 cfg 目录并写入 `gamestate_integration_*.cfg`。
- **多模式兼容**：竞技 / Wingman / 休闲 / 死斗 通用，顶栏显示当前 `map.mode`。
- **实时击杀播报**：
  - ☠ 爆头标签
  - 💥 道具击杀标签（HE / 燃烧瓶 / 燃烧弹）
  - 🔪 刀杀标签
  - ⭐ 自己击杀的 "YOU" 高亮
  - 💀 自己阵亡的红色横条
- **比分板**：地图 · 模式 · 回合阶段 · CT/T 比分。
- **聚合统计**：总击杀 · 爆头数 · 道具杀 · 刀杀。
- **回合时间线**：回合阶段切换、炸弹生命周期、比分变化、比赛起止。
- **玩家列表**：HP 血条、金钱、K/A/D、MVP（仅观战时是完整名单）。
- **双语 UI**：中文 / 英文，自动检测浏览器语言，顶栏可手动切换。
- **诊断面板**（`--raw`）：实时显示 GSI 每帧字段摘要，确认 CS2 是否在推送。
- **单二进制**：HTML/CSS/JS 通过 `include_str!` 嵌入，免 Node、免打包工具。

## 快速开始

需要 Rust 1.86+ 与已通过 Steam 安装的 CS2。

```bash
# 把上游 cs2-gsi 与本仓库 clone 到同级目录（用 path 引用）
git clone https://github.com/ccc007ccc/cs2-gsi.git
git clone https://github.com/ccc007ccc/cs2-gsi-webui.git
cd cs2-gsi-webui

# 默认端口：GSI 57530 / Web 3000
cargo run --release
```

浏览器打开 `http://localhost:3000`，启动 CS2 加入对局即可。

### 命令行参数

```text
Usage: cs2-gsi-webui [OPTIONS]

Options:
      --gsi-port <GSI_PORT>    CS2 推送 GSI 的端口 [default: 57530]
      --web-port <WEB_PORT>    Web UI 监听端口 [default: 3000]
      --no-cfg                 不自动写入 gamestate_integration_*.cfg
      --print-cfg              只打印将要写入的 cfg 内容并退出
      --app-name <APP_NAME>    服务名（影响 cfg 文件名与首行 KV key）[default: GsiWebUi]
      --raw                    把每帧 GSI 字段摘要推到前端诊断面板
  -h, --help                   打印帮助
  -V, --version                打印版本
```

## 架构

```text
┌────────────┐   POST JSON    ┌──────────────────┐
│  CS2 客户端 │───────────────▶│   cs2-gsi crate  │
└────────────┘                │  • 解析 → State  │
                              │  • diff → 事件   │
                              └────────┬─────────┘
                                       │ 强类型事件
                                       ▼
                              ┌──────────────────┐
                              │ tokio::broadcast │
                              └────────┬─────────┘
                                       │ JSON
                                       ▼
                              ┌──────────────────┐
                              │  axum WebSocket  │ ───▶ http://localhost:3000
                              │  + 静态 HTML     │
                              └──────────────────┘
```

## 天梯模式 vs 观战模式

根据 [Valve 官方 GSI 文档](https://developer.valvesoftware.com/wiki/Counter-Strike:_Global_Offensive_Game_State_Integration)，`allplayers_*`、根级 `bomb`、`allgrenades` **只对 GOTV / HLTV / 观察者角色推送** —— 因为把这些数据推给正在玩的玩家就等于官方挂。

| 数据 | 天梯 / 休闲 | 观战 / GOTV |
|---|:---:|:---:|
| 地图 · 模式 · 回合阶段 | ✅ | ✅ |
| CT / T 比分 | ✅ | ✅ |
| 本地玩家 HP / 金钱 / 击杀 | ✅ | ✅ |
| 全员 HP / 位置 / 武器 | ❌ | ✅ |
| 合成 killfeed（`KillFeed` 事件） | ❌ | ✅ |
| 炸弹位置 / 倒计时 | ❌ | ✅ |
| 实时道具 | ❌ | ✅ |

天梯模式下程序自动降级为**本机视角**并显示提示说明限制原因。本地玩家的击杀（含武器、爆头）与阵亡仍能通过 `PlayerGotKill` / `PlayerDied` 推导显示。

## 诊断步骤

```bash
# 1. 不启动监听器，先确认 cfg 渲染没问题：
cargo run --release -- --print-cfg

# 2. 启用诊断面板，浏览器底部展开 "🛠 诊断"：
cargo run --release -- --raw
```

诊断面板每帧显示：

- `has_map / has_round / has_player`（进对局后应该全 `true`）
- `allplayers_len`（天梯下永远是 `0`，正常）
- `player.match_kills`、`map.ct_score` 等

## 许可

可任选：

- **Apache License, Version 2.0**（[LICENSE-APACHE](LICENSE-APACHE) 或 <http://www.apache.org/licenses/LICENSE-2.0>）
- **MIT license**（[LICENSE-MIT](LICENSE-MIT) 或 <http://opensource.org/licenses/MIT>）

### 贡献

除非你显式声明，任何你有意提交以纳入本项目的贡献，将按 Apache-2.0 协议中的定义双授权（MIT 与 Apache-2.0），不附加任何额外条款。

## 致谢

基于 [`cs2-gsi`](https://github.com/ccc007ccc/cs2-gsi)，后者是 [`antonpup/CounterStrike2GSI`](https://github.com/antonpup/CounterStrike2GSI) 的 Rust 移植。
