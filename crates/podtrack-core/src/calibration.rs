//! Calibrations.
//!
//! * Appareil (`DeviceCalibration`) : sept étapes (immobile, gauche, retour, haut, retour,
//!   épaule droite, retour) qui donnent le profil d'axes de l'écouteur ([`DeviceProfile`]).
//!   Reprise de `calibrate()` / `profile_from_calibration` (MousePods), avec le correctif de la
//!   première sauvegarde.
//! * Jeu (`GameCalibration`) : cinq points à l'écran (centre, gauche, droite, haut, bas) ; les
//!   angles de tête aux points du bord donnent l'amplitude d'un profil. Reprise de `calibration.py`.
//!
//! Machines à états pures : le temps arrive en paramètre.

use crate::engine::DeviceProfile;
use crate::profile::{Baseline, Range};
use crate::vec::{self, V3};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

// ------------------------------------------------------------------------- appareil

/// Intègre les mesures d'une étape de calibration (repère de l'écouteur).
#[derive(Debug, Clone)]
pub struct CalibAccumulator {
    pub bias: V3,
    pub up: V3,
    pub yaw: f64,
    pub horiz: V3,
}

impl CalibAccumulator {
    pub fn new(bias: V3, g_ref: V3) -> Self {
        Self { bias, up: vec::vnorm(g_ref), yaw: 0.0, horiz: [0.0; 3] }
    }

    pub fn add(&mut self, w: V3, dt: f64) {
        let wc = vec::sub(w, self.bias);
        self.yaw += vec::dot(wc, self.up) * dt;
        let r = vec::reject(wc, self.up);
        for k in 0..3 {
            self.horiz[k] += r[k] * dt;
        }
    }

    pub fn horiz_deg(&self) -> f64 {
        vec::len(self.horiz)
    }
}

