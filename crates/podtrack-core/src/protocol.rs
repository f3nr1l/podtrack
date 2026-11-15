//! Protocole AACP des AirPods (canal L2CAP PSM 0x1001) : paquets envoyés, décodage des paquets
//! reçus. Repris de MousePods (`airpods_mouse_controller.py`, section « AAP / RTBuddy »).
//!
//! Deux voies pour les capteurs :
//! * SensorDataWX (AirPods Pro 2/3, AirPods 4) : protobuf sur le descripteur RTBuddy 0x100000 ;
//! * RemoteHID (AirPods 3) : HID sur le descripteur RTBuddy 0x800.
//!
//! Les deux livrent le même bloc capteur de 58 octets ([`Sample::parse`]).
//! Les octets ci-dessous sont un format de protocole, pas des réglages.

use crate::vec::V3;

pub const HDR: [u8; 4] = [0x04, 0x00, 0x04, 0x00];
pub const PKT_HANDSHAKE: [u8; 16] =
    [0x00, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02, 0x00, 0, 0, 0, 0, 0, 0, 0, 0];
pub const PKT_FEATURES: [u8; 14] = [0x04, 0x00, 0x04, 0x00, 0x4d, 0x00, 0xd7, 0x00, 0, 0, 0, 0, 0, 0];
pub const PKT_NOTIFICATIONS: [u8; 10] = [0x04, 0x00, 0x04, 0x00, 0x0f, 0x00, 0xff, 0xff, 0xff, 0xff];
pub const WX_PREFIX: [u8; 10] = [0x04, 0x00, 0x04, 0x00, 0x17, 0x00, 0x00, 0x00, 0x10, 0x00];
pub const RHID_PREFIX: [u8; 10] = [0x04, 0x00, 0x04, 0x00, 0x17, 0x00, 0x00, 0x08, 0x00, 0x00];

pub const SVC_ACTIVITY: u8 = 0x0E;
pub const SVC_CMA: u8 = 0x0F;
pub const SVC_DEVMOTION6: u8 = 0x10;

pub const RHID_CONNECT: u8 = 0;
pub const RHID_DISCONNECT: u8 = 1;
pub const RHID_INPUT: u8 = 2;
pub const RHID_SET: u8 = 3;
pub const RHID_GET: u8 = 4;
pub const HID_FEATURE: u8 = 2;

/// Identifiant de réglage « raw_gestures » : active les événements de la tige.
pub const CTRL_STEM_EVENTS: u8 = 0x39;

/// Taille minimale d'un bloc capteur exploitable.
pub const BLOCK_MIN: usize = 50;

/// Commande de réglage `09 00 ident d1 d2 d3 d4`.
pub fn ctrl_cmd(ident: u8, d: [u8; 4]) -> Vec<u8> {
    let mut p = HDR.to_vec();
    p.extend_from_slice(&[0x09, 0x00, ident, d[0], d[1], d[2], d[3]]);
    p
}

pub fn stem_events(on: bool) -> Vec<u8> {
    ctrl_cmd(CTRL_STEM_EVENTS, [if on { 0x0F } else { 0x00 }, 0, 0, 0])
}

/// Paquet SensorDataWX « service_settings » : démarre (intervalle > 0) ou arrête un flux.
pub fn wx_stream(seq: u8, svc: u8, interval_us: u32) -> Vec<u8> {
    let mut body = vec![0x08, seq, 0x42, 0x0B, 0x08, svc, 0x10, 0x02, 0x1A, 0x05, 0x01];
    body.extend_from_slice(&interval_us.to_le_bytes());
    let mut p = WX_PREFIX.to_vec();
    p.extend_from_slice(&(body.len() as u16).to_le_bytes());
    p.extend_from_slice(&body);
    p
}

pub fn rhid_sub(dev: u8, kind: u8, payload: &[u8]) -> Vec<u8> {
    let n = 4 + payload.len() as u32;
    let h = (dev as u32 & 0x7F) | ((n & 0x3FF) << 7) | ((kind as u32 & 7) << 17);
    let mut v = h.to_le_bytes().to_vec();
    v.extend_from_slice(payload);
    v
}

