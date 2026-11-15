//! Registre d'un préfixe Wine (`user.reg`, texte) : installation et retrait des DLL de suivi de
//! tête, sauvegarde et restauration exacte, table TrackIR. Repris de `bridge_install.py`
//! (MousePods), étendu aux préfixes hors Steam et à la table des jeux « Enhanced ».
//!
//! Le registre n'est modifié que jeu fermé : Wine garde le registre en mémoire et réécrit
//! `user.reg` en quittant.

use crate::tracklist::TrackTable;
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const KEY_NPCLIENT: &str = r"Software\\NaturalPoint\\NATURALPOINT\\NPClient Location";
pub const KEY_FREETRACK: &str = r"Software\\Freetrack\\FreeTrackClient";
/// Clé lue par les DLL (nom hérité de MousePods, que les DLL attendent).
pub const KEY_BRIDGE: &str = r"Software\\MousePods";
pub const KEY_TABLE: &str = r"Software\\MousePods\\Table";
pub const DLLS: [&str; 4] = ["NPClient.dll", "NPClient64.dll", "freetrackclient.dll", "freetrackclient64.dll"];

/// Traqueurs connus, reconnus au chemin de leur DLL.
pub const KNOWN_TRACKERS: [(&str, &str); 8] = [
    ("beam eye tracker", "Beam Eye Tracker"),
    ("opentrack", "opentrack"),
    ("lookpilot", "LookPilot"),
    ("aitrack", "AITrack"),
    ("smoothtrack", "SmoothTrack"),
    ("mousepods", "MousePods"),
    ("naturalpoint", "TrackIR"),
    ("trackir", "TrackIR"),
];

/// Échappe les barres obliques inverses comme dans `user.reg`.
pub fn reg_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
}

/// Une section `[nom] horodatage` et sa plage d'octets dans le texte.
fn sections(text: &str) -> Vec<(String, usize, usize)> {
    let mut marks: Vec<(String, usize)> = Vec::new();
    let mut off = 0;
    for line in text.split_inclusive('\n') {
        if line.ends_with('\n') && line.starts_with('[') {
            let body = &line[1..line.len() - 1];
            if let Some(close) = body.rfind(']') {
                let rest = &body[close + 1..];
                let ok = rest.is_empty() || (rest.starts_with(' ') && rest.len() > 1 && rest[1..].bytes().all(|c| c.is_ascii_digit()));
                if ok && close > 0 {
                    marks.push((body[..close].to_lowercase(), off));
                }
            }
        }
        off += line.len();
    }
    let mut out = Vec::new();
    for (i, (name, start)) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map(|m| m.1).unwrap_or(text.len());
        out.push((name.clone(), *start, end));
    }
    out
}

fn span(text: &str, section: &str) -> Option<(usize, usize)> {
    let key = section.to_lowercase();
    // la dernière occurrence gagne, comme un dict Python construit dans l'ordre
    sections(text).into_iter().filter(|s| s.0 == key).last().map(|s| (s.1, s.2))
}

/// Valeur chaîne `"nom"="valeur"` d'une section, telle qu'écrite (échappements compris).
pub fn value(text: &str, section: &str, name: &str) -> Option<String> {
    let (a, b) = span(text, section)?;
    let prefix = format!("\"{name}\"=\"");
    for line in text[a..b].lines() {
        if let Some(rest) = line.strip_prefix(&prefix) {
            if let Some(v) = rest.strip_suffix('"') {
                return Some(v.to_string());
            }
        }
    }
    None
}

pub fn set_section(text: &str, section: &str, lines: &[String], stamp: u64) -> String {
    let block = format!("[{section}] {stamp}\n{}\n\n", lines.join("\n"));
    match span(text, section) {
        Some((a, b)) => format!("{}{}{}", &text[..a], block, &text[b..]),
        None => format!("{}\n\n{}", text.trim_end_matches('\n'), block),
    }
}

pub fn drop_section(text: &str, section: &str) -> String {
    match span(text, section) {
        Some((a, b)) => format!("{}{}", &text[..a], &text[b..]),
        None => text.to_string(),
    }
}

fn section_text<'a>(text: &'a str, section: &str) -> Option<&'a str> {
    span(text, section).map(|(a, b)| &text[a..b])
}

fn now_stamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn read_reg(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("lecture de {}", path.display()))?;
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{} n'est pas en UTF-8 : modification refusée", path.display()))
}

