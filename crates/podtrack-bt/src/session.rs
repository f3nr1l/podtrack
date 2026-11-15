//! Session AACP avec des AirPods : choix de l'appareil, ouverture du canal, poignée de main,
//! démarrage des flux capteurs (SensorDataWX ou RemoteHID), décodage des paquets.

use crate::bluez::{AppleDevice, Bluez};
use crate::l2cap::L2cap;
use anyhow::{Result, bail};
use podtrack_core::protocol::{self, Encoder, Event};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BtConfig {
    /// Adresse des AirPods, ou `auto` (premiers AirPods appairés et connectés).
    pub address: String,
    /// Connecter les AirPods appairés s'ils ne le sont pas.
    pub auto_connect: bool,
    /// Intervalle des échantillons (µs) : 10000 = 100 Hz.
    pub sensor_interval_us: u32,
    /// Échelle du gyroscope.
    pub gyro_dps_per_lsb: f64,
    pub psm: u16,
    /// Niveau BT_SECURITY (2 = medium, lien chiffré).
    pub security: u8,
    /// MTU de réception L2CAP.
    pub imtu: u16,
    pub l2cap_timeout_s: f64,
    pub bt_connect_timeout_s: f64,
    pub bt_probe_timeout_s: f64,
    /// Pauses après la poignée de main, les fonctions et les notifications (s).
    pub handshake_pauses_s: [f64; 3],
    /// Flux « activité » envoyé avant le flux de mouvement (µs) et pause qui suit (s).
    pub activity_warmup_us: u32,
    pub activity_pause_s: f64,
    /// Sans données depuis ce délai, les flux sont relancés (s) ; dans le boîtier, délai plus long.
    pub relaunch_s: f64,
    pub relaunch_case_s: f64,
    /// Sans données malgré les relances pendant ce délai, le canal est fermé puis rouvert (s ; 0 = jamais).
    pub reopen_after_s: f64,
    /// Après ce nombre de réouvertures sans données, déconnexion puis reconnexion Bluetooth
    /// (les écouteurs cessent parfois d'annoncer leurs capteurs ; coupe l'audio un instant ; 0 = jamais).
    pub bt_reset_after_reopens: u32,
    /// Attente entre deux tentatives de connexion : double de min à max (s).
    pub reconnect_min_s: f64,
    pub reconnect_max_s: f64,
    /// Recevoir les appuis sur la tige.
    pub stem_events: bool,
    pub close_pause_s: f64,
}

impl Default for BtConfig {
    fn default() -> Self {
        Self {
            address: "auto".into(),
            auto_connect: true,
            sensor_interval_us: 10_000,
            gyro_dps_per_lsb: 0.122,
            psm: 0x1001,
            security: 2,
            imtu: 2582,
            l2cap_timeout_s: 8.0,
            bt_connect_timeout_s: 20.0,
            bt_probe_timeout_s: 12.0,
            handshake_pauses_s: [0.3, 0.15, 0.15],
            activity_warmup_us: 40_000,
            activity_pause_s: 0.25,
            relaunch_s: 4.0,
            relaunch_case_s: 20.0,
            reopen_after_s: 30.0,
            bt_reset_after_reopens: 2,
            reconnect_min_s: 2.0,
            reconnect_max_s: 20.0,
            stem_events: false,
            close_pause_s: 0.05,
        }
    }
}

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s.max(0.0))
}

#[derive(Debug)]
pub struct NoAirPods;
impl std::fmt::Display for NoAirPods {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "aucun AirPods connecté")
    }
}
impl std::error::Error for NoAirPods {}

/// Choisit l'appareil : adresse configurée, sinon l'appareil connu s'il est connecté, sinon le
/// premier Apple appairé connecté, sinon (auto_connect) le premier qui accepte la connexion.
pub fn pick(bz: &Bluez, cfg: &BtConfig, hint: Option<&str>) -> Result<AppleDevice> {
    if cfg.address != "auto" {
        return bz.device(&cfg.address)?.ok_or_else(|| anyhow::anyhow!("{} n'est pas un appareil Apple appairé", cfg.address));
    }
    let devs = bz.apple_devices()?;
    if let Some(h) = hint {
        if let Some(d) = devs.iter().find(|d| d.address.eq_ignore_ascii_case(h) && d.connected) {
            return Ok(d.clone());
        }
    }
    if let Some(d) = devs.iter().find(|d| d.connected) {
        return Ok(d.clone());
    }
    if cfg.auto_connect {
        for d in &devs {
            log::info!("tentative de connexion : {} ({})", d.name, d.address);
            if bz.connect(d, secs(cfg.bt_probe_timeout_s)) {
                return Ok(d.clone());
            }
        }
    }
    Err(NoAirPods.into())
}

pub struct Session {
    link: L2cap,
    enc: Encoder,
    pub device: AppleDevice,
    pub cfg: BtConfig,
    rhid_devs: HashMap<String, u8>,
    motion_dev: Option<u8>,
}

