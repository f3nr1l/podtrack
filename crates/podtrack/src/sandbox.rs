//! Fonctionnement dans un Flatpak : commandes de l'hôte (`flatpak-spawn --host`), dossier
//! partagé avec l'hôte pour le fichier de pose, commande de lancement vue depuis l'hôte.

use std::path::PathBuf;
use std::process::Command;

pub fn flatpak_id() -> Option<String> {
    std::env::var("FLATPAK_ID").ok().filter(|s| !s.is_empty())
}

/// Commande exécutée sur l'hôte quand podtrack tourne dans un Flatpak.
pub fn host(prog: &str) -> Command {
    if flatpak_id().is_some() {
        let mut c = Command::new("flatpak-spawn");
        c.args(["--host", prog]);
        c
    } else {
        Command::new(prog)
    }
}

/// Dossier des fichiers de pose : `/dev/shm`, ou dans un Flatpak le dossier d'exécution de
/// l'application, visible de l'hôte (`/run/user/<uid>/app/<id>`).
pub fn default_shm_dir() -> PathBuf {
    match (flatpak_id(), std::env::var("XDG_RUNTIME_DIR")) {
        (Some(id), Ok(run)) => PathBuf::from(run).join("app").join(id),
        _ => PathBuf::from("/dev/shm"),
    }
}

/// Commande qui lance podtrack depuis l'hôte (raccourcis GNOME, démarrage automatique).
pub fn launcher() -> String {
    match flatpak_id() {
        Some(id) => format!("flatpak run --command=podtrack {id}"),
        None => std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "podtrack".into()),
    }
}

/// Un processus de l'hôte a-t-il cette variable d'environnement exacte ? (Flatpak : les
/// processus de l'hôte sont invisibles dans le bac à sable, on interroge l'hôte.)
pub fn host_env_match(patterns: &[String]) -> bool {
    // grep -z lit les environnements séparés par des zéros ; le shell de l'hôte développe le motif
    let quote = |t: &str| format!("'{}'", t.replace('\'', "'\\''"));
    let exprs: Vec<String> = patterns.iter().map(|p| format!("-e {}", quote(&format!("^{}$", regex_escape(p))))).collect();
    host("sh")
        .arg("-c")
        .arg(format!("grep -lsqz {} /proc/[0-9]*/environ", exprs.join(" ")))
        .status()
        .is_ok_and(|s| s.success())
}

fn regex_escape(s: &str) -> String {
    let mut o = String::new();
    for ch in s.chars() {
        if "\\.^$*+?()[]{}|".contains(ch) {
            o.push('\\');
        }
        o.push(ch);
    }
    o
}
