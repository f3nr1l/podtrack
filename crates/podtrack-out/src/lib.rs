//! podtrack-out : sorties de podtrack (fichier de pose, registre Wine, opentrack, uinput).

pub mod posefile;
pub mod procs;
pub mod tracklist;
pub mod udp;
pub mod uinput;
pub mod winereg;

/// DLL TrackIR / FreeTrack fournies avec podtrack (compilées depuis MousePods `native/`).
pub mod bundled_dll {
    pub const NPCLIENT: &[u8] = include_bytes!("../data/dll/NPClient.dll");
    pub const NPCLIENT64: &[u8] = include_bytes!("../data/dll/NPClient64.dll");
    pub const FREETRACK: &[u8] = include_bytes!("../data/dll/freetrackclient.dll");
    pub const FREETRACK64: &[u8] = include_bytes!("../data/dll/freetrackclient64.dll");

    /// Écrit les DLL dans `dir` (créé au besoin) et renvoie ce dossier.
    pub fn extract(dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        std::fs::create_dir_all(dir)?;
        for (n, b) in [("NPClient.dll", NPCLIENT), ("NPClient64.dll", NPCLIENT64), ("freetrackclient.dll", FREETRACK), ("freetrackclient64.dll", FREETRACK64)] {
            std::fs::write(dir.join(n), b)?;
        }
        Ok(dir.to_path_buf())
    }
}