/// Profil : gauche = lacet négatif, haut = tangage positif, épaule droite = roulis positif.
pub fn profile_from_calibration(up: V3, yaw_left: f64, up_horiz: V3, roll_right: V3) -> DeviceProfile {
    let lat = vec::vnorm(up_horiz); // lever la tête = rotation autour de l'axe latéral
    let yaw_sign = if yaw_left > 0.0 { -1.0 } else { 1.0 };
    // `fwd` part de l'axe non arrondi ; seul l'axe enregistré est arrondi (comme MousePods)
    let upn = vec::vnorm(up);
    let fwd = vec::cross(upn, vec::vnorm(vec::reject(lat, upn)));
    let roll_sign = if vec::dot(roll_right, fwd) > 0.0 { 1.0 } else { -1.0 };
    let lat = [vec::round_dec(lat[0], 4), vec::round_dec(lat[1], 4), vec::round_dec(lat[2], 4)];
    DeviceProfile { yaw_sign, lat, pitch_sign: 1.0, roll_sign }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStep {
    Still,
    Yaw,
    Back,
    Pitch,
    Roll,
}

pub const DEVICE_STEPS: [DeviceStep; 7] = [
    DeviceStep::Still,
    DeviceStep::Yaw,
    DeviceStep::Back,
    DeviceStep::Pitch,
    DeviceStep::Back,
    DeviceStep::Roll,
    DeviceStep::Back,
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceCalibConfig {
    /// Fenêtre (s) sur laquelle juger l'immobilité.
    pub still_window_s: f64,
    /// Étendue maximale du gyroscope (°/s) dans la fenêtre.
    pub still_range_dps: f64,
    /// Durée d'immobilité à tenir (s).
    pub still_hold_s: f64,
    /// Nombre minimal d'échantillons dans la fenêtre (strictement plus que).
    pub min_samples: usize,
    /// Rotation à atteindre à chaque étape de mouvement (°).
    pub step_deg: f64,
}

impl Default for DeviceCalibConfig {
    fn default() -> Self {
        Self { still_window_s: 0.4, still_range_dps: 6.0, still_hold_s: 1.0, min_samples: 4, step_deg: 20.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceCalibState {
    pub index: usize,
    pub step: Option<DeviceStep>,
    /// Avancement de l'étape en cours (0..1).
    pub progress: f64,
    pub done: bool,
    pub profile: Option<DeviceProfile>,
}

#[derive(Debug, Clone)]
pub struct DeviceCalibration {
    pub cfg: DeviceCalibConfig,
    index: usize,
    t0: f64,
    /// (temps, dt, w, g) reçus depuis le début de l'étape.
    samples: Vec<(f64, f64, V3, V3)>,
    n_used: usize,
    still_since: Option<f64>,
    bias: V3,
    up: V3,
    acc: Option<CalibAccumulator>,
    yaw: f64,
    pitch: V3,
    roll: V3,
    progress: f64,
    profile: Option<DeviceProfile>,
}

impl DeviceCalibration {
    pub fn new(cfg: DeviceCalibConfig, now: f64) -> Self {
        Self {
            cfg,
            index: 0,
            t0: now,
            samples: Vec::new(),
            n_used: 0,
            still_since: None,
            bias: [0.0; 3],
            up: [0.0, 0.0, 1.0],
            acc: None,
            yaw: 0.0,
            pitch: [0.0; 3],
            roll: [0.0; 3],
            progress: 0.0,
            profile: None,
        }
    }

    pub fn restart(&mut self, now: f64) {
        *self = Self::new(self.cfg.clone(), now);
    }

    pub fn feed(&mut self, now: f64, dt: f64, w: V3, g: V3) {
        if self.profile.is_none() && now > self.t0 {
            self.samples.push((now, dt, w, g));
        }
    }

    fn next(&mut self, now: f64) {
        self.index += 1;
        self.t0 = now;
        self.still_since = None;
        self.acc = None;
        self.samples.clear();
        self.n_used = 0;
    }

    /// À appeler régulièrement (≈ 25 Hz) : fait avancer les étapes.
    pub fn tick(&mut self, now: f64) -> DeviceCalibState {
        if self.profile.is_none() && self.index < DEVICE_STEPS.len() {
            let step = DEVICE_STEPS[self.index];
            let c = self.cfg.clone();
            if self.samples.is_empty() {
                self.progress = 0.0;
            } else if matches!(step, DeviceStep::Still | DeviceStep::Back) {
                let recent: Vec<&(f64, f64, V3, V3)> = self.samples.iter().filter(|s| s.0 > now - c.still_window_s).collect();
                let rng = if recent.len() > c.min_samples {
                    (0..3)
                        .map(|k| {
                            let mx = recent.iter().map(|s| s.2[k]).fold(f64::NEG_INFINITY, f64::max);
                            let mn = recent.iter().map(|s| s.2[k]).fold(f64::INFINITY, f64::min);
                            mx - mn
                        })
                        .fold(f64::NEG_INFINITY, f64::max)
                } else {
                    99.0
                };
                if rng < c.still_range_dps {
                    let since = *self.still_since.get_or_insert(now);
                    let held = now - since;
                    self.progress = (held / c.still_hold_s).min(1.0);
                    if held >= c.still_hold_s {
                        if step == DeviceStep::Still {
                            self.bias = vec::mean(recent.iter().map(|s| &s.2).collect::<Vec<_>>());
                            self.up = vec::vnorm(vec::mean(recent.iter().map(|s| &s.3).collect::<Vec<_>>()));
                        }
                        self.next(now);
                    }
                } else {
                    self.still_since = None;
                    self.progress = 0.0;
                }
            } else {
                let (bias, up) = (self.bias, self.up);
                let acc = self.acc.get_or_insert_with(|| CalibAccumulator::new(bias, up));
                for s in &self.samples[self.n_used..] {
                    acc.add(s.2, s.1);
                }
                self.n_used = self.samples.len();
                let mag = if step == DeviceStep::Yaw { acc.yaw.abs() } else { acc.horiz_deg() };
                self.progress = (mag / c.step_deg).min(1.0);
                if mag >= c.step_deg {
                    match step {
                        DeviceStep::Yaw => self.yaw = acc.yaw,
                        DeviceStep::Pitch => self.pitch = acc.horiz,
                        _ => self.roll = acc.horiz,
                    }
                    self.next(now);
                }
            }
            if self.index >= DEVICE_STEPS.len() {
                self.profile = Some(profile_from_calibration(self.up, self.yaw, self.pitch, self.roll));
                self.progress = 1.0;
            }
        }
        self.state()
    }

    pub fn state(&self) -> DeviceCalibState {
        DeviceCalibState {
            index: self.index,
            step: DEVICE_STEPS.get(self.index).copied(),
            progress: self.progress,
            done: self.profile.is_some(),
            profile: self.profile.clone(),
        }
    }
}

// ------------------------------------------------------------------------- jeu (points à l'écran)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Point {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

pub const POINTS: [Point; 5] = [Point::Center, Point::Left, Point::Right, Point::Top, Point::Bottom];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameCalibConfig {
    /// Durée de maintien sur chaque point (s).
    pub hold_s: f64,
    /// Le lacet et le tangage restent chacun dans cet écart pendant le maintien (°).
    pub still_deg: f64,
    /// Un point du bord est au moins à cette distance du centre, de son côté (°).
    pub min_deg: f64,
    /// Sans données pendant cette durée : échec (s).
    pub no_signal_s: f64,
}

impl Default for GameCalibConfig {
    fn default() -> Self {
        Self { hold_s: 2.0, still_deg: 1.5, min_deg: 5.0, no_signal_s: 3.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hint {
    TooClose,
    WrongSide,
    NoSignal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum End {
    Done,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameCalibState {
    pub point: Option<Point>,
    pub end: Option<End>,
    pub progress: f64,
    pub hint: Option<Hint>,
    pub result: Option<Baseline>,
}

#[derive(Debug, Clone)]
pub struct GameCalibration {
    pub cfg: GameCalibConfig,
    index: usize,
    window: VecDeque<(f64, f64, f64)>,
    hint: Option<Hint>,
    edges: [f64; 5],
    end: Option<End>,
    last_seen: Option<f64>,
    now: f64,
    /// Mis à vrai quand le point central est capturé : l'appelant recentre alors le tracker.
    pub center_reached: bool,
}

impl GameCalibration {
    pub fn new(cfg: GameCalibConfig) -> Self {
        Self { cfg, index: 0, window: VecDeque::new(), hint: None, edges: [0.0; 5], end: None, last_seen: None, now: 0.0, center_reached: false }
    }

    pub fn finished(&self) -> bool {
        self.end.is_some()
    }

    pub fn feed(&mut self, now: f64, yaw: f64, pitch: f64) {
        if self.finished() {
            return;
        }
        self.now = now;
        self.last_seen = Some(now);
        self.window.push_back((now, yaw, pitch));
        let span = |w: &VecDeque<(f64, f64, f64)>, f: fn(&(f64, f64, f64)) -> f64| {
            let mx = w.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
            let mn = w.iter().map(f).fold(f64::INFINITY, f64::min);
            mx - mn
        };
        while !self.window.is_empty()
            && (span(&self.window, |s| s.1) >= self.cfg.still_deg || span(&self.window, |s| s.2) >= self.cfg.still_deg)
        {
            self.window.pop_front(); // la tête a bougé : le maintien repart des derniers échantillons
        }
        if let Some(first) = self.window.front() {
            if now - first.0 >= self.cfg.hold_s {
                let n = self.window.len() as f64;
                let y = self.window.iter().fold(0.0, |a, s| a + s.1) / n;
                let p = self.window.iter().fold(0.0, |a, s| a + s.2) / n;
                self.capture(y, p);
                self.window.clear();
            }
        }
    }

    pub fn tick(&mut self, now: f64) {
        if self.finished() {
            return;
        }
        self.now = now;
        match self.last_seen {
            None => self.last_seen = Some(now),
            Some(l) if now - l > self.cfg.no_signal_s => {
                self.end = Some(End::Failed);
                self.hint = Some(Hint::NoSignal);
            }
            _ => {}
        }
    }

    pub fn cancel(&mut self) {
        if !self.finished() {
            self.end = Some(End::Cancelled);
        }
    }

    fn capture(&mut self, yaw: f64, pitch: f64) {
        let point = POINTS[self.index];
        if point == Point::Center {
            self.center_reached = true;
        } else {
            let value = match point {
                Point::Left => -yaw,
                Point::Right => yaw,
                Point::Top => pitch,
                _ => -pitch,
            };
            if value < 0.0 && value.abs() >= self.cfg.min_deg {
                self.hint = Some(Hint::WrongSide);
                return;
            }
            if value < self.cfg.min_deg {
                self.hint = Some(Hint::TooClose);
                return;
            }
            self.edges[self.index] = value;
        }
        self.hint = None;
        self.index += 1;
        if self.index == POINTS.len() {
            self.end = Some(End::Done);
        }
    }

    pub fn state(&self) -> GameCalibState {
        let progress = match (self.finished(), self.window.front()) {
            (false, Some(f)) => ((self.now - f.0) / self.cfg.hold_s).min(1.0),
            _ => 0.0,
        };
        let result = (self.end == Some(End::Done)).then(|| Baseline {
            yaw: Range { neg: self.edges[1], pos: self.edges[2] },
            pitch: Range { neg: self.edges[4], pos: self.edges[3] },
            ..Default::default()
        });
        GameCalibState {
            point: if self.finished() { None } else { Some(POINTS[self.index]) },
            end: self.end,
            progress: vec::round_dec(progress, 3),
            hint: self.hint,
            result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &mut GameCalibration, t: &mut f64, yaw: f64, pitch: f64, secs: f64) {
        let n = (secs * 100.0) as usize;
        for _ in 0..n {
            *t += 0.01;
            c.feed(*t, yaw, pitch);
            c.tick(*t);
        }
    }

    #[test]
    fn full_run() {
        let mut c = GameCalibration::new(GameCalibConfig::default());
        let mut t = 0.0;
        run(&mut c, &mut t, 0.0, 0.0, 2.1);
        assert!(c.center_reached);
        run(&mut c, &mut t, -20.0, 0.0, 2.1);
        run(&mut c, &mut t, 30.0, 0.0, 2.1);
        run(&mut c, &mut t, 0.0, 12.0, 2.1);
        run(&mut c, &mut t, 0.0, -9.0, 2.1);
        let s = c.state();
        assert_eq!(s.end, Some(End::Done));
        let r = s.result.unwrap();
        assert_eq!((r.yaw.neg, r.yaw.pos, r.pitch.pos, r.pitch.neg), (20.0, 30.0, 12.0, 9.0));
    }

    #[test]
    fn hints_and_no_signal() {
        let mut c = GameCalibration::new(GameCalibConfig::default());
        let mut t = 0.0;
        run(&mut c, &mut t, 0.0, 0.0, 2.1);
        run(&mut c, &mut t, -3.0, 0.0, 2.1);
        assert_eq!(c.state().hint, Some(Hint::TooClose));
        run(&mut c, &mut t, 10.0, 0.0, 2.1);
        assert_eq!(c.state().hint, Some(Hint::WrongSide));
        run(&mut c, &mut t, -15.0, 0.0, 2.1);
        assert_eq!(c.state().point, Some(Point::Right));
        for _ in 0..320 {
            t += 0.01;
            c.tick(t);
        }
        assert_eq!(c.state().end, Some(End::Failed));
        assert_eq!(c.state().hint, Some(Hint::NoSignal));
    }

    #[test]
    fn progress_half_after_one_second() {
        let mut c = GameCalibration::new(GameCalibConfig::default());
        let mut t = 0.0;
        run(&mut c, &mut t, 0.0, 0.0, 1.0);
        assert!((c.state().progress - 0.5).abs() <= 0.02);
    }
}
