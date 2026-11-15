//! Fichier de pose partagé avec les DLL (voir `podtrack_core::pose`), en mémoire partagée.

use anyhow::{Result, bail};
use memmap2::MmapMut;
use podtrack_core::pose;
use podtrack_core::tracker::Pose6;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// Dossier privé (0700, à nous) qui contient les fichiers de pose ; refuse un dossier piégé.
pub fn private_dir(base: &Path, name: &str) -> Result<PathBuf> {
    let d = base.join(format!("{name}-{}", uid()));
    match fs::create_dir(&d) {
        Ok(()) => fs::set_permissions(&d, fs::Permissions::from_mode(0o700))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let m = fs::symlink_metadata(&d)?;
    if !m.is_dir() || m.uid() != uid() || m.mode() & 0o077 != 0 {
        bail!("dossier de pose douteux : {}", d.display());
    }
    Ok(d)
}

/// Un identifiant de jeu ne contient que des caractères sûrs pour un nom de fichier.
pub fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub struct PoseFile {
    pub path: PathBuf,
    mm: MmapMut,
    seq: u32,
}

impl PoseFile {
    pub fn open(path: &Path) -> Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let m = f.metadata()?;
        if !m.is_file() || m.uid() != uid() {
            bail!("fichier de pose douteux : {}", path.display());
        }
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        f.set_len(pose::SIZE as u64)?;
        let mut mm = unsafe { MmapMut::map_mut(&f)? };
        let seq = u32::from_le_bytes(mm[0..4].try_into().unwrap());
        mm[pose::OFF_VERSION..pose::OFF_VERSION + 4].copy_from_slice(&pose::VERSION.to_le_bytes());
        Ok(Self { path: path.to_path_buf(), mm, seq })
    }

    /// Écrit la pose puis, en dernier, le compteur : la DLL sait qu'une pose neuve est arrivée.
    pub fn write(&mut self, p: &Pose6, t: f64) {
        pose::write_body(&mut self.mm, p, t);
        self.seq = self.seq.wrapping_add(1);
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
        self.mm[0..4].copy_from_slice(&self.seq.to_le_bytes());
    }

    pub fn seq(&self) -> u32 {
        self.seq
    }

    pub fn remove(self) {
        let _ = fs::remove_file(&self.path);
    }
}