pub fn rhid_frame(generation: u8, subs: &[Vec<u8>]) -> Vec<u8> {
    let mut body = vec![0, generation, 0, 0];
    for s in subs {
        body.extend_from_slice(s);
    }
    let mut p = RHID_PREFIX.to_vec();
    p.extend_from_slice(&(body.len() as u16).to_le_bytes());
    p.extend_from_slice(&body);
    p
}

/// Sous-paquets `(type, appareil, contenu)` d'une trame RemoteHID.
pub fn rhid_parse(pkt: &[u8]) -> Vec<(u8, u8, Vec<u8>)> {
    let mut out = Vec::new();
    if pkt.len() < 16 || pkt[..10] != RHID_PREFIX {
        return out;
    }
    let n = u16::from_le_bytes([pkt[10], pkt[11]]) as usize;
    let body = &pkt[12..(12 + n).min(pkt.len())];
    let mut i = 4;
    while i + 4 <= body.len() {
        let h = u32::from_le_bytes([body[i], body[i + 1], body[i + 2], body[i + 3]]);
        let len = ((h >> 7) & 0x3FF) as usize;
        if len < 4 {
            break;
        }
        let end = (i + len).min(body.len());
        out.push((((h >> 17) & 7) as u8, (h & 0x7F) as u8, body[i + 4..end].to_vec()));
        i += len;
    }
    out
}

/// État d'émission : compteur de séquence SensorDataWX et génération RemoteHID.
#[derive(Debug, Clone)]
pub struct Encoder {
    pub seq: u8,
    pub generation: u8,
}

impl Default for Encoder {
    fn default() -> Self {
        Self { seq: 0x40, generation: 0 }
    }
}

impl Encoder {
    pub fn stream(&mut self, svc: u8, interval_us: u32) -> Vec<u8> {
        self.seq = (self.seq.wrapping_add(1)) & 0x7F;
        wx_stream(self.seq, svc, interval_us)
    }

    pub fn rhid(&mut self, subs: &[Vec<u8>]) -> Vec<u8> {
        self.generation = self.generation.wrapping_add(1);
        rhid_frame(self.generation, subs)
    }

    pub fn rhid_host_connect(&mut self) -> Vec<u8> {
        self.rhid(&[rhid_sub(0, RHID_CONNECT, &[])])
    }

    /// Feature 1 = intervalle de rapport (µs) d'un capteur RemoteHID.
    pub fn rhid_set_interval(&mut self, dev: u8, interval_us: u32) -> Vec<u8> {
        let mut pl = vec![HID_FEATURE, 1];
        pl.extend_from_slice(&interval_us.to_le_bytes());
        self.rhid(&[rhid_sub(dev, RHID_SET, &pl)])
    }
}

// ------------------------------------------------------------------------- protobuf minimal

#[derive(Debug, Clone, PartialEq)]
pub enum PbValue<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

fn varint(b: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    while i < b.len() {
        let c = b[i];
        i += 1;
        if shift < 64 {
            v |= ((c & 0x7F) as u64) << shift;
        }
        if c & 0x80 == 0 {
            return Some((v, i));
        }
        shift += 7;
    }
    None
}

/// Champs d'un message protobuf. `Err(champs lus)` si un varint est tronqué (comme le
/// `ValueError` de MousePods) ; un type filaire inconnu arrête simplement la lecture.
pub fn pb_fields(buf: &[u8]) -> Result<Vec<(u64, PbValue<'_>)>, Vec<(u64, PbValue<'_>)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let Some((key, j)) = varint(buf, i) else { return Err(out) };
        i = j;
        let (f, wt) = (key >> 3, key & 7);
        let v = match wt {
            0 => {
                let Some((v, j)) = varint(buf, i) else { return Err(out) };
                i = j;
                PbValue::Varint(v)
            }
            2 => {
                let Some((n, j)) = varint(buf, i) else { return Err(out) };
                let start = j.min(buf.len());
                let end = j.saturating_add(n as usize).min(buf.len());
                i = j.saturating_add(n as usize);
                PbValue::Bytes(&buf[start..end])
            }
            5 | 1 => {
                let n = if wt == 5 { 4 } else { 8 };
                let start = i.min(buf.len());
                let end = (i + n).min(buf.len());
                i += n;
                PbValue::Bytes(&buf[start..end])
            }
            _ => return Ok(out),
        };
        out.push((f, v));
    }
    Ok(out)
}

