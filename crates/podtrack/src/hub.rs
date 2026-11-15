//! État partagé du démon : moteur, tracker, sorties, calibrations. Le fil Bluetooth y verse les
//! paquets, le fil de sortie y lit la pose à cadence fixe, D-Bus y envoie les commandes.

use crate::config::{self, Config, TrackerChoice};
use podtrack_core::calibration::{DeviceCalibState, DeviceCalibration, GameCalibState, GameCalibration};
use podtrack_core::engine::{MotionEngine, SampleClock};
use podtrack_core::profile::ProfileFilter;
use podtrack_core::protocol::{self, BatteryPart, Event, Place, Sample, Stem};
use podtrack_core::tracker::{HeadTracker, Pose6, Used};
use podtrack_core::vec::V3;
use podtrack_out::posefile::{self, PoseFile};
use podtrack_out::udp::OpentrackSender;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Instant, SystemTime};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "detail")]
pub enum Conn {
    Stopped,
    Searching(String),
    Connected,
    Receiving,
    NoData,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DeviceInfo {
    pub address: String,
    pub name: String,
    pub pid: String,
    pub model: String,
}

struct BridgeGame {
    pf: PoseFile,
    filter: ProfileFilter,
    last: Pose6,
}

pub struct State {
    pub cfg: Config,
    pub cfg_path: PathBuf,
    cfg_mtime: Option<SystemTime>,
    epoch: Instant,
    /// Suivi voulu (démarré / arrêté) ; `quit` termine le démon.
    pub active: bool,
    pub quit: bool,
    pub conn: Conn,
    pub device: Option<DeviceInfo>,
    pub battery: Vec<BatteryPart>,
    pub place: Option<Place>,
    place_cand: Option<(Place, f64)>,
    pub last_stem: Option<String>,
    engine: MotionEngine,
    tracker: HeadTracker,
    clock: SampleClock,
    pub profile_key: Option<String>,
    pub samples: u64,
    pub rate_hz: f64,
    rate_window: Option<(f64, u64)>,
    last_sample: Option<f64>,
    pub rates: V3,
    pub head: V3,
    bridge: BTreeMap<String, BridgeGame>,
    udp: Option<(OpentrackSender, ProfileFilter)>,
    last_tick: Option<f64>,
    pub last_error: Option<String>,
    pub device_calib: Option<DeviceCalibration>,
    pub game_calib: Option<(String, GameCalibration)>,
    /// Une commande a demandé de relancer les flux (changement d'intervalle…).
    pub restart_stream: bool,
}

pub struct Hub {
    pub state: Mutex<State>,
    pub wake: Condvar,
}

fn place_source(p: Place) -> &'static str {
    match p {
        Place::Ear => "head",
        Place::Case => "case",
        Place::Out => "hand",
    }
}

impl Hub {
    pub fn new(cfg_path: PathBuf) -> anyhow::Result<Arc<Hub>> {
        let cfg = config::load(&cfg_path)?;
        let state = State {
            engine: MotionEngine::new(cfg.engine.clone(), Default::default()),
            tracker: HeadTracker::new(cfg.tracker.clone()),
            clock: SampleClock::new(),
            cfg_mtime: config::mtime(&cfg_path),
            cfg,
            cfg_path,
            epoch: Instant::now(),
            active: true,
            quit: false,
            conn: Conn::Stopped,
            device: None,
            battery: Vec::new(),
            place: None,
            place_cand: None,
            last_stem: None,
            profile_key: None,
            samples: 0,
            rate_hz: 0.0,
            rate_window: None,
            last_sample: None,
            rates: [0.0; 3],
            head: [0.0; 3],
            bridge: BTreeMap::new(),
            udp: None,
            last_tick: None,
            last_error: None,
            device_calib: None,
            game_calib: None,
            restart_stream: false,
        };
        let hub = Arc::new(Hub { state: Mutex::new(state), wake: Condvar::new() });
        hub.lock().reconfigure();
        Ok(hub)
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn notify(&self) {
        self.wake.notify_all();
    }
}

impl State {
    pub fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// Applique la configuration : moteur, tracker, profil d'appareil, jeux, opentrack.
    pub fn reconfigure(&mut self) {
        self.engine.cfg = self.cfg.engine.clone();
        self.tracker.configure(self.cfg.tracker.clone());
        self.apply_device_profile();
        self.configure_bridge();
        self.configure_udp();
    }

