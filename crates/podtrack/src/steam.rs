//! Steam : bibliothèques, jeux installés, préfixes Proton (natif et Flatpak).
//! Repris de `steam_games.py` (MousePods), sans les options de lancement.

use crate::config::{SteamConfig, expand};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Lecteur minimal du format KeyValues de Valve : chaînes entre guillemets, accolades, `//`.
#[derive(Debug, Clone)]
pub enum Vdf {
    Str(String),
    Map(Vec<(String, Vdf)>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Map(m) => m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            _ => None,
        }
    }
}

enum Tok {
    S(String),
    Open,
    Close,
}

pub fn parse_vdf(text: &str) -> Vdf {
    let mut toks = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut it = line.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '"' => {
                    let mut s = String::new();
                    while let Some(c) = it.next() {
                        match c {
                            '\\' => {
                                if let Some(n) = it.next() {
                                    s.push(n);
                                }
                            }
                            '"' => break,
                            _ => s.push(c),
                        }
                    }
                    toks.push(Tok::S(s));
                }
                '{' => toks.push(Tok::Open),
                '}' => toks.push(Tok::Close),
                _ => {}
            }
        }
    }
    fn block(t: &[Tok], pos: &mut usize) -> Vdf {
        let mut out = Vec::new();
        while *pos < t.len() {
            let key = match &t[*pos] {
                Tok::Close => {
                    *pos += 1;
                    return Vdf::Map(out);
                }
                Tok::Open => {
                    *pos += 1;
                    continue;
                }
                Tok::S(s) => s.clone(),
            };
            *pos += 1;
            match t.get(*pos) {
                Some(Tok::Open) => {
                    *pos += 1;
                    out.push((key, block(t, pos)));
                }
                Some(Tok::S(v)) => {
                    out.push((key, Vdf::Str(v.clone())));
                    *pos += 1;
                }
                _ => {}
            }
        }
        Vdf::Map(out)
    }
    let mut pos = 0;
    block(&toks, &mut pos)
}

const TOOL_APPIDS: [&str; 10] = ["228980", "1070560", "1391110", "1628350", "1493710", "2180100", "2348590", "1887720", "2805730", "3658110"];

pub fn is_tool(appid: &str, name: &str) -> bool {
    TOOL_APPIDS.contains(&appid)
        || name.starts_with("Proton")
        || name.starts_with("Steam Linux Runtime")
        || name == "Steamworks Common Redistributables"
}

#[derive(Debug, Clone)]
pub struct SteamGame {
    pub appid: String,
    pub name: String,
    pub flatpak: bool,
    pub prefix: Option<PathBuf>,
}

fn roots(cfg: &SteamConfig) -> Vec<(PathBuf, bool)> {
    let mut out: Vec<(PathBuf, bool)> = Vec::new();
    let all = cfg.roots.iter().map(|p| (p, false)).chain(cfg.flatpak_roots.iter().map(|p| (p, true)));
    for (p, flatpak) in all {
        let p = expand(p);
        let real = std::fs::canonicalize(&p).unwrap_or(p);
        if real.join("steamapps/libraryfolders.vdf").is_file() && !out.iter().any(|(q, _)| *q == real) {
            out.push((real, flatpak));
        }
    }
    out
}

fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![root.to_path_buf()];
    let text = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
    if let Some(Vdf::Map(entries)) = parse_vdf(&text).get("libraryfolders") {
        for (_, e) in entries {
            if let Some(p) = e.get("path").and_then(|v| v.str()) {
                let p = std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
                if !libs.contains(&p) {
                    libs.push(p);
                }
            }
        }
    }
    libs
}

/// Jeux installés (outils exclus), par appid.
pub fn installed(cfg: &SteamConfig) -> BTreeMap<String, SteamGame> {
    let mut out = BTreeMap::new();
    for (root, flatpak) in roots(cfg) {
        for lib in libraries(&root) {
            let Ok(rd) = std::fs::read_dir(lib.join("steamapps")) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                    continue;
                }
                let text = std::fs::read_to_string(e.path()).unwrap_or_default();
                let vdf = parse_vdf(&text);
                let Some(state) = vdf.get("AppState") else { continue };
                let appid = state.get("appid").and_then(|v| v.str()).unwrap_or_default().to_string();
                let gname = state.get("name").and_then(|v| v.str()).unwrap_or_default().to_string();
                if appid.is_empty() || is_tool(&appid, &gname) {
                    continue;
                }
                let pfx = lib.join("steamapps/compatdata").join(&appid).join("pfx");
                let prefix = pfx.join("user.reg").is_file().then_some(pfx);
                out.entry(appid.clone()).or_insert(SteamGame { appid, name: gname, flatpak, prefix });
            }
        }
    }
    out
}

pub fn proton_prefix(cfg: &SteamConfig, appid: &str, flatpak: bool) -> Option<PathBuf> {
    installed(cfg).get(appid).filter(|g| g.flatpak == flatpak || g.prefix.is_some()).and_then(|g| g.prefix.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn vdf() {
        let v = super::parse_vdf("\"libraryfolders\"\n{\n // c\n \"0\" { \"path\" \"/a\\\\b\" }\n}\n");
        let p = v.get("LibraryFolders").unwrap().get("0").unwrap().get("path").unwrap().str().unwrap();
        assert_eq!(p, "/a\\b");
    }
}
