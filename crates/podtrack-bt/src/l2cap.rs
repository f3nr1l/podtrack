//! Socket L2CAP SEQPACKET (BR/EDR) par la libc : sécurité « medium » (lien chiffré, exigé pour
//! les capteurs) et MTU de réception élevé (avec 672, les AirPods n'annoncent pas leurs capteurs).

use anyhow::{Result, bail};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

const AF_BLUETOOTH: libc::c_int = 31;
const BTPROTO_L2CAP: libc::c_int = 0;
const SOL_BLUETOOTH: libc::c_int = 274;
const BT_SECURITY: libc::c_int = 4;
const SOL_L2CAP: libc::c_int = 6;
const L2CAP_OPTIONS: libc::c_int = 1;

#[repr(C)]
struct SockaddrL2 {
    family: u16,
    psm: u16,
    bdaddr: [u8; 6],
    cid: u16,
    bdaddr_type: u8,
}

/// `AA:BB:CC:DD:EE:FF` → octets inversés (bdaddr_t est petit-boutiste).
pub fn parse_bdaddr(s: &str) -> Result<[u8; 6]> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 {
        bail!("adresse Bluetooth invalide : {s}");
    }
    let mut b = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        b[5 - i] = u8::from_str_radix(p, 16)?;
    }
    Ok(b)
}

fn errno() -> std::io::Error {
    std::io::Error::last_os_error()
}

pub struct L2cap {
    fd: OwnedFd,
}

impl L2cap {
    /// Ouvre le canal. `security` : niveau BT_SECURITY (2 = medium) ; `imtu` : MTU de réception.
    /// Les réglages sont tentés sans bloquer la connexion s'ils échouent (comme MousePods).
    pub fn connect(addr: &str, psm: u16, security: u8, imtu: u16, timeout: Duration) -> Result<Self> {
        let raw = unsafe { libc::socket(AF_BLUETOOTH, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, BTPROTO_L2CAP) };
        if raw < 0 {
            bail!("socket L2CAP : {}", errno());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let s = fd.as_raw_fd();
        unsafe {
            let sec: [u8; 2] = [security, 0];
            if libc::setsockopt(s, SOL_BLUETOOTH, BT_SECURITY, sec.as_ptr().cast(), 2) == 0 {
                let mut opts = [0u8; 12];
                let mut len: libc::socklen_t = 12;
                if libc::getsockopt(s, SOL_L2CAP, L2CAP_OPTIONS, opts.as_mut_ptr().cast(), &mut len) == 0 {
                    opts[2..4].copy_from_slice(&imtu.to_le_bytes());
                    if libc::setsockopt(s, SOL_L2CAP, L2CAP_OPTIONS, opts.as_ptr().cast(), len) != 0 {
                        log::warn!("MTU L2CAP non réglé : {}", errno());
                    }
                }
            } else {
                log::warn!("sécurité L2CAP non réglée : {}", errno());
            }
        }
        let sa = SockaddrL2 { family: AF_BLUETOOTH as u16, psm: psm.to_le(), bdaddr: parse_bdaddr(addr)?, cid: 0, bdaddr_type: 0 };
        unsafe {
            let fl = libc::fcntl(s, libc::F_GETFL);
            libc::fcntl(s, libc::F_SETFL, fl | libc::O_NONBLOCK);
            let r = libc::connect(s, (&sa as *const SockaddrL2).cast(), std::mem::size_of::<SockaddrL2>() as libc::socklen_t);
            if r != 0 {
                let e = errno();
                if e.raw_os_error() != Some(libc::EINPROGRESS) {
                    bail!("connexion L2CAP à {addr} : {e}");
                }
                let mut p = libc::pollfd { fd: s, events: libc::POLLOUT, revents: 0 };
                let n = libc::poll(&mut p, 1, timeout.as_millis() as libc::c_int);
                if n <= 0 {
                    bail!("connexion L2CAP à {addr} : délai dépassé");
                }
                let mut err: libc::c_int = 0;
                let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                libc::getsockopt(s, libc::SOL_SOCKET, libc::SO_ERROR, (&mut err as *mut libc::c_int).cast(), &mut len);
                if err != 0 {
                    bail!("connexion L2CAP à {addr} : {}", std::io::Error::from_raw_os_error(err));
                }
            }
            libc::fcntl(s, libc::F_SETFL, fl & !libc::O_NONBLOCK);
        }
        Ok(Self { fd })
    }

    pub fn send(&self, pkt: &[u8]) -> std::io::Result<()> {
        let n = unsafe { libc::send(self.fd.as_raw_fd(), pkt.as_ptr().cast(), pkt.len(), libc::MSG_NOSIGNAL) };
        if n < 0 { Err(errno()) } else { Ok(()) }
    }

    /// Attend un paquet au plus `timeout`. `Ok(None)` : rien reçu ; erreur : canal perdu.
    pub fn recv(&self, timeout: Duration) -> std::io::Result<Option<Vec<u8>>> {
        let s = self.fd.as_raw_fd();
        let mut p = libc::pollfd { fd: s, events: libc::POLLIN, revents: 0 };
        let n = unsafe { libc::poll(&mut p, 1, timeout.as_millis() as libc::c_int) };
        if n < 0 {
            let e = errno();
            return if e.kind() == std::io::ErrorKind::Interrupted { Ok(None) } else { Err(e) };
        }
        if n == 0 {
            return Ok(None);
        }
        let mut buf = vec![0u8; 8192];
        let r = unsafe { libc::recv(s, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if r < 0 {
            return Err(errno());
        }
        if r == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "canal fermé"));
        }
        buf.truncate(r as usize);
        Ok(Some(buf))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bdaddr_is_reversed() {
        assert_eq!(super::parse_bdaddr("12:34:56:78:9A:BC").unwrap(), [0xBC, 0x9A, 0x78, 0x56, 0x34, 0x12]);
        assert_eq!(std::mem::size_of::<super::SockaddrL2>(), 14);
    }
}
