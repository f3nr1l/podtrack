//! Jeux : bascule du traqueur (podtrack ou un autre) dans le préfixe Wine, calibration de
//! l'amplitude. Le registre n'est touché que jeu fermé.

use crate::config::{GameKind, TrackerChoice};
use crate::hub::State;
use anyhow::{Context, Result, bail};
use podtrack_core::calibration::End;
use podtrack_out::procs;
use podtrack_out::tracklist::{self, TrackTable};
use podtrack_out::winereg::{Prefix, Status};
use std::path::PathBuf;

/// Dossier de MousePods (installation antérieure à podtrack), repris pour la migration.
const LEGACY_DIR: &str = "mousepods";

fn dll_source(s: &State) -> Result<PathBuf> {
    if let Some(d) = &s.cfg.bridge.dll_dir {
        return Ok(crate::config::expand(d));
    }
    let dir = dirs::data_dir().context("pas de dossier de données")?.join("podtrack").join("dll");
    podtrack_out::bundled_dll::extract(&dir)?;
    Ok(dir)
}

pub fn game_running(s: &State, id: &str) -> Result<bool> {
    let g = s.cfg.games.get(id).with_context(|| format!("jeu inconnu : {id}"))?;
    // dans un Flatpak, les processus de l'hôte ne sont visibles qu'en interrogeant l'hôte
    if crate::sandbox::flatpak_id().is_some() {
        return Ok(match &g.kind {
            GameKind::Wine { prefix } => crate::sandbox::host_env_match(&[format!("WINEPREFIX={}", crate::config::expand(prefix).display())]),
            GameKind::Steam { appid, .. } => crate::sandbox::host_env_match(&[format!("SteamAppId={appid}"), format!("STEAM_COMPAT_APP_ID={appid}")]),
        });
    }
    Ok(match &g.kind {
        GameKind::Wine { prefix } => procs::prefix_in_use(&crate::config::expand(prefix)),
        GameKind::Steam { appid, .. } => procs::steam_app_running(appid),
    })
}


pub fn set_tracker(s: &mut State, id: &str, tracker: &str) -> Result<String> {
    let choice = match tracker {
        "podtrack" | "airpods" => TrackerChoice::Podtrack,
        "other" | "none" => TrackerChoice::Other,
        t => bail!("traqueur inconnu : {t} (podtrack ou other)"),
    };
    let game = s.cfg.games.get(id).cloned().with_context(|| format!("jeu inconnu : {id}"))?;
    let root = game.prefix(&s.cfg.steam).with_context(|| format!("préfixe de {} introuvable", game.name))?;
    let pfx = Prefix::new(&root, &s.cfg.bridge.dir_name);
    let legacy = Prefix::new(&root, LEGACY_DIR);
    let current = pfx.status()?;
    let already = match choice {
        TrackerChoice::Podtrack => current == Status::On && !legacy.has_our_files(),
        TrackerChoice::Other => !pfx.has_our_files() && !legacy.has_our_files(),
    };
    if !already {
        if game_running(s, id)? {
            bail!("{} est encore lancé (ou son wineserver) : fermer le jeu, attendre quelques secondes, puis recommencer", game.name);
        }
        let snap = pfx.snapshot()?;
        log::info!("sauvegarde du registre : {}", snap.display());
        if legacy.has_our_files() {
            legacy.uninstall().context("retrait de l'installation MousePods")?;
            log::info!("{} : installation MousePods retirée", game.name);
        }
        match choice {
            TrackerChoice::Podtrack => {
                let games = tracklist::load(s.cfg.bridge.games_csv.as_deref().map(crate::config::expand).as_deref());
                let table = TrackTable::from_games(&games);
                let pose = s.pose_path(id)?;
                pfx.install(&dll_source(s)?, &pose, &table, s.cfg.bridge.dll_debug)?;
            }
            TrackerChoice::Other => {
                if pfx.has_our_files() {
                    pfx.uninstall()?;
                }
            }
        }
    }
    let g = s.cfg.games.get_mut(id).unwrap();
    g.tracker = choice;
    g.enabled = choice == TrackerChoice::Podtrack;
    s.save_config()?;
    let after = pfx.status()?;
    Ok(match after {
        Status::On => format!("{} : suivi par podtrack", game.name),
        Status::Off => format!("{} : aucun traqueur", game.name),
        Status::Foreign(n) => format!("{} : traqueur {n}", game.name),
    })
}

/// Réglages remplacés par la dernière calibration appliquée, pour pouvoir y revenir.
pub enum CalibUndo {
    Mouse(Option<podtrack_core::mouse::AimRange>),
    Baseline(Option<podtrack_core::profile::Baseline>),
    Game(String, Box<podtrack_core::profile::GameProfile>),
}

/// Résultat d'une calibration terminée, sans la clore.
fn calibration_result(s: &State, target_side: f64, target_vert: f64) -> Result<(String, podtrack_core::profile::Baseline)> {
    let Some((target, cal)) = s.game_calib.as_ref() else { bail!("aucune calibration en cours") };
    let st = cal.state();
    if st.end != Some(End::Done) {
        bail!("calibration non terminée");
    }
    let mut b = st.result.context("calibration sans résultat")?;
    b.target_side = target_side;
    b.target_vert = target_vert;
    Ok((target.clone(), b))
}

