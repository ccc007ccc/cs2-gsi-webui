//! # cs2-gsi-webui
//!
//! A live web dashboard for [`cs2-gsi`].
//!
//! Bridges the GSI listener to the browser:
//! 1. Receive Counter-Strike 2 GSI pushes on `--gsi-port`
//! 2. Normalise typed events into JSON
//! 3. Broadcast them via WebSocket to all connected browsers
//! 4. Serve a single-file `index.html` UI
//!
//! ```text
//! cargo run --release
//! cargo run --release -- --gsi-port 57530 --web-port 3000
//! cargo run --release -- --print-cfg     # print rendered cfg and exit
//! cargo run --release -- --raw           # also stream raw payload summaries
//! ```
//!
//! ## Matchmaking vs spectator
//!
//! Valve intentionally restricts `allplayers_*`, root-level `bomb`, and
//! `allgrenades` to GOTV / HLTV / observer roles. If you are playing a
//! competitive / casual match, this dashboard can only display:
//!
//! - your own kills / damage / death (via `PlayerGotKill`, `PlayerDied`, …)
//! - score (via `map.team_ct.score` / `map.team_t.score`)
//! - round phase, map, mode
//!
//! For the full match (entire roster, killfeed sourced from all players,
//! grenade trails) you must be in spectator / GOTV / replay.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use clap::Parser;
use cs2_gsi::cfg::GsiCfg;
use cs2_gsi::events::{
    BombDefused, BombDropped, BombExploded, BombPickedUp, BombPlanted, BombPlanting, GameOver,
    KillFeed, MatchStarted, NewGameState, PlayerDied, PlayerGotKill, PlayerTookDamage,
    RoundConcluded, RoundPhaseUpdated, RoundStarted, TeamScoreChanged,
};
use cs2_gsi::model::{GameState, MapPhase, Player, PlayerTeam, RoundPhase, WinningTeam};
use cs2_gsi::GameStateListener;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

// ---------------- CLI ----------------

#[derive(Parser, Debug, Clone)]
#[command(author, version, about = "Web UI dashboard for cs2-gsi")]
struct Cli {
    /// Port CS2 will POST GSI payloads to.
    #[arg(long, default_value_t = 57530)]
    gsi_port: u16,

    /// Port the web UI listens on.
    #[arg(long, default_value_t = 3000)]
    web_port: u16,

    /// Skip auto-writing `gamestate_integration_*.cfg` into the CS2 cfg dir.
    #[arg(long, default_value_t = false)]
    no_cfg: bool,

    /// Print the rendered cfg to stdout and exit (useful for debugging).
    #[arg(long, default_value_t = false)]
    print_cfg: bool,

    /// Service name — affects the cfg filename and the top-level KeyValues key.
    #[arg(long, default_value = "GsiWebUi")]
    app_name: String,

    /// Stream a per-tick raw GSI field summary to the diagnostics panel.
    /// Heavier traffic — useful only for debugging.
    #[arg(long, default_value_t = false)]
    raw: bool,
}

// ---------------- WebSocket protocol ----------------

/// Normalised event sent to browsers. The `type` tag drives rendering.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
enum UiEvent {
    /// A kill. `source = "killfeed"` is the synthesised event (requires
    /// `allplayers`, only available in spectator). `source = "self"` is
    /// derived from the local player's `round_kills` delta — the only
    /// available source while playing matchmaking / casual.
    #[serde(rename = "killfeed")]
    Killfeed {
        ts: u64,
        source: &'static str,
        killer: String,
        killer_team: String,
        victim: String,
        victim_team: String,
        weapon: String,
        weapon_kind: String,
        headshot: bool,
        knife: bool,
        grenade: bool,
    },
    /// Local player died (works without `allplayers`).
    #[serde(rename = "self_died")]
    SelfDied {
        ts: u64,
        player: String,
        previous_health: i32,
    },
    /// Local player took non-lethal damage.
    #[serde(rename = "self_damage")]
    SelfDamage {
        ts: u64,
        player: String,
        previous: i32,
        new: i32,
    },
    /// Lightweight match snapshot.
    #[serde(rename = "snapshot")]
    Snapshot {
        ts: u64,
        map: String,
        mode: String,
        phase: String,
        round: u32,
        score_ct: u32,
        score_t: u32,
        /// `true` when full spectator data (`allplayers`) is available.
        spectator_view: bool,
        /// Player list. Empty in main menu, single entry in matchmaking.
        players: Vec<PlayerView>,
    },
    /// Round phase transition.
    #[serde(rename = "round")]
    Round { ts: u64, phase: String, round: u32 },
    /// Round ended with a winner.
    #[serde(rename = "round_end")]
    RoundEnd { ts: u64, winner: String },
    /// Bomb lifecycle event.
    #[serde(rename = "bomb")]
    Bomb { ts: u64, state: String },
    /// Team score change.
    #[serde(rename = "score")]
    Score {
        ts: u64,
        team: String,
        previous: u32,
        new: u32,
    },
    /// Match meta event (started / gameover).
    #[serde(rename = "match")]
    Match { ts: u64, kind: String },
    /// System message used for connection acks.
    #[serde(rename = "system")]
    System { ts: u64, message: String },
    /// Raw payload summary (only when `--raw` is enabled).
    #[serde(rename = "raw")]
    Raw { ts: u64, payload: serde_json::Value },
}

