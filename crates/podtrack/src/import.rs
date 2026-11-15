//! Reprise de la configuration de MousePods (`config.json`) : profils d'appareil et profils de jeu.

use crate::config::{Config, Game, GameKind};
use anyhow::{Context, Result};
use podtrack_core::engine::DeviceProfile;
use podtrack_core::profile::{Axis, GameProfile, Neck};
use serde_json::Value;
use std::path::Path;

fn f(v: &Value, k: &str, d: f64) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn b(v: &Value, k: &str, d: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(d)
}

fn axis(v: Option<&Value>, name: &str) -> Axis {
    let mut a = Axis::default_for(name);
    let Some(v) = v else { return a };
    if v.get("edge_neg").is_none() && v.get("edge_pos").is_none() && ["gain", "limit", "curve"].iter().any(|k| v.get(k).is_some()) {
        let d = match name {
            "yaw" => (2.0, 1.4, 180.0),
            "pitch" => (1.5, 1.4, 90.0),
            _ => (1.0, 1.0, 90.0),
        };
        return Axis::from_legacy(f(v, "gain", d.0), f(v, "curve", d.1), f(v, "limit", d.2), b(v, "enabled", true), b(v, "invert", false));
    }
    a.enabled = b(v, "enabled", a.enabled);
    a.invert = b(v, "invert", a.invert);
    a.edge_neg = f(v, "edge_neg", a.edge_neg);
    a.edge_pos = f(v, "edge_pos", a.edge_pos);
    a.target = f(v, "target", a.target);
    a.curve = f(v, "curve", a.curve);
    a
}

pub fn game_profile(v: &Value) -> GameProfile {
    let d = GameProfile::default();
    let neck = v.get("neck").cloned().unwrap_or(Value::Null);
    let axes = v.get("axes");
    let mut p = GameProfile {
        smoothing: f(v, "smoothing", d.smoothing),
        neck: Neck { enabled: b(&neck, "enabled", false), strength: f(&neck, "strength", 1.0), lean_forward: b(&neck, "lean_forward", false), ..Default::default() },
        own: v.get("own").and_then(Value::as_bool).unwrap_or(false),
        ..d
    };
    p.axes.yaw = axis(axes.and_then(|a| a.get("yaw")), "yaw");
    p.axes.pitch = axis(axes.and_then(|a| a.get("pitch")), "pitch");
    p.axes.roll = axis(axes.and_then(|a| a.get("roll")), "roll");
    if v.get("own").is_none() {
        p.own = p.axes != GameProfile::default().axes;
    }
    p.normalized()
}

/// `map` : identifiant MousePods → identifiant podtrack (jeux hors Steam).
pub fn run(cfg: &mut Config, path: &Path, map: &[(String, String)]) -> Result<Vec<String>> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).with_context(|| format!("lecture de {}", path.display()))?)?;
    let mut done = Vec::new();
    if let Some(profiles) = v.get("profiles").and_then(Value::as_object) {
        for (k, p) in profiles {
            if let Some(lat) = p.get("lat").and_then(Value::as_array).filter(|l| l.len() == 3) {
                let lat = [0, 1, 2].map(|i| lat[i].as_f64().unwrap_or(0.0));
                cfg.devices.insert(k.clone(), DeviceProfile { yaw_sign: f(p, "yaw_sign", 1.0), lat, pitch_sign: f(p, "pitch_sign", 1.0), roll_sign: f(p, "roll_sign", 1.0) });
                done.push(format!("profil d'appareil {k}"));
            }
        }
    }
    if let Some(games) = v.get("games").and_then(Value::as_object) {
        for (appid, g) in games {
            let profile = game_profile(g.get("profile").unwrap_or(&Value::Null));
            let target = map.iter().find(|(from, _)| from == appid).map(|(_, to)| to.clone());
            if let Some(id) = target {
                match cfg.games.get_mut(&id) {
                    Some(game) => {
                        game.profile = profile;
                        done.push(format!("profil de jeu {appid} → {id}"));
                    }
                    None => done.push(format!("jeu {id} absent de la configuration : l'ajouter d'abord")),
                }
                continue;
            }
            let id = format!("steam-{appid}");
            let flatpak = b(g, "flatpak", false);
            cfg.games.entry(id.clone()).or_insert(Game {
                name: format!("Steam {appid}"),
                kind: GameKind::Steam { appid: appid.clone(), flatpak },
                enabled: false,
                tracker: Default::default(),
                profile: profile.clone(),
            });
            done.push(format!("jeu Steam {appid} → {id}"));
        }
    }
    Ok(done)
}