/// Blocs capteur d'un paquet SensorDataWX (souvent plusieurs par paquet).
pub fn motion_blocks(pkt: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    if pkt.len() < 12 || pkt[..10] != WX_PREFIX {
        return out;
    }
    let n = u16::from_le_bytes([pkt[10], pkt[11]]) as usize;
    let body = &pkt[12..(12 + n).min(pkt.len())];
    let fields = match pb_fields(body) {
        Ok(f) => (f, false),
        Err(f) => (f, true),
    };
    for (f, v) in fields.0 {
        if f != 7 {
            continue;
        }
        let PbValue::Bytes(inner) = v else { continue };
        // une erreur dans un sous-message interrompt tout le paquet (comme en Python)
        let Ok(sub) = pb_fields(inner) else { return out };
        let mut svc = None;
        let mut blk = None;
        for (f2, v2) in sub {
            match (f2, v2) {
                (1, PbValue::Varint(s)) => svc = Some(s),
                (1, PbValue::Bytes(_)) => svc = None,
                (3, PbValue::Bytes(b)) => blk = Some(b),
                (3, PbValue::Varint(_)) => blk = None,
                _ => {}
            }
        }
        if let (Some(s), Some(b)) = (svc, blk) {
            if b.len() >= BLOCK_MIN
                && (s == SVC_ACTIVITY as u64 || s == SVC_DEVMOTION6 as u64 || s == SVC_CMA as u64)
            {
                out.push(b.to_vec());
            }
        }
    }
    let _ = fields.1;
    out
}

// ------------------------------------------------------------------------- bloc capteur

/// Un échantillon du bloc de 58 octets.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Horodatage du capteur, en nanosecondes.
    pub ts_ns: u64,
    /// Vitesse angulaire (°/s), repère de l'écouteur.
    pub w: V3,
    /// Gravité (≈ 1000 = 1 g), repère de l'écouteur ; seule la direction sert.
    pub g: V3,
    /// Partie vectorielle Q15 du quaternion d'attitude calculé par les AirPods.
    pub att: [i16; 3],
    /// Accélération utilisateur (gravité retirée), m/s².
    pub acc: V3,
}

fn i16_at(b: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([b[o], b[o + 1]])
}

impl Sample {
    pub fn parse(blk: &[u8], dps_per_lsb: f64) -> Option<Sample> {
        if blk.len() < BLOCK_MIN {
            return None;
        }
        let mut ts = [0u8; 8];
        ts.copy_from_slice(&blk[1..9]);
        let w = [i16_at(blk, 26), i16_at(blk, 28), i16_at(blk, 30)];
        let g = [i16_at(blk, 44), i16_at(blk, 46), i16_at(blk, 48)];
        let a = [i16_at(blk, 38), i16_at(blk, 40), i16_at(blk, 42)];
        let k = 9.81 / 1000.0;
        Some(Sample {
            ts_ns: u64::from_le_bytes(ts),
            w: [w[0] as f64 * dps_per_lsb, w[1] as f64 * dps_per_lsb, w[2] as f64 * dps_per_lsb],
            g: [g[0] as f64, g[1] as f64, g[2] as f64],
            att: [i16_at(blk, 20), i16_at(blk, 22), i16_at(blk, 24)],
            acc: [a[0] as f64 * k, a[1] as f64 * k, a[2] as f64 * k],
        })
    }
}

// ------------------------------------------------------------------------- événements

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    Ear,
    Out,
    Case,
}