#[derive(Clone, Debug, Serialize)]
struct PlayerView {
    steamid: String,
    name: String,
    team: String,
    health: i32,
    armor: i32,
    money: i32,
    kills: i32,
    assists: i32,
    deaths: i32,
    mvps: i32,
    score: i32,
    round_kills: i32,
    round_hs: i32,
    is_alive: bool,
    is_local: bool,
    weapon_active: Option<String>,
}

// ---------------- shared state ----------------

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<UiEvent>,
    listener: Arc<GameStateListener>,
}

// ---------------- helpers ----------------

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn team_str(t: &PlayerTeam) -> String {
    match t {
        PlayerTeam::CT => "CT".to_string(),
        PlayerTeam::T => "T".to_string(),
        PlayerTeam::Unassigned => "SPEC".to_string(),
    }
}

fn round_phase_str(p: &RoundPhase) -> String {
    match p {
        RoundPhase::Freezetime => "freezetime",
        RoundPhase::Live => "live",
        RoundPhase::Over => "over",
        RoundPhase::Unknown => "unknown",
    }
    .to_string()
}

fn map_phase_str(p: &MapPhase) -> String {
    match p {
        MapPhase::Warmup => "warmup",
        MapPhase::Live => "live",
        MapPhase::Intermission => "intermission",
        MapPhase::Gameover => "gameover",
        MapPhase::Unknown => "unknown",
    }
    .to_string()
}

fn winning_team_str(w: &WinningTeam) -> String {
    match w {
        WinningTeam::Ct => "ct".to_string(),
        WinningTeam::T => "t".to_string(),
        WinningTeam::None => "none".to_string(),
    }
}

/// Classify a CS2 weapon class name (`weapon_*`) into a coarse bucket
/// the UI uses for badge rendering.
fn classify_weapon(weapon: &str) -> &'static str {
    let w = weapon.to_lowercase();
    let w = w.trim_start_matches("weapon_");

    if w.starts_with("knife") || w == "bayonet" || w.contains("knife") {
        return "knife";
    }
    if matches!(
        w,
        "hegrenade"
            | "inferno"
            | "molotov"
            | "incgrenade"
            | "decoy"
            | "flashbang"
            | "smokegrenade"
            | "frag"
            | "firebomb"
    ) {
        return "grenade";
    }
    if matches!(w, "awp" | "ssg08" | "scar20" | "g3sg1") {
        return "sniper";
    }
    if matches!(w, "nova" | "xm1014" | "mag7" | "sawedoff") {
        return "shotgun";
    }
    if matches!(
        w,
        "mp9" | "mac10" | "mp7" | "ump45" | "p90" | "bizon" | "mp5sd"
    ) {
        return "smg";
    }
    if matches!(w, "m249" | "negev") {
        return "mg";
    }
    if matches!(
        w,
        "glock"
            | "usp_silencer"
            | "hkp2000"
            | "p250"
            | "fiveseven"
            | "tec9"
            | "cz75a"
            | "deagle"
            | "elite"
            | "revolver"
    ) {
        return "pistol";
    }
    if matches!(
        w,
        "ak47"
            | "m4a1"
            | "m4a4"
            | "m4a1_silencer"
            | "famas"
            | "galilar"
            | "aug"
            | "sg556"
            | "sg553"
            | "taser"
    ) {
        return "rifle";
    }
    "other"
}

