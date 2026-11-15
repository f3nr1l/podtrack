//! podtrack : suivi de tête par AirPods sous Linux (portage Rust de MousePods).

mod config;
mod daemon;
mod dbus;
mod games;
mod hub;
mod import;
mod signals;
mod steam;
mod tray;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use config::{Game, GameKind};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "podtrack", version, about = "Suivi de tête par AirPods (jeux TrackIR/Proton, opentrack, souris)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Lance le démon (canal Bluetooth, sorties, service D-Bus, icône de barre).
    Daemon {
        /// Sans icône dans la barre du haut.
        #[arg(long)]
        no_tray: bool,
    },
    /// État du démon.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Affiche les angles en direct (Ctrl+C pour quitter).
    Watch,
    /// Recentre la tête.
    Recenter,
    /// Démarre le suivi.
    Start,
    /// Arrête le suivi (les jeux recentrent), le démon reste lancé.
    Stop,
    /// Bascule démarré / arrêté.
    Toggle,
    /// Arrête le démon.
    Quit,
    /// Choisit le traqueur d'un jeu : podtrack, ou other (Beam, opentrack…). Jeu fermé.
    Tracker { game: String, tracker: String },
    /// Gère la liste des jeux.
    Game {
        #[command(subcommand)]
        cmd: GameCmd,
    },
    /// Calibrations.
    Calibrate {
        #[command(subcommand)]
        cmd: CalibCmd,
    },
    /// Reprend la configuration de MousePods.
    ImportMousepods {
        /// config.json de MousePods.
        path: PathBuf,
        /// Correspondance d'identifiant, ex. 900000001=star-citizen (répétable).
        #[arg(long = "map")]
        map: Vec<String>,
    },
    /// Configuration.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
}

#[derive(Subcommand)]
enum GameCmd {
    /// Liste les jeux configurés.
    List,
    /// Ajoute un jeu Wine hors Steam (préfixe), ex. Star Citizen par lug-helper.
    AddWine { id: String, name: String, prefix: PathBuf },
    /// Ajoute un jeu Steam (appid ; --flatpak pour Steam en Flatpak).
    AddSteam {
        id: String,
        appid: String,
        #[arg(long)]
        flatpak: bool,
    },
    /// Retire un jeu de la configuration (rend d'abord la main avec `tracker <jeu> other`).
    Remove { id: String },
    /// Jeux Steam installés.
    Steam,
}

#[derive(Subcommand)]
enum CalibCmd {
    /// Axes de l'écouteur (7 étapes guidées dans le terminal).
    Device,
    /// Amplitude par cinq points regardés (centre, gauche, droite, haut, bas) ; cible : baseline ou un jeu.
    Game {
        target: String,
        #[arg(long, default_value_t = 180.0)]
        target_side: f64,
        #[arg(long, default_value_t = 90.0)]
        target_vert: f64,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Chemin du fichier.
    Path,
    /// Configuration complète avec toutes les valeurs par défaut.
    Defaults,
    /// Configuration actuelle, complétée des valeurs par défaut.
    Show,
}

fn status() -> Result<serde_json::Value> {
    let s: String = dbus::call("Status", &())?;
    Ok(serde_json::from_str(&s)?)
}

fn print_status(v: &serde_json::Value) {
    let conn = &v["connection"];
    let state = conn["state"].as_str().unwrap_or("?");
    let detail = conn["detail"].as_str().map(|d| format!(" ({d})")).unwrap_or_default();
    println!("Suivi        : {}", if v["active"].as_bool() == Some(true) { "démarré" } else { "arrêté" });
    println!("Connexion    : {state}{detail}");
    if let Some(d) = v["device"].as_object() {
        println!("AirPods      : {} — {} ({})", d["name"].as_str().unwrap_or(""), d["model"].as_str().unwrap_or(""), d["address"].as_str().unwrap_or(""));
    }
    if let Some(b) = v["battery"].as_array().filter(|b| !b.is_empty()) {
        let parts: Vec<String> = b
            .iter()
            .map(|p| {
                let name = match p["part"].as_u64() { Some(1) => "seul", Some(2) => "droit", Some(4) => "gauche", Some(8) => "boîtier", _ => "?" };
                format!("{name} {}%", p["level"])
            })
            .collect();
        println!("Batterie     : {}", parts.join(", "));
    }
    println!(
        "Profil       : {} {}",
        v["profile"].as_str().unwrap_or("-"),
        if v["profile_calibrated"].as_bool() == Some(true) { "" } else { "(non calibré)" }
    );
    println!("Échantillons : {} à {} Hz, source {}, biais {}", v["samples"], v["rate_hz"], v["source"].as_str().unwrap_or(""), if v["bias_ok"].as_bool() == Some(true) { "ok" } else { ".." });
    let h = &v["head"];
    println!("Tête         : lacet {:+.1}°  tangage {:+.1}°  roulis {:+.1}°", h[0].as_f64().unwrap_or(0.0), h[1].as_f64().unwrap_or(0.0), h[2].as_f64().unwrap_or(0.0));
    if let Some(g) = v["games"].as_object() {
        for (id, x) in g {
            let p = &x["pose"];
            println!("Jeu {id:<9}: caméra {:+.1}° {:+.1}° {:+.1}° (seq {})", p[3].as_f64().unwrap_or(0.0), p[4].as_f64().unwrap_or(0.0), p[5].as_f64().unwrap_or(0.0), x["seq"]);
        }
    }
    if let Some(e) = v["last_error"].as_str() {
        println!("Dernière erreur : {e}");
    }
}

fn step_text(step: &str) -> &'static str {
    match step {
        "still" => "Regarde l'écran et ne bouge plus",
        "yaw" => "Tourne la tête vers la GAUCHE",
        "back" => "Reviens en face de l'écran et ne bouge plus",
        "pitch" => "Lève la tête vers le HAUT",
        "roll" => "Penche la tête vers l'épaule DROITE",
        _ => "",
    }
}