    fn apply_device_profile(&mut self) {
        let Some(dev) = &self.device else { return };
        let src = place_source(self.place.unwrap_or(Place::Ear));
        let key = format!("{}_{}", dev.pid, src);
        let prof = self.cfg.devices.get(&key).cloned();
        if self.profile_key.as_deref() != Some(key.as_str()) || prof.as_ref() != Some(self.engine.profile()) {
            if prof.is_none() {
                log::warn!("profil {key} non calibré : axes par défaut (lance une calibration)");
            }
            self.engine.set_profile(prof.unwrap_or_default());
            self.profile_key = Some(key);
        }
    }

    pub fn pose_path(&self, game_id: &str) -> anyhow::Result<PathBuf> {
        if !posefile::safe_id(game_id) {
            anyhow::bail!("identifiant de jeu invalide : {game_id}");
        }
        let dir = posefile::private_dir(&self.cfg.output.shm_dir, &self.cfg.output.shm_name)?;
        Ok(dir.join(format!("pose-{game_id}")))
    }

    fn configure_bridge(&mut self) {
        let baseline = self.cfg.baseline.clone().and_then(|b| b.normalized());
        let wanted: BTreeMap<String, _> = self
            .cfg
            .games
            .iter()
            .filter(|(_, g)| g.enabled && g.tracker == TrackerChoice::Podtrack)
            .map(|(id, g)| (id.clone(), g.profile.clone().normalized().effective(baseline.as_ref())))
            .collect();
        let gone: Vec<String> = self.bridge.keys().filter(|k| !wanted.contains_key(*k)).cloned().collect();
        for id in gone {
            if let Some(b) = self.bridge.remove(&id) {
                b.pf.remove();
            }
        }
        for (id, profile) in wanted {
            if let Some(b) = self.bridge.get_mut(&id) {
                if b.filter.profile != profile.clone().normalized() {
                    b.filter = ProfileFilter::new(profile);
                }
                continue;
            }
            match self.pose_path(&id).and_then(|p| PoseFile::open(&p)) {
                Ok(pf) => {
                    self.bridge.insert(id, BridgeGame { pf, filter: ProfileFilter::new(profile), last: [0.0; 6] });
                }
                Err(e) => {
                    log::error!("jeu {id} : fichier de pose impossible : {e}");
                    self.last_error = Some(format!("jeu {id} : {e}"));
                }
            }
        }
    }

    fn configure_udp(&mut self) {
        let o = &self.cfg.opentrack;
        if !o.enabled {
            self.udp = None;
            return;
        }
        match &mut self.udp {
            Some((s, f)) => {
                s.set_target(&o.host, o.port);
                if f.profile != o.profile.clone().normalized() {
                    *f = ProfileFilter::new(o.profile.clone());
                }
            }
            None => match OpentrackSender::new(&o.host, o.port) {
                Ok(s) => self.udp = Some((s, ProfileFilter::new(o.profile.clone()))),
                Err(e) => self.last_error = Some(format!("opentrack : {e}")),
            },
        }
    }