fn player_view(p: &Player, sid: &str, is_local: bool) -> PlayerView {
    let active_weapon = p
        .weapons
        .values()
        .find(|w| matches!(w.state, cs2_gsi::model::WeaponState::Active))
        .map(|w| w.name.clone());
    PlayerView {
        steamid: sid.to_string(),
        name: p.name.clone(),
        team: team_str(&p.team),
        health: p.state.health,
        armor: p.state.armor,
        money: p.state.money,
        kills: p.match_stats.kills,
        assists: p.match_stats.assists,
        deaths: p.match_stats.deaths,
        mvps: p.match_stats.mvps,
        score: p.match_stats.score,
        round_kills: p.state.round_kills,
        round_hs: p.state.round_killhs,
        is_alive: p.state.health > 0,
        is_local,
        weapon_active: active_weapon,
    }
}

/// Build a snapshot. Emitted as long as **either** `map` or `player` is
/// present — does not require `allplayers` (which is spectator-only).
fn build_snapshot(state: &GameState) -> Option<UiEvent> {
    if state.map.is_none() && state.player.is_none() {
        return None;
    }

    let map_name = state
        .map
        .as_ref()
        .map(|m| m.name.clone())
        .unwrap_or_default();
    let mode = state
        .map
        .as_ref()
        .map(|m| m.mode.clone())
        .unwrap_or_default();
    let round = state.map.as_ref().map(|m| m.round).unwrap_or(0);
    let score_ct = state.map.as_ref().map(|m| m.team_ct.score).unwrap_or(0);
    let score_t = state.map.as_ref().map(|m| m.team_t.score).unwrap_or(0);

    let phase = state
        .round
        .as_ref()
        .map(|r| round_phase_str(&r.phase))
        .or_else(|| state.map.as_ref().map(|m| map_phase_str(&m.phase)))
        .unwrap_or_else(|| "unknown".to_string());

    let mut players: Vec<PlayerView> = Vec::new();
    let local_steamid = state.player.as_ref().map(|p| p.steamid.clone());

    if !state.allplayers.is_empty() {
        // Spectator / GOTV: full roster
        players.extend(state.allplayers.iter().map(|(sid, p)| {
            let is_local = local_steamid.as_deref() == Some(sid.as_str());
            player_view(p, sid, is_local)
        }));
    } else if let Some(p) = state.player.as_ref() {
        // Matchmaking / casual: local player only
        players.push(player_view(p, &p.steamid, true));
    }

    players.sort_by(|a, b| {
        a.team
            .cmp(&b.team)
            .then(b.score.cmp(&a.score))
            .then(a.name.cmp(&b.name))
    });

    Some(UiEvent::Snapshot {
        ts: now_ts(),
        map: map_name,
        mode,
        phase,
        round,
        score_ct,
        score_t,
        spectator_view: !state.allplayers.is_empty(),
        players,
    })
}

/// Compact summary of the current `GameState` for the diagnostics panel.
fn raw_summary(state: &GameState) -> serde_json::Value {
    serde_json::json!({
        "has_map": state.map.is_some(),
        "has_round": state.round.is_some(),
        "has_player": state.player.is_some(),
        "allplayers_len": state.allplayers.len(),
        "grenades_len": state.grenades.len(),
        "has_bomb": state.bomb.is_some(),
        "map": state.map.as_ref().map(|m| serde_json::json!({
            "name": m.name,
            "mode": m.mode,
            "phase": format!("{:?}", m.phase),
            "round": m.round,
            "ct_score": m.team_ct.score,
            "t_score": m.team_t.score,
        })),
        "round": state.round.as_ref().map(|r| serde_json::json!({
            "phase": format!("{:?}", r.phase),
            "bomb": format!("{:?}", r.bomb),
            "win_team": format!("{:?}", r.win_team),
        })),
        "player": state.player.as_ref().map(|p| serde_json::json!({
            "steamid": p.steamid,
            "name": p.name,
            "team": format!("{:?}", p.team),
            "activity": format!("{:?}", p.activity),
            "health": p.state.health,
            "armor": p.state.armor,
            "money": p.state.money,
            "round_kills": p.state.round_kills,
            "round_hs": p.state.round_killhs,
            "match_kills": p.match_stats.kills,
            "match_deaths": p.match_stats.deaths,
            "weapons": p.weapons.len(),
        })),
    })
}

