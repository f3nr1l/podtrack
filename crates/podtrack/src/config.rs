//! Configuration complète de podtrack (`~/.config/podtrack/config.toml`). Chaque réglage a sa
//! valeur par défaut ; un fichier partiel est complété, un fichier absent vaut tous les défauts.

use anyhow::{Context, Result};
use podtrack_bt::session::BtConfig;
use podtrack_core::calibration::{DeviceCalibConfig, GameCalibConfig};
use podtrack_core::engine::{DeviceProfile, EngineConfig};
use podtrack_core::mouse::MouseConfig;
use podtrack_out::uinput::JoystickConfig;
use podtrack_core::profile::{Baseline, GameProfile};
use podtrack_core::tracker::TrackerConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TrackerChoice {
    /// podtrack fournit le suivi de tête du jeu.
    #[default]
    Podtrack,
    /// Un autre traqueur (celui installé avant podtrack) : podtrack rend les clés d'origine.
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum GameKind {
    /// Préfixe Wine hors Steam (Lutris, Heroic, installation manuelle…).
    Wine { prefix: PathBuf },
    /// Jeu Steam sous Proton.
    Steam { appid: String, #[serde(default)] flatpak: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Game {
    pub name: String,
    #[serde(flatten)]
    pub kind: GameKind,
    /// Le démon écrit la pose de ce jeu.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub tracker: TrackerChoice,
    #[serde(default)]
    pub profile: GameProfile,
}

impl Game {
    pub fn prefix(&self, steam: &SteamConfig) -> Option<PathBuf> {
        match &self.kind {
            GameKind::Wine { prefix } => Some(expand(prefix)),
            GameKind::Steam { appid, flatpak } => crate::steam::proton_prefix(steam, appid, *flatpak),
        }
    }
}

/// `~` en tête de chemin → dossier personnel.
pub fn expand(p: &Path) -> PathBuf {
    match p.strip_prefix("~") {
        Ok(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        Err(_) => p.to_path_buf(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    /// Cadence d'écriture des poses et d'envoi UDP (Hz).
    pub rate_hz: f64,
    /// Dossier de mémoire partagée des fichiers de pose.
    pub shm_dir: PathBuf,
    /// Préfixe du dossier privé (`<shm_dir>/<nom>-<uid>`).
    pub shm_name: String,
    /// Sans échantillon depuis ce délai, la pose n'est plus rafraîchie : les DLL recentrent (s).
    pub stale_after_s: f64,
    /// Pas de temps maximal du filtre de profil (s).
    pub max_dt: f64,
    /// Prolongation du mouvement entre deux paquets Bluetooth, au plus ce délai (s ; 0 = aucune).
    pub extrapolate_max_s: f64,
    /// Lissage de la vitesse servant à la prolongation : poids de la nouvelle mesure (0..1).
    pub extrapolate_alpha: f64,
    /// Écart entre deux échantillons au-delà duquel la prolongation repart de zéro (°).
    pub extrapolate_jump_deg: f64,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self { rate_hz: 120.0, shm_dir: crate::sandbox::default_shm_dir(), shm_name: "podtrack".into(), stale_after_s: 1.0, max_dt: 0.1, extrapolate_max_s: 0.04, extrapolate_alpha: 0.3, extrapolate_jump_deg: 15.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpentrackConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    /// Profil appliqué à la sortie UDP (courbes, zone morte, lissage).
    pub profile: GameProfile,
}

impl Default for OpentrackConfig {
    fn default() -> Self {
        let mut profile = GameProfile { own: true, smoothing: 0.0, ..Default::default() };
        for a in [&mut profile.axes.yaw, &mut profile.axes.pitch, &mut profile.axes.roll] {
            a.edge_neg = 180.0;
            a.edge_pos = 180.0;
            a.target = 180.0;
            a.curve = 1.0;
            a.enabled = true;
        }
        Self { enabled: false, host: "127.0.0.1".into(), port: 4242, profile }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BridgeConfig {
    /// Dossier de nos DLL dans `drive_c`.
    pub dir_name: String,
    /// Liste des jeux opentrack (tables TrackIR) ; vide = liste fournie avec podtrack.
    pub games_csv: Option<PathBuf>,
    /// Dossier de DLL à installer ; vide = DLL fournies avec podtrack.
    pub dll_dir: Option<PathBuf>,
    /// Journal des DLL (`C:\<dir>\npclient.log`).
    pub dll_debug: bool,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self { dir_name: "podtrack".into(), games_csv: None, dll_dir: None, dll_debug: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SteamConfig {
    /// Racines Steam à explorer (natif et Flatpak par défaut).
    pub roots: Vec<PathBuf>,
    pub flatpak_roots: Vec<PathBuf>,
}

impl Default for SteamConfig {
    fn default() -> Self {
        Self {
            roots: vec!["~/.local/share/Steam".into(), "~/.steam/steam".into()],
            flatpak_roots: vec!["~/.var/app/com.valvesoftware.Steam/.local/share/Steam".into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaceConfig {
    /// Un changement oreille / boîtier / main doit durer ce délai pour être pris en compte (s).
    pub debounce_s: f64,
    /// Fréquence de la détection d'un jeu en cours (s).
    pub game_scan_s: f64,
}

impl Default for PlaceConfig {
    fn default() -> Self {
        Self { debounce_s: 3.0, game_scan_s: 2.0 }
    }
}

/// Surveillance du son des AirPods : le profil mains libres (micro) réserve l'essentiel de la
/// liaison Bluetooth et ralentit fortement les capteurs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Fréquence de la vérification du profil audio (s ; 0 = jamais).
    pub check_s: f64,
    /// Début des noms de profils PipeWire / PulseAudio « mains libres ».
    pub handsfree_profiles: Vec<String>,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self { check_s: 5.0, handsfree_profiles: vec!["headset-head-unit".into(), "handsfree".into(), "hsp".into(), "hfp".into()] }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    /// Raccourcis (format GNOME, ex. `<Super>XF86AudioMute`), vides = aucun.
    pub recenter: String,
    pub toggle: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self { recenter: String::new(), toggle: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub bluetooth: BtConfig,
    pub engine: EngineConfig,
    pub tracker: TrackerConfig,
    pub output: OutputConfig,
    pub opentrack: OpentrackConfig,
    /// Manette virtuelle « tête » (jeux Linux natifs).
    pub joystick: JoystickConfig,
    /// Souris par la tête.
    pub mouse: MouseConfig,
    pub bridge: BridgeConfig,
    pub steam: SteamConfig,
    pub place: PlaceConfig,
    pub audio: AudioConfig,
    pub hotkeys: Hotkeys,
    pub device_calibration: DeviceCalibConfig,
    pub game_calibration: GameCalibConfig,
    /// Calibration de référence de l'amplitude (jeux sans valeurs propres).
    pub baseline: Option<Baseline>,
    /// Profils d'axes par appareil : clé `<pid>_<source>` (ex. `2027_head`).
    pub devices: BTreeMap<String, DeviceProfile>,
    /// Jeux suivis, par identifiant court (lettres, chiffres, `-`, `_`).
    pub games: BTreeMap<String, Game>,
}

pub fn path() -> PathBuf {
    if let Ok(p) = std::env::var("PODTRACK_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config")).join("podtrack").join("config.toml")
}

pub fn load(p: &Path) -> Result<Config> {
    match std::fs::read_to_string(p) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("lecture de {}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e.into()),
    }
}

/// Écriture atomique (fichier temporaire puis renommage).
pub fn save(p: &Path, cfg: &Config) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    let tmp = p.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

/// Lit, modifie, réécrit.
pub fn update(p: &Path, f: impl FnOnce(&mut Config) -> Result<()>) -> Result<Config> {
    let mut c = load(p)?;
    f(&mut c)?;
    save(p, &c)?;
    Ok(c)
}

pub fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}