    /// Relit la configuration si le fichier a changé. Renvoie vrai si c'est le cas.
    pub fn reload_if_changed(&mut self) -> bool {
        let m = config::mtime(&self.cfg_path);
        if m == self.cfg_mtime {
            return false;
        }
        self.cfg_mtime = m;
        match config::load(&self.cfg_path) {
            Ok(c) => {
                let interval_changed = c.bluetooth.sensor_interval_us != self.cfg.bluetooth.sensor_interval_us;
                self.cfg = c;
                self.reconfigure();
                self.restart_stream |= interval_changed;
                log::info!("configuration rechargée");
                true
            }
            Err(e) => {
                log::warn!("configuration illisible, ancienne gardée : {e:#}");
                self.last_error = Some(format!("{e:#}"));
                false
            }
        }
    }

    /// Enregistre la configuration en mémoire (et note la date pour ne pas la relire).
    pub fn save_config(&mut self) -> anyhow::Result<()> {
        config::save(&self.cfg_path, &self.cfg)?;
        self.cfg_mtime = config::mtime(&self.cfg_path);
        self.reconfigure();
        Ok(())
    }

    pub fn set_device(&mut self, dev: Option<DeviceInfo>) {
        self.device = dev;
        self.apply_device_profile();
    }

    pub fn tracker_suspend_autocenter(&mut self, on: bool) {
        self.tracker.suspend_autocenter = on;
    }

    pub fn recenter(&mut self) {
        self.tracker.recenter();
        self.head = [0.0; 3];
    }

    pub fn stale(&self, now: f64) -> bool {
        match self.last_sample {
            None => true,
            Some(t) => now - t > self.cfg.output.stale_after_s,
        }
    }

    /// Paquets décodés : capteurs, oreille, tige, batterie.
    pub fn on_events(&mut self, events: Vec<Event>, now: f64) {
        for e in events {
            match e {
                Event::Motion(blocks) => {
                    for blk in blocks {
                        self.on_block(&blk, now);
                    }
                }
                Event::Place(p) => self.on_place(p, now),
                Event::Battery(b) if !b.is_empty() => self.battery = b,
                Event::Stem(k, raw) => self.on_stem(k, raw),
                _ => {}
            }
        }
    }

    fn on_place(&mut self, p: Place, now: f64) {
        match self.place {
            None => {
                self.place = Some(p); // premier état : appliqué tout de suite
                self.apply_device_profile();
            }
            Some(cur) if cur == p => self.place_cand = None,
            Some(_) => match self.place_cand {
                Some((c, since)) if c == p => {
                    if now - since >= self.cfg.place.debounce_s {
                        self.place = Some(p);
                        self.place_cand = None;
                        self.recenter();
                        self.apply_device_profile();
                    }
                }
                _ => self.place_cand = Some((p, now)),
            },
        }
    }

    fn on_stem(&mut self, k: Option<Stem>, raw: u8) {
        let name = k.map(|s| format!("{s:?}").to_lowercase()).unwrap_or(format!("0x{raw:02x}"));
        log::info!("pression tige : {name}");
        self.last_stem = Some(name);
    }

    fn on_block(&mut self, blk: &[u8], now: f64) {
        let Some(s) = Sample::parse(blk, self.cfg.bluetooth.gyro_dps_per_lsb) else { return };
        let dt = self.clock.dt(s.ts_ns, now);
        self.samples += 1;
        // fréquence : échantillons comptés sur au moins une seconde (plusieurs blocs arrivent par paquet)
        let (t0, n0) = *self.rate_window.get_or_insert((now, self.samples));
        if now - t0 >= 1.0 {
            self.rate_hz = (self.samples - n0) as f64 / (now - t0);
            self.rate_window = Some((now, self.samples));
        }
        self.last_sample = Some(now);
        let rates = self.engine.step(s.w, s.g, dt);
        self.rates = rates;
        self.tracker.update(rates, dt, self.engine.g, Some(self.engine.lat()), self.engine.signs(), Some(s.att), now);
        self.head = self.tracker.head;
        if let Some(c) = self.device_calib.as_mut() {
            c.feed(now, dt, s.w, s.g);
        }
        if let Some((_, c)) = self.game_calib.as_mut() {
            c.feed(now, self.head[0], self.head[1]);
            if c.center_reached {
                c.center_reached = false;
                self.tracker.recenter(); // les angles suivants partent du centre
            }
        }
    }