impl Session {
    pub fn open(bz: &Bluez, cfg: &BtConfig, hint: Option<&str>) -> Result<Session> {
        let mut dev = pick(bz, cfg, hint)?;
        if cfg.auto_connect && !dev.connected {
            bz.connect(&dev, secs(cfg.bt_connect_timeout_s));
            if let Ok(Some(d)) = bz.device(&dev.address) {
                dev = d;
            }
        }
        let link = L2cap::connect(&dev.address, cfg.psm, cfg.security, cfg.imtu, secs(cfg.l2cap_timeout_s))?;
        let mut s = Session { link, enc: Encoder::default(), device: dev, cfg: cfg.clone(), rhid_devs: HashMap::new(), motion_dev: None };
        let p = cfg.handshake_pauses_s;
        for (pkt, pause) in [
            (&protocol::PKT_HANDSHAKE[..], p[0]),
            (&protocol::PKT_FEATURES[..], p[1]),
            (&protocol::PKT_NOTIFICATIONS[..], p[2]),
        ] {
            s.link.send(pkt)?;
            sleep(secs(pause));
        }
        let hc = s.enc.rhid_host_connect();
        s.send(&hc);
        if cfg.stem_events {
            s.send(&protocol::stem_events(true));
        }
        log::info!("canal AAP ouvert avec {} ({}, modèle {})", s.device.address, protocol::model_name(&s.device.pid), s.device.pid);
        Ok(s)
    }

    pub fn send(&self, pkt: &[u8]) {
        log::debug!("émis {}", hex(pkt));
        if let Err(e) = self.link.send(pkt) {
            log::warn!("envoi AAP impossible : {e}");
        }
    }

    fn rhid_start(&mut self) {
        if let Some(&dev) = self.rhid_devs.get("devmotion6") {
            let p = self.enc.rhid_set_interval(dev, self.cfg.sensor_interval_us);
            self.send(&p);
            self.motion_dev = Some(dev);
        }
    }

    /// Démarre (ou relance) le flux de mouvement.
    pub fn start_tracking(&mut self) {
        if protocol::is_remotehid_only(&self.device.pid) && self.rhid_devs.is_empty() {
            let p = self.enc.rhid_host_connect();
            self.send(&p);
            return;
        }
        if !self.rhid_devs.is_empty() {
            if self.rhid_devs.contains_key("devmotion6") {
                self.rhid_start();
            } else {
                let p = self.enc.rhid_host_connect();
                self.send(&p);
            }
            return;
        }
        let p = self.enc.stream(protocol::SVC_ACTIVITY, self.cfg.activity_warmup_us);
        self.send(&p);
        sleep(secs(self.cfg.activity_pause_s));
        let p = self.enc.stream(protocol::SVC_DEVMOTION6, self.cfg.sensor_interval_us);
        self.send(&p);
        let p = self.enc.stream(protocol::SVC_ACTIVITY, 0);
        self.send(&p);
        let p = self.enc.rhid_host_connect();
        self.send(&p);
    }

    /// Attend et décode un paquet (au plus `timeout`).
    pub fn poll(&mut self, timeout: Duration) -> Result<Vec<Event>> {
        let Some(pkt) = self.link.recv(timeout)? else { return Ok(Vec::new()) };
        if log::log_enabled!(log::Level::Debug) && !pkt.starts_with(&protocol::WX_PREFIX) {
            log::debug!("reçu {} octets : {}", pkt.len(), hex(&pkt[..pkt.len().min(48)]));
        }
        let events = protocol::decode(&pkt, self.motion_dev);
        for e in &events {
            if let Event::RhidAnnounce { dev, service } = e {
                self.rhid_devs.insert(service.clone(), *dev);
                if service == "devmotion6" {
                    log::info!("capteur devmotion6 annoncé (RemoteHID {dev}) : démarrage");
                    self.rhid_start();
                }
            }
        }
        Ok(events)
    }

    pub fn close(mut self) {
        if let Some(dev) = self.motion_dev {
            let p = self.enc.rhid_set_interval(dev, 0);
            self.send(&p);
        }
        let p = self.enc.stream(protocol::SVC_DEVMOTION6, 0);
        self.send(&p);
        if self.cfg.stem_events {
            self.send(&protocol::stem_events(false));
        }
        sleep(secs(self.cfg.close_pause_s));
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|c| format!("{c:02x}")).collect()
}

/// Délai avant la prochaine tentative de connexion.
pub fn next_backoff(cur: f64, cfg: &BtConfig) -> f64 {
    if cur <= 0.0 { cfg.reconnect_min_s } else { (cur * 2.0).min(cfg.reconnect_max_s) }
}

pub fn ensure_supported(dev: &AppleDevice) -> Result<()> {
    if matches!(dev.pid.as_str(), "2002" | "200f") {
        bail!("{} n'a pas de capteurs de mouvement", protocol::model_name(&dev.pid));
    }
    Ok(())
}
