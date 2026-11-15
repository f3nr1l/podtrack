//! Détection d'un jeu en cours : un processus dont l'environnement porte le préfixe Wine
//! (`WINEPREFIX`) ou l'appid Steam (`SteamAppId`, `STEAM_COMPAT_APP_ID`, ou `AppId=` en argument).

use std::fs;
use std::path::Path;

fn env_of(pid: &str) -> Option<Vec<u8>> {
    fs::read(format!("/proc/{pid}/environ")).ok()
}

fn pids() -> impl Iterator<Item = String> {
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.bytes().all(|c| c.is_ascii_digit()))
}

fn same_path(a: &[u8], b: &Path) -> bool {
    let a = Path::new(std::str::from_utf8(a).unwrap_or(""));
    let ca = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// Un processus tourne-t-il dans ce préfixe Wine (wineserver compris) ?
pub fn prefix_in_use(prefix: &Path) -> bool {
    let me = std::process::id().to_string();
    pids().filter(|p| *p != me).any(|pid| {
        env_of(&pid).is_some_and(|env| {
            env.split(|&c| c == 0).any(|kv| kv.strip_prefix(b"WINEPREFIX=").is_some_and(|v| same_path(v, prefix)))
        })
    })
}

/// Le jeu Steam `appid` tourne-t-il ?
pub fn steam_app_running(appid: &str) -> bool {
    let keys: [&[u8]; 2] = [b"SteamAppId=", b"STEAM_COMPAT_APP_ID="];
    pids().any(|pid| {
        let env_hit = env_of(&pid).is_some_and(|env| {
            env.split(|&c| c == 0).any(|kv| keys.iter().any(|k| kv.strip_prefix(*k) == Some(appid.as_bytes())))
        });
        env_hit
            || fs::read(format!("/proc/{pid}/cmdline"))
                .is_ok_and(|c| c.split(|&b| b == 0).any(|a| a.strip_prefix(b"AppId=") == Some(appid.as_bytes())))
    })
}