fn point_text(p: &str) -> &'static str {
    match p {
        "center" => "Regarde le CENTRE de l'écran et tiens la position",
        "left" => "Regarde le bord GAUCHE de l'écran (ce que la caméra doit atteindre)",
        "right" => "Regarde le bord DROIT de l'écran",
        "top" => "Regarde le HAUT de l'écran",
        "bottom" => "Regarde le BAS de l'écran",
        _ => "",
    }
}

fn calibrate_device() -> Result<()> {
    dbus::call::<()>("DeviceCalibrationStart", &())?;
    let mut last = String::new();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let s: String = dbus::call("DeviceCalibrationState", &())?;
        let v: serde_json::Value = serde_json::from_str(&s)?;
        if v.is_null() {
            println!("\nCalibration terminée et enregistrée.");
            return Ok(());
        }
        let step = v["step"].as_str().unwrap_or("").to_string();
        let key = format!("{}{step}", v["index"]);
        if key != last {
            println!("\nÉtape {}/7 : {}", v["index"].as_u64().unwrap_or(0) + 1, step_text(&step));
            last = key;
        }
        let pct = (v["progress"].as_f64().unwrap_or(0.0) * 30.0) as usize;
        print!("\r[{:<30}]", "#".repeat(pct));
        use std::io::Write;
        std::io::stdout().flush().ok();
    }
}

fn calibrate_game(target: &str, side: f64, vert: f64) -> Result<()> {
    dbus::call::<()>("GameCalibrationStart", &(target))?;
    let mut last = String::new();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let s: String = dbus::call("GameCalibrationState", &())?;
        let v: serde_json::Value = serde_json::from_str(&s)?;
        let st = &v[1];
        if let Some(end) = st["end"].as_str() {
            if end == "done" {
                let msg: String = dbus::call("GameCalibrationApply", &(side, vert))?;
                println!("\n{msg}");
                return Ok(());
            }
            dbus::call::<()>("GameCalibrationCancel", &())?;
            bail!("calibration {end} ({})", st["hint"].as_str().unwrap_or(""));
        }
        let point = st["point"].as_str().unwrap_or("").to_string();
        let hint = st["hint"].as_str().map(|h| match h {
            "too_close" => " — trop près du centre, va plus loin",
            "wrong_side" => " — mauvais côté",
            _ => "",
        }).unwrap_or("");
        let key = format!("{point}{hint}");
        if key != last {
            println!("\n{}{hint}", point_text(&point));
            last = key;
        }
        let pct = (st["progress"].as_f64().unwrap_or(0.0) * 30.0) as usize;
        print!("\r[{:<30}]", "#".repeat(pct));
        use std::io::Write;
        std::io::stdout().flush().ok();
    }
}