// ---------------- GSI event wiring ----------------

fn wire_listener(listener: &GameStateListener, tx: broadcast::Sender<UiEvent>, raw_enabled: bool) {
    // Synthesised killfeed (spectator only)
    {
        let tx = tx.clone();
        listener.on(move |e: &KillFeed| {
            let weapon = e.weapon.clone().unwrap_or_else(|| "unknown".to_string());
            let kind = classify_weapon(&weapon);
            let _ = tx.send(UiEvent::Killfeed {
                ts: now_ts(),
                source: "killfeed",
                killer: e.killer.name.clone(),
                killer_team: team_str(&e.killer.team),
                victim: e.victim.name.clone(),
                victim_team: team_str(&e.victim.team),
                weapon,
                weapon_kind: kind.to_string(),
                headshot: e.is_headshot,
                knife: kind == "knife",
                grenade: kind == "grenade",
            });
        });
    }

    // Local kills (works in matchmaking — derived from round_kills delta)
    {
        let tx = tx.clone();
        listener.on(move |e: &PlayerGotKill| {
            let weapon = e.weapon.clone().unwrap_or_else(|| "unknown".to_string());
            let kind = classify_weapon(&weapon);
            let _ = tx.send(UiEvent::Killfeed {
                ts: now_ts(),
                source: "self",
                killer: e.player.name.clone(),
                killer_team: team_str(&e.player.team),
                victim: "(unknown)".to_string(),
                victim_team: "?".to_string(),
                weapon,
                weapon_kind: kind.to_string(),
                headshot: e.is_headshot,
                knife: kind == "knife",
                grenade: kind == "grenade",
            });
        });
    }

    // Local death
    {
        let tx = tx.clone();
        listener.on(move |e: &PlayerDied| {
            let _ = tx.send(UiEvent::SelfDied {
                ts: now_ts(),
                player: e.player.name.clone(),
                previous_health: e.previous_health,
            });
        });
    }

    // Local damage
    {
        let tx = tx.clone();
        listener.on(move |e: &PlayerTookDamage| {
            let _ = tx.send(UiEvent::SelfDamage {
                ts: now_ts(),
                player: e.player.name.clone(),
                previous: e.previous_health,
                new: e.new_health,
            });
        });
    }

    // Round phase
    {
        let tx = tx.clone();
        listener.on(move |e: &RoundPhaseUpdated| {
            let _ = tx.send(UiEvent::Round {
                ts: now_ts(),
                phase: round_phase_str(&e.new),
                round: 0,
            });
        });
    }
    {
        let tx = tx.clone();
        listener.on(move |_: &RoundStarted| {
            let _ = tx.send(UiEvent::Round {
                ts: now_ts(),
                phase: "live".to_string(),
                round: 0,
            });
        });
    }
    {
        let tx = tx.clone();
        listener.on(move |e: &RoundConcluded| {
            let _ = tx.send(UiEvent::RoundEnd {
                ts: now_ts(),
                winner: winning_team_str(&e.winning_team),
            });
        });
    }

    // Score
    {
        let tx = tx.clone();
        listener.on(move |e: &TeamScoreChanged| {
            let _ = tx.send(UiEvent::Score {
                ts: now_ts(),
                team: team_str(&e.team),
                previous: e.previous,
                new: e.new,
            });
        });
    }

    // Bomb
    macro_rules! bomb_evt {
        ($lst:expr, $tx:expr, $ev:ty, $name:literal) => {{
            let tx = $tx.clone();
            $lst.on(move |_: &$ev| {
                let _ = tx.send(UiEvent::Bomb {
                    ts: now_ts(),
                    state: $name.to_string(),
                });
            });
        }};
    }
    bomb_evt!(listener, tx, BombPlanting, "planting");
    bomb_evt!(listener, tx, BombPlanted, "planted");
    bomb_evt!(listener, tx, BombDefused, "defused");
    bomb_evt!(listener, tx, BombExploded, "exploded");
    bomb_evt!(listener, tx, BombDropped, "dropped");
    bomb_evt!(listener, tx, BombPickedUp, "pickedup");

    // Match
    {
        let tx = tx.clone();
        listener.on(move |_: &MatchStarted| {
            let _ = tx.send(UiEvent::Match {
                ts: now_ts(),
                kind: "started".to_string(),
            });
        });
    }
    {
        let tx = tx.clone();
        listener.on(move |_: &GameOver| {
            let _ = tx.send(UiEvent::Match {
                ts: now_ts(),
                kind: "gameover".to_string(),
            });
        });
    }

    // Per-tick snapshot + optional raw summary
    {
        let tx = tx.clone();
        listener.on(move |e: &NewGameState| {
            if let Some(snap) = build_snapshot(&e.state) {
                let _ = tx.send(snap);
            }
            if raw_enabled {
                let _ = tx.send(UiEvent::Raw {
                    ts: now_ts(),
                    payload: raw_summary(&e.state),
                });
            }
        });
    }
}