fn fmt_edges(neg: f64, pos: f64) -> String {
    format!("{neg:.1}° / {pos:.1}°").replace('.', ",")
}

/// Décrit ce que changerait la calibration terminée, avant de l'appliquer.
pub fn game_calibration_preview(s: &State, target_side: f64, target_vert: f64) -> Result<String> {
    let (target, b) = calibration_result(s, target_side, target_vert)?;
    let line = |name: &str, old: Option<(f64, f64)>, new: (f64, f64)| match old {
        Some((on, op)) => {
            let ratio = (on + op) / (new.0 + new.1).max(0.1);
            let effet = if ratio > 1.15 {
                format!(", caméra {ratio:.1} fois plus sensible").replace('.', ",")
            } else if ratio < 0.87 {
                format!(", caméra {:.1} fois moins sensible", 1.0 / ratio).replace('.', ",")
            } else {
                String::new()
            };
            format!("{name} : {} → {}{effet}", fmt_edges(on, op), fmt_edges(new.0, new.1))
        }
        None => format!("{name} : {}", fmt_edges(new.0, new.1)),
    };
    let (yaw, pitch) = ((b.yaw.neg, b.yaw.pos), (b.pitch.neg, b.pitch.pos));
    Ok(if target == "mouse" {
        let old = s.cfg.mouse.pointeur.range;
        format!(
            "Pointage absolu, angle de tête aux bords de l'écran.\n{}\n{}",
            line("Horizontal", old.map(|r| (r.x_neg, r.x_pos)), yaw),
            line("Vertical", old.map(|r| (r.y_neg, r.y_pos)), pitch)
        )
    } else if target == "baseline" {
        let old = s.cfg.baseline.as_ref();
        format!(
            "Référence d'amplitude, angle de tête aux bords de l'écran.\n{}\n{}",
            line("Lacet", old.map(|o| (o.yaw.neg, o.yaw.pos)), yaw),
            line("Tangage", old.map(|o| (o.pitch.neg, o.pitch.pos)), pitch)
        )
    } else {
        let g = s.cfg.games.get(&target).context("jeu disparu")?;
        let a = &g.profile.axes;
        format!(
            "{} : angle de tête qui amène la caméra à {target_side:.0}° (lacet) et {target_vert:.0}° (tangage).\n{}\n{}",
            g.name,
            line("Lacet", Some((a.yaw.edge_neg, a.yaw.edge_pos)), yaw),
            line("Tangage", Some((a.pitch.edge_neg, a.pitch.edge_pos)), pitch)
        )
    })
}

/// Enregistre le résultat de la calibration par points (baseline, pointeur ou amplitude d'un
/// jeu) et garde les réglages remplacés pour `undo_game_calibration`.
pub fn apply_game_calibration(s: &mut State, target_side: f64, target_vert: f64) -> Result<String> {
    let (target, b) = calibration_result(s, target_side, target_vert)?;
    s.game_calib = None;
    s.tracker_suspend_autocenter(false);
    let (undo, msg) = if target == "mouse" {
        let old = s.cfg.mouse.pointeur.range.replace(podtrack_core::mouse::AimRange { x_neg: b.yaw.neg, x_pos: b.yaw.pos, y_pos: b.pitch.pos, y_neg: b.pitch.neg });
        (CalibUndo::Mouse(old), format!("Pointeur : {} × {}", fmt_edges(b.yaw.neg, b.yaw.pos), fmt_edges(b.pitch.neg, b.pitch.pos)))
    } else if target == "baseline" {
        let old = s.cfg.baseline.replace(b.clone());
        (CalibUndo::Baseline(old), format!("Référence : lacet {}, tangage {}", fmt_edges(b.yaw.neg, b.yaw.pos), fmt_edges(b.pitch.neg, b.pitch.pos)))
    } else {
        let g = s.cfg.games.get_mut(&target).context("jeu disparu")?;
        let old = Box::new(g.profile.clone());
        let a = &mut g.profile.axes;
        a.yaw.edge_neg = b.yaw.neg;
        a.yaw.edge_pos = b.yaw.pos;
        a.yaw.target = target_side;
        a.pitch.edge_neg = b.pitch.neg;
        a.pitch.edge_pos = b.pitch.pos;
        a.pitch.target = target_vert;
        g.profile.own = true;
        let msg = format!("{} : lacet {}, tangage {}", g.name, fmt_edges(b.yaw.neg, b.yaw.pos), fmt_edges(b.pitch.neg, b.pitch.pos));
        (CalibUndo::Game(target.clone(), old), msg)
    };
    s.save_config()?;
    s.calib_undo = Some(undo);
    Ok(msg)
}

/// Rétablit les réglages remplacés par la dernière calibration appliquée.
pub fn undo_game_calibration(s: &mut State) -> Result<String> {
    let msg = match s.calib_undo.take().context("aucune calibration à annuler")? {
        CalibUndo::Mouse(r) => {
            s.cfg.mouse.pointeur.range = r;
            "Pointeur : réglage précédent rétabli".to_string()
        }
        CalibUndo::Baseline(b) => {
            s.cfg.baseline = b;
            "Référence précédente rétablie".to_string()
        }
        CalibUndo::Game(id, p) => {
            let g = s.cfg.games.get_mut(&id).context("jeu disparu")?;
            g.profile = *p;
            format!("{} : profil précédent rétabli", g.name)
        }
    };
    s.save_config()?;
    Ok(msg)
}