fn valid_id(id: &str) -> Result<()> {
    if !podtrack_out::posefile::safe_id(id) {
        bail!("identifiant invalide (lettres, chiffres, - et _) : {id}");
    }
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).format_timestamp_secs().init();
    let cli = Cli::parse();
    let cfg_path = config::path();
    match cli.cmd {
        Cmd::Daemon { no_tray } => {
            let hub = hub::Hub::new(cfg_path)?;
            daemon::run(hub, !no_tray)?;
        }
        Cmd::Status { json } => {
            let v = status()?;
            if json { println!("{}", serde_json::to_string_pretty(&v)?) } else { print_status(&v) }
        }
        Cmd::Watch => loop {
            let v = status()?;
            let h = &v["head"];
            let r = &v["rates"];
            print!(
                "\rtête {:+7.1} {:+7.1} {:+7.1}°   vitesses {:+7.1} {:+7.1} {:+7.1}°/s   {:>5} Hz  biais {}   ",
                h[0].as_f64().unwrap_or(0.0), h[1].as_f64().unwrap_or(0.0), h[2].as_f64().unwrap_or(0.0),
                r[0].as_f64().unwrap_or(0.0), r[1].as_f64().unwrap_or(0.0), r[2].as_f64().unwrap_or(0.0),
                v["rate_hz"], if v["bias_ok"].as_bool() == Some(true) { "ok" } else { ".." }
            );
            use std::io::Write;
            std::io::stdout().flush().ok();
            std::thread::sleep(Duration::from_millis(100));
        },
        Cmd::Recenter => dbus::call::<()>("Recenter", &())?,
        Cmd::Start => dbus::call::<()>("Start", &())?,
        Cmd::Stop => dbus::call::<()>("Stop", &())?,
        Cmd::Toggle => {
            let on: bool = dbus::call("Toggle", &())?;
            println!("{}", if on { "suivi démarré" } else { "suivi arrêté" });
        }
        Cmd::Quit => dbus::call::<()>("Quit", &())?,
        Cmd::Tracker { game, tracker } => {
            let msg = match dbus::call::<String>("SetTracker", &(game.as_str(), tracker.as_str())) {
                Ok(m) => m,
                // sans démon, on agit directement sur la configuration et le préfixe
                Err(e) if format!("{e}").contains("ne tourne pas") || format!("{e}").contains("ServiceUnknown") => {
                    let hub = hub::Hub::new(cfg_path)?;
                    let mut s = hub.lock();
                    games::set_tracker(&mut s, &game, &tracker)?
                }
                Err(e) => return Err(e),
            };
            println!("{msg}");
        }
        Cmd::Game { cmd } => match cmd {
            GameCmd::List => {
                let c = config::load(&cfg_path)?;
                for (id, g) in &c.games {
                    let where_ = match &g.kind {
                        GameKind::Wine { prefix } => format!("Wine {}", prefix.display()),
                        GameKind::Steam { appid, flatpak } => format!("Steam {appid}{}", if *flatpak { " (Flatpak)" } else { "" }),
                    };
                    println!("{id:<16} {:<24} {where_:<40} traqueur {:?}{}", g.name, g.tracker, if g.enabled { "" } else { " (inactif)" });
                }
            }
            GameCmd::AddWine { id, name, prefix } => {
                valid_id(&id)?;
                let prefix = std::fs::canonicalize(config::expand(&prefix)).with_context(|| format!("préfixe {}", prefix.display()))?;
                if !prefix.join("user.reg").is_file() {
                    bail!("{} n'est pas un préfixe Wine (pas de user.reg)", prefix.display());
                }
                config::update(&cfg_path, |c| {
                    c.games.insert(id.clone(), Game { name, kind: GameKind::Wine { prefix }, enabled: false, tracker: Default::default(), profile: Default::default() });
                    Ok(())
                })?;
                println!("jeu {id} ajouté ; active-le avec : podtrack tracker {id} podtrack");
            }
            GameCmd::AddSteam { id, appid, flatpak } => {
                valid_id(&id)?;
                let c = config::load(&cfg_path)?;
                let name = steam::installed(&c.steam).get(&appid).map(|g| g.name.clone()).unwrap_or(format!("Steam {appid}"));
                config::update(&cfg_path, |c| {
                    c.games.insert(id.clone(), Game { name, kind: GameKind::Steam { appid, flatpak }, enabled: false, tracker: Default::default(), profile: Default::default() });
                    Ok(())
                })?;
                println!("jeu {id} ajouté ; active-le avec : podtrack tracker {id} podtrack");
            }
            GameCmd::Remove { id } => {
                config::update(&cfg_path, |c| {
                    c.games.remove(&id).with_context(|| format!("jeu inconnu : {id}"))?;
                    Ok(())
                })?;
            }
            GameCmd::Steam => {
                let c = config::load(&cfg_path)?;
                for g in steam::installed(&c.steam).values() {
                    println!("{:<10} {:<40} {}{}", g.appid, g.name, if g.prefix.is_some() { "Proton" } else { "natif / jamais lancé" }, if g.flatpak { " (Flatpak)" } else { "" });
                }
            }
        },
        Cmd::Calibrate { cmd } => match cmd {
            CalibCmd::Device => calibrate_device()?,
            CalibCmd::Game { target, target_side, target_vert } => calibrate_game(&target, target_side, target_vert)?,
        },
        Cmd::ImportMousepods { path, map } => {
            let map: Vec<(String, String)> = map
                .iter()
                .map(|m| m.split_once('=').map(|(a, b)| (a.to_string(), b.to_string())).context("--map attend ancien=nouveau"))
                .collect::<Result<_>>()?;
            let mut done = Vec::new();
            config::update(&cfg_path, |c| {
                done = import::run(c, &path, &map)?;
                Ok(())
            })?;
            for d in done {
                println!("{d}");
            }
        }
        Cmd::Config { cmd } => match cmd {
            ConfigCmd::Path => println!("{}", cfg_path.display()),
            ConfigCmd::Defaults => print!("{}", toml::to_string_pretty(&config::Config::default())?),
            ConfigCmd::Show => print!("{}", toml::to_string_pretty(&config::load(&cfg_path)?)?),
        },
    }
    Ok(())
}