// ---------------- HTTP / WS ----------------

const INDEX_HTML: &str = include_str!("../assets/index.html");

async fn index_handler() -> impl IntoResponse {
    Html(INDEX_HTML)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.tx.subscribe();

    // Send the current snapshot immediately so reconnecting clients catch up.
    if let Some(gs) = state.listener.current_state() {
        if let Some(snap) = build_snapshot(&gs) {
            if let Ok(json) = serde_json::to_string(&snap) {
                let _ = sender.send(Message::Text(json)).await;
            }
        }
    }
    let hello = UiEvent::System {
        ts: now_ts(),
        message: "connected".to_string(),
    };
    if let Ok(json) = serde_json::to_string(&hello) {
        let _ = sender.send(Message::Text(json)).await;
    }

    let send_task = tokio::spawn(async move {
        while let Ok(evt) = rx.recv().await {
            let json = match serde_json::to_string(&evt) {
                Ok(s) => s,
                Err(e) => {
                    warn!("serialize event failed: {e}");
                    continue;
                }
            };
            if sender.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }
    debug!("ws client disconnected");
}

// ---------------- main ----------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cs2_gsi_webui=info,cs2_gsi=info".into()),
        )
        .init();

    let cli = Cli::parse();

    // Sub-command: just dump the cfg and exit.
    if cli.print_cfg {
        let cfg = GsiCfg::for_localhost(&cli.app_name, cli.gsi_port);
        println!("{}", cfg.render());
        println!("\n# expected file name: {}", cfg.file_name().display());
        return Ok(());
    }

    // 1. Best-effort: drop the gsi cfg into the CS2 cfg dir.
    if !cli.no_cfg {
        match GsiCfg::for_localhost(&cli.app_name, cli.gsi_port).write_to_cs2() {
            Ok(p) => info!("gsi cfg written -> {}", p.display()),
            Err(e) => warn!(
                "auto-write gsi cfg failed ({e}); copy manually:\n{}",
                GsiCfg::for_localhost(&cli.app_name, cli.gsi_port).render()
            ),
        }
    }

    // 2. Broadcast channel
    let (tx, _) = broadcast::channel::<UiEvent>(1024);

    // 3. GSI listener
    let listener = Arc::new(GameStateListener::new(cli.gsi_port));
    wire_listener(&listener, tx.clone(), cli.raw);
    listener.start().await?;
    info!("gsi listener  -> http://0.0.0.0:{}", cli.gsi_port);
    if cli.raw {
        info!("raw payload summary enabled (visible in the diagnostics panel)");
    }

    // 4. Axum server
    let state = AppState {
        tx: tx.clone(),
        listener: listener.clone(),
    };
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", cli.web_port).parse()?;
    let tcp = tokio::net::TcpListener::bind(addr).await?;
    info!("web ui        -> http://localhost:{}", cli.web_port);
    info!("press Ctrl-C to stop");
    info!("");
    info!("  matchmaking : local kills + score + round phase + map");
    info!("  spectator   : full match (all players, killfeed, grenades)");

    let server = tokio::spawn(async move {
        if let Err(e) = axum::serve(tcp, app).await {
            error!("axum server error: {e}");
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("shutting down ...");
    listener.stop().await?;
    server.abort();
    Ok(())
}
