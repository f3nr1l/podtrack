//! Sortie opentrack « UDP over network » : 6 doubles, cadence fixe gérée par l'appelant.

use podtrack_core::pose::opentrack_packet;
use podtrack_core::tracker::Pose6;
use std::net::UdpSocket;

pub struct OpentrackSender {
    sock: UdpSocket,
    target: String,
    pub sent: u64,
    pub errors: u64,
}

impl OpentrackSender {
    pub fn new(host: &str, port: u16) -> std::io::Result<Self> {
        Ok(Self { sock: UdpSocket::bind("0.0.0.0:0")?, target: format!("{host}:{port}"), sent: 0, errors: 0 })
    }

    pub fn set_target(&mut self, host: &str, port: u16) {
        self.target = format!("{host}:{port}");
    }

    pub fn send(&mut self, pose: &Pose6) {
        match opentrack_packet(pose).map(|b| self.sock.send_to(&b, &self.target)) {
            Some(Ok(_)) => self.sent += 1,
            _ => self.errors += 1,
        }
    }
}