/// Remplace `user.reg` de façon atomique (fichier temporaire, fsync, renommage) : un disque plein
/// ou une coupure laisse l'ancien registre intact.
pub fn write_reg(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().context("registre sans dossier")?;
    let tmp = dir.join(format!(".user.reg.podtrack-{}", std::process::id()));
    let res = (|| -> Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        if let Ok(m) = fs::metadata(path) {
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(m.permissions().mode() & 0o777));
        }
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Un préfixe Wine et l'emplacement de nos DLL dedans.
#[derive(Debug, Clone)]
pub struct Prefix {
    pub root: PathBuf,
    /// Dossier des DLL sous `drive_c` (par défaut `podtrack`).
    pub dir_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Aucune clé de suivi de tête.
    Off,
    /// Nos DLL.
    On,
    /// Un autre traqueur (nom reconnu, ou « ? »).
    Foreign(String),
}

impl Prefix {
    pub fn new(root: impl Into<PathBuf>, dir_name: &str) -> Self {
        Self { root: root.into(), dir_name: dir_name.to_string() }
    }

    pub fn user_reg(&self) -> PathBuf {
        self.root.join("user.reg")
    }

    pub fn dll_dir(&self) -> PathBuf {
        self.root.join("drive_c").join(&self.dir_name)
    }

    fn backup(&self) -> PathBuf {
        self.dll_dir().join(format!("user.reg.before-{}", self.dir_name))
    }

    /// Chemin Windows de nos DLL, tel qu'écrit dans `user.reg`.
    pub fn our_path(&self) -> String {
        reg_escape(&format!("C:\\{}\\", self.dir_name))
    }

    pub fn exists(&self) -> bool {
        self.user_reg().is_file()
    }

    pub fn status(&self) -> Result<Status> {
        if !self.exists() {
            return Ok(Status::Off);
        }
        let text = read_reg(&self.user_reg())?;
        Ok(match value(&text, KEY_NPCLIENT, "Path") {
            None => Status::Off,
            Some(p) if p == self.our_path() => Status::On,
            Some(p) => {
                let low = p.to_lowercase();
                Status::Foreign(KNOWN_TRACKERS.iter().find(|(n, _)| low.contains(n)).map(|t| t.1).unwrap_or("?").to_string())
            }
        })
    }

    /// Copie datée de `user.reg` à côté de lui, avant une modification.
    pub fn snapshot(&self) -> Result<PathBuf> {
        let stamp = chrono_like_stamp();
        let dst = self.root.join(format!("user.reg.podtrack-{stamp}"));
        fs::copy(self.user_reg(), &dst).context("copie de sauvegarde de user.reg")?;
        Ok(dst)
    }

    /// Installe nos DLL et pointe les clés TrackIR / FreeTrack vers elles. `pose_file` : chemin
    /// Linux du fichier de pose ; `tables` : tables TrackIR à écrire ; `debug` : journal des DLL.
    pub fn install(&self, dll_src: &Path, pose_file: &Path, tables: &TrackTable, debug: bool) -> Result<()> {
        if !self.exists() {
            bail!("préfixe introuvable : {}", self.root.display());
        }
        for n in DLLS {
            if !dll_src.join(n).is_file() {
                bail!("DLL manquante : {}", dll_src.join(n).display());
            }
        }
        let dest = self.dll_dir();
        fs::create_dir_all(&dest)?;
        let backup = self.backup();
        if !backup.exists() {
            fs::copy(self.user_reg(), &backup).context("sauvegarde du registre d'origine")?;
        }
        for n in DLLS {
            fs::copy(dll_src.join(n), dest.join(n))?;
        }
        let mut text = read_reg(&self.user_reg())?;
        let stamp = now_stamp();
        let ours = format!("\"Path\"=\"{}\"", self.our_path());
        for key in [KEY_NPCLIENT, KEY_FREETRACK] {
            text = set_section(&text, key, std::slice::from_ref(&ours), stamp);
        }
        let win_pose = format!("Z:{}", pose_file.display().to_string().replace('/', "\\"));
        let mut bridge = vec![format!("\"PoseFile\"=\"{}\"", reg_escape(&win_pose))];
        if debug {
            bridge.push("\"Debug\"=dword:00000001".into());
        }
        text = set_section(&text, KEY_BRIDGE, &bridge, stamp);
        let lines = tables.reg_lines();
        text = if lines.is_empty() { drop_section(&text, KEY_TABLE) } else { set_section(&text, KEY_TABLE, &lines, stamp) };
        write_reg(&self.user_reg(), &text)
    }

    /// Retire nos DLL et remet les clés telles qu'avant l'installation. Une clé reprise entre-temps
    /// par un autre traqueur est laissée en place.
    pub fn uninstall(&self) -> Result<()> {
        let backup = self.backup();
        if !backup.exists() {
            bail!("pas de sauvegarde {} : rien à restaurer", backup.display());
        }
        let before = read_reg(&backup)?;
        let mut text = read_reg(&self.user_reg())?;
        for key in [KEY_NPCLIENT, KEY_FREETRACK, KEY_BRIDGE, KEY_TABLE] {
            let is_path_key = key == KEY_NPCLIENT || key == KEY_FREETRACK;
            if is_path_key {
                if let Some(cur) = value(&text, key, "Path") {
                    if cur != self.our_path() {
                        continue; // un autre traqueur a repris la clé
                    }
                }
            }
            text = drop_section(&text, key);
            if let Some(old) = section_text(&before, key) {
                text = format!("{}\n\n{}", text.trim_end_matches('\n'), old);
            }
        }
        write_reg(&self.user_reg(), &text)?;
        fs::remove_dir_all(self.dll_dir())?;
        Ok(())
    }

    /// Nos fichiers sont dans le préfixe (sauvegarde comprise), quoi que disent les clés.
    pub fn has_our_files(&self) -> bool {
        self.backup().is_file()
    }
}

fn chrono_like_stamp() -> String {
    // AAAAMMJJ-HHMMSS en heure locale, sans dépendance
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracklist::TrackTable;

    const OPENTRACK_REG: &str = "WINE REGISTRY Version 2\n;; All keys relative to \\\\User\\\\S-1\n\n#arch=win64\n\n\
[Software\\\\NaturalPoint\\\\NATURALPOINT\\\\NPClient Location] 1700000000\n\
#time=1da\n\"Path\"=\"Z:/home/x/.local/share/opentrack-launcher/install/modules/\"\n\n\
[Software\\\\Wine] 1700000000\n\"Version\"=\"win10\"\n";

    fn setup() -> (tempdir::Dir, Prefix, PathBuf) {
        let d = tempdir::Dir::new();
        fs::write(d.path().join("user.reg"), OPENTRACK_REG).unwrap();
        fs::create_dir_all(d.path().join("drive_c")).unwrap();
        let src = d.path().join("dll");
        fs::create_dir_all(&src).unwrap();
        for n in DLLS {
            fs::write(src.join(n), b"MZ").unwrap();
        }
        let p = Prefix::new(d.path(), "podtrack");
        (d, p, src)
    }

    #[test]
    fn install_uninstall_roundtrip() {
        let (_d, p, src) = setup();
        assert_eq!(p.status().unwrap(), Status::Foreign("opentrack".into()));
        let mut t = TrackTable::default();
        t.entries.push((3450, [0x34, 0x4e, 0xce, 0xf4, 0xa2, 0xb4, 0xc6, 0x3e]));
        p.install(&src, Path::new("/dev/shm/podtrack-1000/pose-jeu"), &t, false).unwrap();
        assert_eq!(p.status().unwrap(), Status::On);
        let text = read_reg(&p.user_reg()).unwrap();
        assert_eq!(value(&text, KEY_FREETRACK, "Path").unwrap(), "C:\\\\podtrack\\\\");
        assert_eq!(value(&text, KEY_BRIDGE, "PoseFile").unwrap(), "Z:\\\\dev\\\\shm\\\\podtrack-1000\\\\pose-jeu");
        assert!(text.contains("\"3450\"=hex:34,4e,ce,f4,a2,b4,c6,3e"));
        // une seconde installation garde la sauvegarde d'origine
        p.install(&src, Path::new("/dev/shm/x"), &t, true).unwrap();
        p.uninstall().unwrap();
        let text = read_reg(&p.user_reg()).unwrap();
        assert_eq!(value(&text, KEY_NPCLIENT, "Path").unwrap(), "Z:/home/x/.local/share/opentrack-launcher/install/modules/");
        assert!(value(&text, KEY_FREETRACK, "Path").is_none());
        assert!(!text.contains("MousePods"));
        assert_eq!(value(&text, r"Software\\Wine", "Version").unwrap(), "win10");
        assert!(!p.dll_dir().exists());
    }

    #[test]
    fn key_taken_over_is_left_alone() {
        let (_d, p, src) = setup();
        p.install(&src, Path::new("/dev/shm/x"), &TrackTable::default(), false).unwrap();
        let text = read_reg(&p.user_reg()).unwrap();
        let beam = "\"Path\"=\"Z:\\\\home\\\\x\\\\.local\\\\share\\\\Beam Eye Tracker\\\\interop\\\\\"".to_string();
        let text = set_section(&text, KEY_NPCLIENT, &[beam], 1);
        write_reg(&p.user_reg(), &text).unwrap();
        assert_eq!(p.status().unwrap(), Status::Foreign("Beam Eye Tracker".into()));
        p.uninstall().unwrap();
        assert_eq!(p.status().unwrap(), Status::Foreign("Beam Eye Tracker".into()));
    }

    #[test]
    fn section_names_are_case_insensitive() {
        let text = "[software\\\\wine] 1\n\"Version\"=\"x\"\n";
        assert_eq!(value(text, r"Software\\Wine", "Version").unwrap(), "x");
    }
}

#[cfg(test)]
mod tempdir {
    use std::path::{Path, PathBuf};
    pub struct Dir(PathBuf);
    impl Dir {
        pub fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!("podtrack-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
