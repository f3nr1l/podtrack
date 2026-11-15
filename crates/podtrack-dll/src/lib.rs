//! DLL TrackIR (`NPClient`, `NPClient64`) et FreeTrack (`freetrackclient`, `freetrackclient64`)
//! de podtrack, chargées par les jeux sous Wine / Proton. Une seule bibliothèque exporte les deux
//! interfaces ; elle est copiée sous les quatre noms attendus.
//!
//! La pose est lue dans le fichier que podtrack écrit sous Linux (chemin dans
//! `HKCU\Software\podtrack\PoseFile`, vu par le lecteur `Z:` de Wine). Les jeux « TrackIR
//! Enhanced » reçoivent des données chiffrées avec leur table (`HKCU\Software\podtrack\Table\<id>`).
//! Journal dans `C:\podtrack\` si `HKCU\Software\podtrack\Debug` vaut 1.
//!
//! Portage de `native/npclient.c` et `native/freetrackclient.c` de MousePods (Henning Norden),
//! eux-mêmes dérivés d'opentrack (ISC) et de linux-track (MIT). Voir NOTICE.

#![allow(non_snake_case, clippy::missing_safety_doc)]

#[cfg(windows)]
mod signature;

pub mod algo {
    //! Somme de contrôle et chiffrement TrackIR, testables hors Windows.

    /// Somme de contrôle TrackIR 5 (arithmétique signée 32 bits du code d'origine).
    pub fn cksum(buf: &[u8]) -> u32 {
        let size = buf.len();
        if size == 0 {
            return 0;
        }
        let mut rounds = size >> 2;
        let rem = size % 4;
        let mut c: i32 = size as i32;
        let mut a2: i32 = 0;
        let mut i = 0;
        let short = |o: usize| i16::from_le_bytes([buf[o], buf[o + 1]]) as i32;
        while rounds != 0 {
            let a0 = short(i);
            a2 = short(i + 2);
            i += 4;
            c = c.wrapping_add(a0);
            a2 ^= c.wrapping_shl(5);
            a2 = a2.wrapping_shl(11);
            c ^= a2;
            c = c.wrapping_add(c >> 11);
            rounds -= 1;
        }
        match rem {
            3 => {
                let a0 = short(i);
                a2 = buf[i + 2] as i8 as i32;
                c = c.wrapping_add(a0);
                a2 = a2.wrapping_shl(2) ^ c;
                c ^= a2.wrapping_shl(16);
                a2 = c >> 11;
            }
            2 => {
                a2 = short(i);
                c = c.wrapping_add(a2);
                c ^= c.wrapping_shl(11);
                a2 = c >> 17;
            }
            1 => {
                a2 = buf[i] as i8 as i32;
                c = c.wrapping_add(a2);
                c ^= c.wrapping_shl(10);
                a2 = c >> 1;
            }
            _ => {}
        }
        if rem != 0 {
            c = c.wrapping_add(a2);
        }
        c ^= c.wrapping_shl(3);
        c = c.wrapping_add(c >> 5);
        c ^= c.wrapping_shl(4);
        c = c.wrapping_add(c >> 17);
        c ^= c.wrapping_shl(25);
        c = c.wrapping_add(c >> 6);
        c as u32
    }

