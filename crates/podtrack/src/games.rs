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
    Ok(match &g.kind {
        GameKind::Wine { prefix } => procs::prefix_in_use(&crate::config::expand(prefix)),
        GameKind::Steam { appid, .. } => procs::steam_app_running(appid),
    })
}


pub fn set_tracker(s: &mut State, id: &str, tracker: &str) -> Result<String> {
    let choice = match tracker {
        "podtrack" | "airpods" => TrackerChoice::Podtrack,
        "other" | "beam" | "none" => TrackerChoice::Other,
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
            bail!("{} tourne encore (ou son wineserver) : ferme-le, attends quelques secondes, recommence", game.name);
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

/// Enregistre le résultat de la calibration par points : baseline, ou amplitude d'un jeu.
pub fn apply_game_calibration(s: &mut State, target_side: f64, target_vert: f64) -> Result<String> {
    let Some((target, cal)) = s.game_calib.take() else { bail!("aucune calibration en cours") };
    s.tracker_suspend_autocenter(false);
    let st = cal.state();
    if st.end != Some(End::Done) {
        s.game_calib = Some((target, cal));
        bail!("calibration non terminée");
    }
    let mut b = st.result.unwrap();
    b.target_side = target_side;
    b.target_vert = target_vert;
    let msg = if target == "mouse" {
        s.cfg.mouse.pointeur.range = Some(podtrack_core::mouse::AimRange { x_neg: b.yaw.neg, x_pos: b.yaw.pos, y_pos: b.pitch.pos, y_neg: b.pitch.neg });
        format!("souris (pointeur) : {:.1}/{:.1}° × {:.1}/{:.1}°", b.yaw.neg, b.yaw.pos, b.pitch.neg, b.pitch.pos)
    } else if target == "baseline" {
        s.cfg.baseline = Some(b.clone());
        format!("référence : lacet {:.1}/{:.1}°, tangage {:.1}/{:.1}°", b.yaw.neg, b.yaw.pos, b.pitch.neg, b.pitch.pos)
    } else {
        let g = s.cfg.games.get_mut(&target).context("jeu disparu")?;
        let a = &mut g.profile.axes;
        a.yaw.edge_neg = b.yaw.neg;
        a.yaw.edge_pos = b.yaw.pos;
        a.yaw.target = target_side;
        a.pitch.edge_neg = b.pitch.neg;
        a.pitch.edge_pos = b.pitch.pos;
        a.pitch.target = target_vert;
        g.profile.own = true;
        format!("{} : lacet {:.1}/{:.1}°, tangage {:.1}/{:.1}°", g.name, b.yaw.neg, b.yaw.pos, b.pitch.neg, b.pitch.pos)
    };
    s.save_config()?;
    Ok(msg)
}
