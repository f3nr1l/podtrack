//! Raccourcis clavier globaux par les raccourcis personnalisés de GNOME (gsettings) : sous
//! Wayland, une application ne peut pas capter une touche globale elle-même.

use crate::config::Hotkeys;
use anyhow::{Context, Result};
use std::process::Command;

const SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const BASE: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";
/// Anciens raccourcis (MousePods, sc-headtracker) remplacés par ceux de podtrack.
const LEGACY: [&str; 1] = ["airpods-mouse-recenter"];

fn gs(args: &[&str]) -> Result<String> {
    let out = Command::new("gsettings").args(args).output().context("gsettings introuvable")?;
    if !out.status.success() {
        anyhow::bail!("gsettings {} : {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn list() -> Result<Vec<String>> {
    let cur = gs(&["get", SCHEMA, "custom-keybindings"])?;
    let inner = cur.trim_start_matches("@as").trim().trim_start_matches('[').trim_end_matches(']');
    Ok(inner.split(',').map(|s| s.trim().trim_matches('\'').to_string()).filter(|s| !s.is_empty()).collect())
}

fn set_list(paths: &[String]) -> Result<()> {
    let v = format!("[{}]", paths.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(", "));
    gs(&["set", SCHEMA, "custom-keybindings", &v]).map(|_| ())
}

fn exe() -> String {
    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "podtrack".into())
}

/// Écrit (ou retire si vide) les raccourcis de podtrack.
pub fn apply(h: &Hotkeys) -> Result<()> {
    let mut paths = list()?;
    paths.retain(|p| !LEGACY.iter().any(|l| p.ends_with(&format!("/{l}/"))));
    for (ident, name, binding, cmd) in [
        ("podtrack-recenter", "Recentrer le suivi de tête (podtrack)", &h.recenter, "recenter"),
        ("podtrack-toggle", "Démarrer / arrêter le suivi de tête (podtrack)", &h.toggle, "toggle"),
    ] {
        let path = format!("{BASE}{ident}/");
        if binding.is_empty() {
            paths.retain(|p| *p != path);
            continue;
        }
        let sch = format!("{SCHEMA}.custom-keybinding:{path}");
        gs(&["set", &sch, "name", name])?;
        gs(&["set", &sch, "binding", binding])?;
        gs(&["set", &sch, "command", &format!("{} {cmd}", exe())])?;
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    set_list(&paths)
}

/// Raccourci déjà pris par un autre raccourci personnalisé ? (nom de celui-ci)
pub fn conflict(binding: &str) -> Option<String> {
    let paths = list().ok()?;
    for p in paths {
        if p.contains("/podtrack-") {
            continue;
        }
        let sch = format!("{SCHEMA}.custom-keybinding:{p}");
        let b = gs(&["get", &sch, "binding"]).ok()?;
        if b.trim_matches('\'').eq_ignore_ascii_case(binding) {
            return gs(&["get", &sch, "name"]).ok().map(|n| n.trim_matches('\'').to_string());
        }
    }
    None
}