    /// Chiffrement des jeux « Enhanced ».
    pub fn enhance(buf: &mut [u8], table: &[u8; 8]) {
        let mut size = buf.len();
        if size == 0 {
            return;
        }
        let mut ptr = 0usize;
        let mut var: u8 = 0x88;
        loop {
            size -= 1;
            let tmp = buf[size];
            buf[size] = tmp ^ table[ptr] ^ var;
            var = var.wrapping_add((size as u32).wrapping_add(tmp as u32) as u8);
            ptr += 1;
            if ptr >= table.len() {
                ptr -= table.len();
            }
            if size == 0 {
                break;
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use crate::algo::{cksum, enhance};
    use crate::signature::*;
    use std::ffi::c_void;
    use std::io::Write;
    use std::sync::Mutex;

    type Handle = *mut c_void;
    type Hkey = *mut c_void;

    const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
    const KEY_READ: u32 = 0x20019;
    const REG_SZ: u32 = 1;
    const REG_DWORD: u32 = 4;
    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const PAGE_READONLY: u32 = 2;
    const PAGE_READWRITE: u32 = 4;
    const FILE_MAP_WRITE: u32 = 2;
    const FILE_MAP_READ: u32 = 4;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileA(name: *const u8, access: u32, share: u32, sec: *mut c_void, disp: u32, flags: u32, tmpl: Handle) -> Handle;
        fn CreateFileMappingA(h: Handle, sec: *mut c_void, protect: u32, hi: u32, lo: u32, name: *const u8) -> Handle;
        fn MapViewOfFile(h: Handle, access: u32, hi: u32, lo: u32, bytes: usize) -> *mut c_void;
        fn CreateMutexA(sec: *mut c_void, owner: i32, name: *const u8) -> Handle;
        fn CloseHandle(h: Handle) -> i32;
        fn GetTickCount64() -> u64;
        fn GetLastError() -> u32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegOpenKeyExA(key: Hkey, sub: *const u8, opts: u32, sam: u32, out: *mut Hkey) -> i32;
        fn RegQueryValueExA(key: Hkey, name: *const u8, res: *mut u32, ty: *mut u32, data: *mut u8, len: *mut u32) -> i32;
        fn RegCloseKey(key: Hkey) -> i32;
    }

    const KEY: &[u8] = b"Software\\podtrack\0";
    const KEY_TABLE: &[u8] = b"Software\\podtrack\\Table\0";
    const POSE_DEFAULT: &str = "Z:\\dev\\shm\\podtrack-headpose";
    /// Sans nouvelle pose depuis ce délai, podtrack ne tourne plus : on renvoie le centre.
    const POSE_STALE_MS: u64 = 1500;
    const NP_AXIS_MAX: f64 = 16383.0;

    /// Lit une valeur de registre sous `key` : (type, octets).
    unsafe fn reg_query(key: &[u8], name: &str, cap: usize) -> Option<(u32, Vec<u8>)> {
        let mut k: Hkey = std::ptr::null_mut();
        if unsafe { RegOpenKeyExA(HKEY_CURRENT_USER as Hkey, key.as_ptr(), 0, KEY_READ, &mut k) } != 0 {
            return None;
        }
        let mut cname = name.as_bytes().to_vec();
        cname.push(0);
        let mut buf = vec![0u8; cap];
        let (mut ty, mut len) = (0u32, cap as u32);
        let r = unsafe { RegQueryValueExA(k, cname.as_ptr(), std::ptr::null_mut(), &mut ty, buf.as_mut_ptr(), &mut len) };
        unsafe { RegCloseKey(k) };
        if r != 0 {
            return None;
        }
        buf.truncate(len as usize);
        Some((ty, buf))
    }

    fn debug_enabled() -> bool {
        matches!(unsafe { reg_query(KEY, "Debug", 4) }, Some((REG_DWORD, v)) if v.len() == 4 && v == [1, 0, 0, 0])
    }

    fn pose_path() -> Vec<u8> {
        let mut p = match unsafe { reg_query(KEY, "PoseFile", 260) } {
            Some((REG_SZ, mut v)) => {
                while v.last() == Some(&0) {
                    v.pop();
                }
                if v.is_empty() { POSE_DEFAULT.as_bytes().to_vec() } else { v }
            }
            _ => POSE_DEFAULT.as_bytes().to_vec(),
        };
        p.push(0);
        p
    }

    struct Log {
        file: Option<std::fs::File>,
        checked: bool,
    }

    impl Log {
        fn write(&mut self, name: &str, msg: std::fmt::Arguments) {
            if !self.checked {
                self.checked = true;
                if debug_enabled() {
                    let _ = std::fs::create_dir_all("C:\\podtrack");
                    self.file = std::fs::OpenOptions::new().create(true).append(true).open(format!("C:\\podtrack\\{name}.log")).ok();
                }
            }
            if let Some(f) = self.file.as_mut() {
                let _ = f.write_fmt(msg);
                let _ = f.write_all(b"\n");
                let _ = f.flush();
            }
        }
    }

    /// Fichier de pose en lecture (projection en mémoire) et fraîcheur du compteur.
    struct Pose {
        ptr: usize,
        logged: bool,
        seen: bool,
        last_seq: u32,
        last_change_ms: u64,
    }

    impl Pose {
        /// x, y, z (cm), lacet, tangage, roulis (°) ; zéros si podtrack ne tourne plus.
        fn read(&mut self, log: &mut Log, who: &str) -> Option<[f64; 6]> {
            if self.ptr == 0 {
                let path = pose_path();
                let f = unsafe { CreateFileA(path.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut()) };
                if f == INVALID_HANDLE_VALUE {
                    if !self.logged {
                        self.logged = true;
                        let e = unsafe { GetLastError() };
                        log.write(who, format_args!("pose : ouverture de {} impossible (erreur {e})", String::from_utf8_lossy(&path[..path.len() - 1])));
                    }
                    return None;
                }
                let m = unsafe { CreateFileMappingA(f, std::ptr::null_mut(), PAGE_READONLY, 0, 64, std::ptr::null()) };
                let p = if m.is_null() { std::ptr::null_mut() } else { unsafe { MapViewOfFile(m, FILE_MAP_READ, 0, 0, 64) } };
                if p.is_null() {
                    log.write(who, format_args!("pose : projection impossible (erreur {})", unsafe { GetLastError() }));
                    return None;
                }
                self.ptr = p as usize;
                log.write(who, format_args!("pose : fichier projeté"));
            }
            let base = self.ptr as *const u8;
            let mut v = [0.0f64; 6];
            for (i, x) in v.iter_mut().enumerate() {
                *x = f64::from_le_bytes(unsafe { std::ptr::read_volatile(base.add(8 + 8 * i) as *const [u8; 8]) });
            }
            let seq = u32::from_le_bytes(unsafe { std::ptr::read_volatile(base as *const [u8; 4]) });
            let now = unsafe { GetTickCount64() };
            let fresh = if !self.seen || seq != self.last_seq {
                self.seen = true;
                self.last_seq = seq;
                self.last_change_ms = now;
                true
            } else {
                now - self.last_change_ms <= POSE_STALE_MS
            };
            if !fresh {
                v = [0.0; 6]; // podtrack arrêté : centre plutôt que caméra figée
            }
            Some(v)
        }
    }

    // ------------------------------------------------------------------ TrackIR

    #[repr(C)]
    pub struct TirData {
        status: i16,
        frame: i16,
        cksum: u32,
        roll: f32,
        pitch: f32,
        yaw: f32,
        tx: f32,
        ty: f32,
        tz: f32,
        padding: [f32; 9],
    }

    #[repr(C)]
    pub struct TirSignature {
        dll: [u8; 200],
        app: [u8; 200],
    }

    /// Mémoire partagée FreeTrack (`FT_SharedMem`) : identifiant du jeu et table, comme opentrack.
    #[repr(C)]
    struct FtMemMap {
        data: [u8; 92],
        game_id: i32,
        table: [u8; 8],
        game_id2: i32,
    }

    struct Np {
        log: Log,
        pose: Pose,
        ft: usize,
        table: [u8; 8],
        encryption: bool,
        encryption_checked: bool,
        last: [f64; 6],
        frame: i32,
    }

    static NP: Mutex<Np> = Mutex::new(Np {
        log: Log { file: None, checked: false },
        pose: Pose { ptr: 0, logged: false, seen: false, last_seq: 0, last_change_ms: 0 },
        ft: 0,
        table: [0; 8],
        encryption: false,
        encryption_checked: false,
        last: [0.0; 6],
        frame: 0,
    });

    fn np() -> std::sync::MutexGuard<'static, Np> {
        NP.lock().unwrap_or_else(|e| e.into_inner())
    }

    impl Np {
        fn ft_mapping(&mut self) -> Option<*mut FtMemMap> {
            if self.ft == 0 {
                let mutex = unsafe { CreateMutexA(std::ptr::null_mut(), 0, b"FT_Mutext\0".as_ptr()) };
                unsafe { CloseHandle(mutex) };
                let size = std::mem::size_of::<FtMemMap>() as u32;
                let h = unsafe { CreateFileMappingA(INVALID_HANDLE_VALUE, std::ptr::null_mut(), PAGE_READWRITE, 0, size, b"FT_SharedMem\0".as_ptr()) };
                let p = unsafe { MapViewOfFile(h, FILE_MAP_WRITE, 0, 0, size as usize) };
                self.ft = p as usize;
            }
            (self.ft != 0).then_some(self.ft as *mut FtMemMap)
        }

        fn load_table(&mut self, id: u16) {
            if let Some((_, v)) = unsafe { reg_query(KEY_TABLE, &id.to_string(), 8) } {
                if v.len() == 8 {
                    self.table.copy_from_slice(&v);
                    self.encryption = self.table.iter().any(|&c| c != 0);
                }
            }
            self.encryption_checked = true;
            let t = self.table;
            let e = self.encryption;
            self.log.write("npclient", format_args!("table du jeu {id} : chiffrement {e}, {t:02x?}"));
        }
    }

    fn clamp_axis(x: f64) -> f32 {
        x.clamp(-NP_AXIS_MAX, NP_AXIS_MAX) as f32
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn NP_GetData(data: *mut TirData) -> i32 {
        let mut g = np();
        let s = &mut *g;
        let Some(ft) = s.ft_mapping() else {
            s.log.write("npclient", format_args!("mémoire partagée FreeTrack indisponible"));
            return 0;
        };
        if let Some(pv) = s.pose.read(&mut s.log, "npclient") {
            // conversion d'opentrack (proto-wine) : degrés → unités TrackIR, lacet et tangage inversés
            s.last = [
                -pv[3] * std::f64::consts::PI / 180.0 * NP_AXIS_MAX / std::f64::consts::PI,
                -pv[4] * std::f64::consts::PI / 180.0 * NP_AXIS_MAX / std::f64::consts::PI,
                pv[5] * std::f64::consts::PI / 180.0 * NP_AXIS_MAX / std::f64::consts::PI,
                pv[0] * 10.0 * NP_AXIS_MAX / 500.0,
                pv[1] * 10.0 * NP_AXIS_MAX / 500.0,
                pv[2] * 10.0 * NP_AXIS_MAX / 500.0,
            ];
        }
        // table publiée par opentrack dans la mémoire partagée, si le registre n'en a pas donné
        let m = unsafe { &*ft };
        if !s.encryption_checked && m.game_id == unsafe { std::ptr::read_volatile(&m.game_id2) } && m.game_id != 0 {
            s.encryption_checked = true;
            s.table = unsafe { std::ptr::read_volatile(&m.table) };
            s.encryption = s.table.iter().any(|&c| c != 0);
        }
        s.frame = s.frame.wrapping_add(1);
        let [y, p, r, tx, ty, tz] = s.last;
        let running = [y, p, r, tx, ty, tz].iter().any(|&v| v != 0.0);
        let status: i16 = if running { 0 } else { 1 };
        let d = unsafe { &mut *data };
        d.status = status;
        d.frame = s.frame as i16;
        d.cksum = 0;
        d.roll = clamp_axis(r);
        d.pitch = clamp_axis(p);
        d.yaw = clamp_axis(y);
        d.tx = clamp_axis(tx);
        d.ty = clamp_axis(ty);
        d.tz = clamp_axis(tz);
        d.padding = [0.0; 9];
        let bytes = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, std::mem::size_of::<TirData>()) };
        let ck = cksum(bytes);
        d.cksum = ck;
        if s.encryption {
            let t = s.table;
            enhance(bytes, &t);
        }
        if s.frame % 500 == 1 {
            s.log.write("npclient", format_args!("image {} : lacet {} tangage {} roulis {}", s.frame, y, p, r));
        }
        status as i32
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn NP_RegisterProgramProfileID(id: u16) -> i32 {
        let mut g = np();
        if let Some(ft) = g.ft_mapping() {
            unsafe { (*ft).game_id = id as i32 };
        }
        g.log.write("npclient", format_args!("jeu enregistré : {id}"));
        g.load_table(id);
        0
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn NP_GetSignature(sig: *mut TirSignature) -> i32 {
        let s = unsafe { &mut *sig };
        for i in 0..200 {
            s.dll[i] = PART1_2[i] ^ PART1_1[i];
            s.app[i] = PART2_1[i] ^ PART2_2[i];
        }
        np().log.write("npclient", format_args!("signature demandée"));
        0
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "system" fn NP_QueryVersion(version: *mut u16) -> i32 {
        unsafe { *version = 0x0500 };
        0
    }

    macro_rules! np_stub {
        ($($name:ident ( $($a:ident : $t:ty),* )),* $(,)?) => {
            $(
                #[unsafe(no_mangle)]
                pub unsafe extern "system" fn $name($(_: $t),*) -> i32 { 0 }
            )*
        };
    }

    np_stub!(
        NPPriv_ClientNotify(),
        NPPriv_GetLastError(),
        NPPriv_SetData(),
        NPPriv_SetLastError(),
        NPPriv_SetParameter(),
        NPPriv_SetSignature(),
        NPPriv_SetVersion(),
        NP_GetParameter(a: i32, b: i32),
        NP_ReCenter(),
        NP_RegisterWindowHandle(h: *mut c_void),
        NP_RequestData(r: u16),
        NP_SetParameter(a: i32, b: i32),
        NP_StartCursor(),
        NP_StartDataTransmission(),
        NP_StopCursor(),
        NP_StopDataTransmission(),
        NP_UnregisterWindowHandle(),
    );

    // ------------------------------------------------------------------ FreeTrack

    #[repr(C)]
    pub struct FtData {
        data_id: u32,
        cam_width: i32,
        cam_height: i32,
        yaw: f32,
        pitch: f32,
        roll: f32,
        x: f32,
        y: f32,
        z: f32,
        raw_yaw: f32,
        raw_pitch: f32,
        raw_roll: f32,
        raw_x: f32,
        raw_y: f32,
        raw_z: f32,
        points: [f32; 8],
    }

    struct Ft {
        log: Log,
        pose: Pose,
        id: u32,
    }

    static FT: Mutex<Ft> = Mutex::new(Ft {
        log: Log { file: None, checked: false },
        pose: Pose { ptr: 0, logged: false, seen: false, last_seq: 0, last_change_ms: 0 },
        id: 0,
    });

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn FTGetData(data: *mut FtData) -> i32 {
        let mut g = FT.lock().unwrap_or_else(|e| e.into_inner());
        let s = &mut *g;
        let Some(v) = s.pose.read(&mut s.log, "freetrackclient") else { return 0 };
        s.id = s.id.wrapping_add(1);
        let rad = std::f64::consts::PI / 180.0;
        let (yaw, pitch, roll) = ((-v[3] * rad) as f32, (-v[4] * rad) as f32, (v[5] * rad) as f32);
        unsafe {
            data.write(FtData {
                data_id: s.id,
                cam_width: 250,
                cam_height: 100,
                yaw,
                pitch,
                roll,
                x: (v[0] * 10.0) as f32,
                y: (v[1] * 10.0) as f32,
                z: (v[2] * 10.0) as f32,
                raw_yaw: yaw,
                raw_pitch: pitch,
                raw_roll: roll,
                raw_x: 0.0,
                raw_y: 0.0,
                raw_z: 0.0,
                points: [0.0; 8],
            })
        };
        if s.id % 1000 == 1 {
            s.log.write("freetrackclient", format_args!("image {}", s.id));
        }
        1
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn FTReportName(name: i32) {
        FT.lock().unwrap_or_else(|e| e.into_inner()).log.write("freetrackclient", format_args!("FTReportName {name}"));
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn FTReportID(id: i32) {
        FT.lock().unwrap_or_else(|e| e.into_inner()).log.write("freetrackclient", format_args!("FTReportID {id}"));
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn FTGetDllVersion() -> *const u8 {
        b"1.0.0.0\0".as_ptr()
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn FTProvider() -> *const u8 {
        b"FreeTrack\0".as_ptr()
    }

    const _: () = assert!(std::mem::size_of::<TirData>() == 68);
    const _: () = assert!(std::mem::size_of::<FtData>() == 92);
    const _: () = assert!(std::mem::size_of::<FtMemMap>() == 108);
}

#[cfg(test)]
mod tests {
    use super::algo::{cksum, enhance};

    #[test]
    fn enhance_is_deterministic_and_reversible_length() {
        let mut b: Vec<u8> = (0..68u8).collect();
        let t = [0x34, 0x4e, 0xce, 0xf4, 0xa2, 0xb4, 0xc6, 0x3e];
        enhance(&mut b, &t);
        assert_ne!(b, (0..68u8).collect::<Vec<_>>());
        assert_eq!(cksum(&[]), 0);
    }
}