/// Paquet 0x06 : 0 = oreille, 1 = dehors, 2 = boîtier (un écouteur porté suffit pour « oreille »).
pub fn ear_place(a: u8, b: u8) -> Place {
    if a == 0 || b == 0 {
        Place::Ear
    } else if a == 1 || b == 1 {
        Place::Out
    } else if a == 2 || b == 2 {
        Place::Case
    } else {
        Place::Out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stem {
    Single,
    Double,
    Triple,
    Long,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryPart {
    /// 0x01 seul, 0x02 droit, 0x04 gauche, 0x08 boîtier.
    pub part: u8,
    pub level: u8,
    /// 0x01 en charge, 0x02 sur batterie, 0x04 déconnecté.
    pub state: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Blocs capteur bruts (58 octets) ; RemoteHID : seulement pour l'appareil `motion_dev`.
    Motion(Vec<Vec<u8>>),
    /// Annonce RemoteHID d'un service (`AccessoryService`) sur un appareil.
    RhidAnnounce { dev: u8, service: String },
    Place(Place),
    Stem(Option<Stem>, u8),
    Battery(Vec<BatteryPart>),
    /// Chaînes d'information du modèle (nom, modèle, firmware…).
    Info(Vec<String>),
    Other,
}

/// Décode un paquet reçu. `motion_dev` : appareil RemoteHID qui porte devmotion6, s'il est connu.
pub fn decode(pkt: &[u8], motion_dev: Option<u8>) -> Vec<Event> {
    if pkt.len() >= 10 && pkt[..10] == WX_PREFIX {
        return vec![Event::Motion(motion_blocks(pkt))];
    }
    if pkt.len() >= 10 && pkt[..10] == RHID_PREFIX {
        let mut events = Vec::new();
        let mut blocks = Vec::new();
        for (kind, dev, pl) in rhid_parse(pkt) {
            if kind == RHID_CONNECT && find(&pl, b"<?xml").is_some() {
                if let Some(name) = accessory_service(&pl) {
                    events.push(Event::RhidAnnounce { dev, service: name });
                }
            } else if kind == RHID_INPUT && Some(dev) == motion_dev && pl.len() >= 51 {
                blocks.push(pl[1..].to_vec()); // pl[0] = type de rapport
            }
        }
        if !blocks.is_empty() {
            events.push(Event::Motion(blocks));
        }
        return events;
    }
    if pkt.len() >= 8 && pkt[..4] == HDR {
        match pkt[4] {
            0x06 => return vec![Event::Place(ear_place(pkt[6], pkt[7]))],
            0x19 => {
                let k = match pkt[6] {
                    0x05 => Some(Stem::Single),
                    0x06 => Some(Stem::Double),
                    0x07 => Some(Stem::Triple),
                    0x08 => Some(Stem::Long),
                    _ => None,
                };
                return vec![Event::Stem(k, pkt[6])];
            }
            0x04 => {
                let n = pkt[6] as usize;
                let mut parts = Vec::new();
                for e in 0..n {
                    let o = 7 + 5 * e;
                    if o + 4 > pkt.len() {
                        break;
                    }
                    parts.push(BatteryPart { part: pkt[o], level: pkt[o + 2], state: pkt[o + 3] });
                }
                return vec![Event::Battery(parts)];
            }
            0x1D if pkt.len() > 11 => {
                let strs = pkt[11..]
                    .split(|&c| c == 0)
                    .filter(|s| !s.is_empty())
                    .map(|s| String::from_utf8_lossy(s).into_owned())
                    .collect();
                return vec![Event::Info(strs)];
            }
            _ => {}
        }
    }
    vec![Event::Other]
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn accessory_service(pl: &[u8]) -> Option<String> {
    let key = b"AccessoryService</key><string>";
    let i = find(pl, key)? + key.len();
    let name: String = pl[i..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
        .map(|&c| c as char)
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Nom lisible d'un identifiant produit Apple (Modalias `v004Cp<pid>`).
pub fn model_name(pid: &str) -> String {
    match pid {
        "2002" => "AirPods (1re génération)",
        "200a" => "AirPods Max",
        "200e" => "AirPods Pro",
        "200f" => "AirPods (2e génération)",
        "2013" => "AirPods (3e génération)",
        "2014" => "AirPods Pro (2e génération)",
        "2019" => "AirPods 4",
        "201b" => "AirPods 4 (ANC)",
        "2024" => "AirPods Pro 2 (USB-C)",
        "2027" => "AirPods Pro 3",
        "202d" => "AirPods Max (USB-C)",
        _ => return format!("Apple 0x{pid}"),
    }
    .to_string()
}

/// AirPods 3 : capteurs seulement par RemoteHID.
pub fn is_remotehid_only(pid: &str) -> bool {
    pid == "2013"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }
    fn tohex(b: &[u8]) -> String {
        b.iter().map(|c| format!("{c:02x}")).collect()
    }

    #[test]
    fn wx_public_vector() {
        let p = hex(concat!(
            "04000400170000001000450008ff7f10013a3e08101a3a015021d8c7130d0000030043998d62bb00",
            "0000006cb9a83eb5fc0300020001005e1003d37706fdff000001008d020d035c0034aac63786ff0000"
        ));
        let blocks = motion_blocks(&p);
        assert_eq!(blocks.len(), 1);
        assert_eq!(tohex(&blocks[0][20..26]), "6cb9a83eb5fc");
    }

    #[test]
    fn rhid_header_and_frames() {
        let h = u32::from_le_bytes([0x87, 0x98, 0x01, 0x00]);
        assert_eq!(h & 0x7F, 7);
        assert_eq!((h >> 7) & 0x3FF, 817);
        assert_eq!((h >> 17) & 7, 0);
        let mut pl = vec![2, 4];
        pl.extend_from_slice(&1u32.to_le_bytes());
        assert_eq!(tohex(&rhid_sub(8, RHID_SET, &pl)), "08050600020401000000");
        let f = rhid_frame(5, &[rhid_sub(0, RHID_CONNECT, &[])]);
        assert_eq!(tohex(&f), "04000400170000080000".to_owned() + "0800" + "00050000" + "00020000");
    }

    #[test]
    fn first_stream_sequence_matches_mousepods() {
        let mut e = Encoder::default();
        assert_eq!(tohex(&e.stream(SVC_ACTIVITY, 40000)), "04000400170000001000".to_owned() + "0f00" + "0841420b080e10021a0501409c0000");
        assert_eq!(tohex(&e.stream(SVC_DEVMOTION6, 10000)), "04000400170000001000".to_owned() + "0f00" + "0842420b081010021a050110270000");
        assert_eq!(tohex(&e.stream(SVC_ACTIVITY, 0)), "04000400170000001000".to_owned() + "0f00" + "0843420b080e10021a050100000000");
        assert_eq!(tohex(&e.rhid_host_connect()), "04000400170000080000".to_owned() + "0800" + "00010000" + "00020000");
    }

    #[test]
    fn ear_places() {
        assert_eq!(ear_place(0, 0), Place::Ear);
        assert_eq!(ear_place(2, 2), Place::Case);
        assert_eq!(ear_place(1, 1), Place::Out);
        assert_eq!(ear_place(0, 3), Place::Ear);
    }

    #[test]
    fn rhid_roundtrip_input() {
        let mut blk = vec![0u8; 58];
        blk[0] = 1;
        let mut pl = vec![0u8];
        pl.extend_from_slice(&blk);
        let f = rhid_frame(1, &[rhid_sub(7, RHID_INPUT, &pl)]);
        assert_eq!(rhid_parse(&f)[0].0, RHID_INPUT);
        assert_eq!(decode(&f, Some(7)), vec![Event::Motion(vec![blk])]);
    }

    #[test]
    fn device_announce() {
        let plist = b"<?xml version=\"1.0\"?><plist><dict><key>AccessoryService</key><string>devmotion6</string></dict></plist>";
        let f = rhid_frame(1, &[rhid_sub(7, RHID_CONNECT, plist)]);
        assert_eq!(decode(&f, None), vec![Event::RhidAnnounce { dev: 7, service: "devmotion6".into() }]);
    }
}