    /// Fil de sortie, à cadence fixe : poses des jeux et UDP.
    pub fn tick_outputs(&mut self, now: f64) {
        if let Some((_, c)) = self.game_calib.as_mut() {
            c.tick(now);
        }
        let dt = match self.last_tick {
            None => 0.0,
            Some(t) => (now - t).clamp(0.0, self.cfg.output.max_dt),
        };
        self.last_tick = Some(now);
        // arrêté ou sans données : on ne rafraîchit plus, les DLL recentrent d'elles-mêmes
        if !self.active || self.stale(now) {
            return;
        }
        let head: Pose6 = [0.0, 0.0, 0.0, self.head[0], self.head[1], self.head[2]];
        for b in self.bridge.values_mut() {
            let p = b.filter.apply(head, dt);
            b.pf.write(&p, now);
            b.last = p;
        }
        if let Some((s, f)) = self.udp.as_mut() {
            let p = f.apply(head, dt);
            s.send(&p);
        }
    }

    pub fn device_calib_tick(&mut self, now: f64) -> Option<DeviceCalibState> {
        let st = self.device_calib.as_mut()?.tick(now);
        if st.done {
            if let (Some(prof), Some(key)) = (st.profile.clone(), self.profile_key.clone()) {
                self.cfg.devices.insert(key.clone(), prof);
                if let Err(e) = self.save_config() {
                    self.last_error = Some(format!("enregistrement de {key} : {e:#}"));
                }
                log::info!("profil {key} enregistré");
            }
            self.device_calib = None;
        }
        Some(st)
    }

    pub fn game_calib_state(&self) -> Option<(String, GameCalibState)> {
        self.game_calib.as_ref().map(|(t, c)| (t.clone(), c.state()))
    }

    pub fn status(&self) -> serde_json::Value {
        let now = self.now();
        let games: BTreeMap<&String, serde_json::Value> = self
            .bridge
            .iter()
            .map(|(id, b)| (id, serde_json::json!({ "pose": b.last, "seq": b.pf.seq(), "file": b.pf.path })))
            .collect();
        serde_json::json!({
            "active": self.active,
            "connection": self.conn,
            "device": self.device,
            "battery": self.battery.iter().map(|b| serde_json::json!({"part": b.part, "level": b.level, "state": b.state})).collect::<Vec<_>>(),
            "place": self.place,
            "profile": self.profile_key,
            "profile_calibrated": self.profile_key.as_ref().is_some_and(|k| self.cfg.devices.contains_key(k)),
            "samples": self.samples,
            "rate_hz": (self.rate_hz * 10.0).round() / 10.0,
            "data_age_s": self.last_sample.map(|t| now - t),
            "stale": self.stale(now),
            "bias_ok": self.engine.bias_ok,
            "source": match self.tracker.used { Some(Used::Quaternion) => "quaternion", Some(Used::Gyro) => "gyro", None => "" },
            "rates": self.rates,
            "head": self.head,
            "games": games,
            "opentrack": self.udp.as_ref().map(|(s, _)| serde_json::json!({"sent": s.sent, "errors": s.errors})),
            "last_stem": self.last_stem,
            "last_error": self.last_error,
            "device_calibration": self.device_calib.as_ref().map(|c| c.state()),
            "game_calibration": self.game_calib_state().map(|(t, s)| serde_json::json!({"target": t, "state": s})),
        })
    }
}

pub fn model_info(d: &podtrack_bt::bluez::AppleDevice) -> DeviceInfo {
    DeviceInfo { address: d.address.clone(), name: d.name.clone(), pid: d.pid.clone(), model: protocol::model_name(&d.pid) }
}
