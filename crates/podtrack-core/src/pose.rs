//! Format du fichier de pose lu par les DLL TrackIR / FreeTrack (64 octets, petit-boutiste) :
//! `u32 seq | u32 version | f64 x, y, z (cm) | f64 lacet, tangage, roulis (°) | f64 t (s)`.
//! `seq` est écrit en dernier : la DLL ne lit jamais une pose à moitié écrite comme neuve.

use crate::tracker::Pose6;

pub const SIZE: usize = 64;
pub const VERSION: u32 = 1;
pub const OFF_SEQ: usize = 0;
pub const OFF_VERSION: usize = 4;
pub const OFF_POSE: usize = 8;

/// Écrit pose et horodatage (sans toucher à `seq`).
pub fn write_body(buf: &mut [u8], pose: &Pose6, t: f64) {
    for (i, v) in pose.iter().chain(std::iter::once(&t)).enumerate() {
        let o = OFF_POSE + 8 * i;
        buf[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }
}

pub fn read(buf: &[u8]) -> (u32, u32, Pose6, f64) {
    let u = |o: usize| u32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
    let f = |o: usize| f64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
    let mut p = [0.0; 6];
    for (i, v) in p.iter_mut().enumerate() {
        *v = f(OFF_POSE + 8 * i);
    }
    (u(OFF_SEQ), u(OFF_VERSION), p, f(OFF_POSE + 48))
}

/// Paquet opentrack « UDP over network » : 6 doubles (x, y, z, lacet, tangage, roulis), 48 octets.
pub fn opentrack_packet(pose: &Pose6) -> Option<[u8; 48]> {
    if !pose.iter().all(|v| v.is_finite()) {
        return None; // opentrack jette les paquets NaN / infinis
    }
    let mut b = [0u8; 48];
    for (i, v) in pose.iter().enumerate() {
        b[8 * i..8 * i + 8].copy_from_slice(&v.to_le_bytes());
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut b = [0u8; SIZE];
        write_body(&mut b, &[1.0, 2.0, 3.0, 10.0, -5.0, 0.5], 42.0);
        let (_, _, p, t) = read(&b);
        assert_eq!(p, [1.0, 2.0, 3.0, 10.0, -5.0, 0.5]);
        assert_eq!(t, 42.0);
        assert!(opentrack_packet(&[f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0]).is_none());
    }
}
