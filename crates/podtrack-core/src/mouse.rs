//! Souris par la tête : vitesses de la tête → mouvements du curseur, clics par inclinaison ou
//! immobilité, défilement. Repris de `Controller.on_rates` / `render` (MousePods), modes
//! « souris » (relatif, avec accélération) et « pointeur » (absolu, la direction visée = position).
//!
//! Pur : le temps arrive en paramètre ; les sorties sont des actions que l'appelant applique.

use crate::vec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Relatif : la vitesse de la tête déplace le curseur.
    #[default]
    Souris,
    /// Absolu : la direction visée donne la position à l'écran.
    Pointeur,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourisConfig {
    pub px_per_deg: f64,
    /// Accélération : plus la tête tourne vite, plus le gain augmente.
    pub acceleration_enabled: bool,
    pub acceleration: f64,
    /// Courbes de réponse horizontale et verticale : vitesse de sortie = réf · (v / réf)^courbe.
    /// 1 = linéaire ; plus grand = petits mouvements plus précis, grands plus rapides.
    pub curve_x: f64,
    pub curve_y: f64,
    /// Vitesse de tête (°/s) où la courbe vaut la ligne droite.
    pub curve_ref_dps: f64,
    /// Vitesse de référence de l'accélération (°/s) et plafond du facteur.
    pub accel_ref_dps: f64,
    pub accel_cap: f64,
    pub rate_deadzone_dps: f64,
    /// Trou dans le flux : on continue sur la lancée ce délai, puis on freine sur `fade_s`.
    pub coast_s: f64,
    pub fade_s: f64,
}

impl Default for SourisConfig {
    fn default() -> Self {
        Self { px_per_deg: 20.0, acceleration_enabled: true, acceleration: 1.0, curve_x: 1.0, curve_y: 1.0, curve_ref_dps: 30.0, accel_ref_dps: 60.0, accel_cap: 2.5, rate_deadzone_dps: 2.5, coast_s: 0.08, fade_s: 0.1 }
    }
}

/// Amplitudes de visée mesurées en regardant les coins de l'écran (°).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct AimRange {
    pub x_neg: f64,
    pub x_pos: f64,
    pub y_pos: f64,
    pub y_neg: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointeurConfig {
    /// Sans calibration des coins : pourcentage de l'écran par degré.
    pub screen_pct_per_deg: f64,
    /// Rapport vertical / horizontal sans calibration.
    pub vertical_ratio: f64,
    pub rate_deadzone_dps: f64,
    /// Amplitudes calibrées (coins) ; absentes = `screen_pct_per_deg`.
    pub range: Option<AimRange>,
    /// Retard du lissage de position (s).
    pub render_lag_s: f64,
}

impl Default for PointeurConfig {
    fn default() -> Self {
        Self { screen_pct_per_deg: 3.0, vertical_ratio: 1.6, rate_deadzone_dps: 0.6, range: None, render_lag_s: 0.025 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RollClickConfig {
    pub enabled: bool,
    pub trigger_deg: f64,
    pub max_duration_s: f64,
    /// Fuite de l'intégrateur de roulis (s) et seuils d'amorce / de repos (°).
    pub leak_s: f64,
    pub start_deg: f64,
    pub rest_deg: f64,
    pub right: String,
    pub left: String,
}

impl Default for RollClickConfig {
    fn default() -> Self {
        Self { enabled: false, trigger_deg: 15.0, max_duration_s: 0.6, leak_s: 0.4, start_deg: 4.0, rest_deg: 2.0, right: "left_click".into(), left: "right_click".into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DwellConfig {
    pub enabled: bool,
    pub time_s: f64,
    pub radius_px: f64,
    pub min_move_px: f64,
}

impl Default for DwellConfig {
    fn default() -> Self {
        Self { enabled: false, time_s: 1.0, radius_px: 12.0, min_move_px: 60.0 }
    }
}

/// Action d'un appui sur la tige : left_click, right_click, middle_click, toggle_scroll,
/// toggle_pause, recenter, none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StemActions {
    pub single: String,
    pub double: String,
    pub triple: String,
    pub long: String,
}

impl Default for StemActions {
    fn default() -> Self {
        Self { single: "left_click".into(), double: "right_click".into(), triple: "toggle_scroll".into(), long: "toggle_pause".into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MouseConfig {
    /// Souris par la tête active (sinon en pause).
    pub enabled: bool,
    pub mode: Mode,
    pub sensitivity_x: f64,
    pub sensitivity_y: f64,
    pub invert_x: bool,
    pub invert_y: bool,
    /// Lissage des vitesses (0..1) : coupure du 1€ = 8 + (0,6 − 8)·s Hz, bêta `filter_beta`.
    pub smoothing: f64,
    pub filter_beta: f64,
    /// Mettre la souris en pause tant qu'un jeu reçoit la pose.
    pub pause_when_gaming: bool,
    pub souris: SourisConfig,
    pub pointeur: PointeurConfig,
    pub scroll_lines_per_deg: f64,
    pub roll_click: RollClickConfig,
    pub dwell_click: DwellConfig,
    pub stem: StemActions,
    /// Pression d'un clic (s).
    pub click_press_s: f64,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: Mode::Souris,
            sensitivity_x: 1.0,
            sensitivity_y: 1.0,
            invert_x: false,
            invert_y: false,
            smoothing: 0.35,
            filter_beta: 0.02,
            pause_when_gaming: true,
            souris: SourisConfig::default(),
            pointeur: PointeurConfig::default(),
            scroll_lines_per_deg: 0.25,
            roll_click: RollClickConfig::default(),
            dwell_click: DwellConfig::default(),
            stem: StemActions::default(),
            click_press_s: 0.015,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MouseOut {
    /// Déplacement relatif (px, fractions accumulées par l'appelant).
    Rel(f64, f64),
    /// Position absolue (0..1).
    Abs(f64, f64),
    /// Défilement (lignes, fractionnaires).
    Scroll(f64),
    /// Commande nommée (clic, bascule…).
    Command(String),
}

/// Courbe de réponse de la souris appliquée à une vitesse (°/s), signe conservé.
#[inline]
pub fn shape_rate(v: f64, curve: f64, reference: f64) -> f64 {
    if curve == 1.0 || v == 0.0 || reference <= 0.0 {
        return v;
    }
    (reference * (v.abs() / reference).powf(curve)).copysign(v)
}

/// Vitesse du curseur (px/s) pour une vitesse de tête (°/s), axe x ou y, sans inversion :
/// zone morte, courbe, gain et accélération (sur cet axe seul). Sert au graphe de la fenêtre.
pub fn cursor_speed(c: &SourisConfig, v: f64, curve: f64, sensitivity: f64) -> f64 {
    let s = shape_rate(soft_deadzone(v, c.rate_deadzone_dps), curve, c.curve_ref_dps);
    let accel = if c.acceleration_enabled { c.acceleration * (s.abs() / c.accel_ref_dps.max(1e-6)).min(c.accel_cap) } else { 0.0 };
    s * c.px_per_deg * (1.0 + accel) * sensitivity
}

#[inline]
pub fn soft_deadzone(v: f64, dz: f64) -> f64 {
    if v.abs() <= dz { 0.0 } else { (v.abs() - dz).copysign(v) }
}

/// Le 1€ de la souris (formule de MousePods, `OneEuro` du contrôleur).
#[derive(Debug, Clone)]
struct MouseEuro {
    mc: f64,
    beta: f64,
    x: Option<f64>,
    dx: f64,
}

impl MouseEuro {
    fn a(cutoff: f64, dt: f64) -> f64 {
        1.0 / (1.0 + 1.0 / (2.0 * std::f64::consts::PI * cutoff * dt))
    }
    fn apply(&mut self, x: f64, dt: f64) -> f64 {
        let Some(prev) = self.x.filter(|_| dt > 0.0) else {
            self.x = Some(x);
            self.dx = 0.0;
            return x;
        };
        let d = (x - prev) / dt;
        self.dx += Self::a(1.0, dt) * (d - self.dx);
        let n = prev + Self::a(self.mc + self.beta * self.dx.abs(), dt) * (x - prev);
        self.x = Some(n);
        n
    }
}

#[derive(Debug, Clone)]
pub struct MouseEngine {
    pub cfg: MouseConfig,
    fy: MouseEuro,
    fp: MouseEuro,
    pub scroll_mode: bool,
    ang: [f64; 2],
    vel: (f64, f64),
    target: (f64, f64),
    cur: (f64, f64),
    last_sample: f64,
    last_render: Option<f64>,
    roll_ang: f64,
    roll_t0: Option<f64>,
    roll_latched: bool,
    dwell_anchor: Option<(f64, f64)>,
    dwell_t0: f64,
    dwell_moved: f64,
    dwell_fired: bool,
    pos_px: [f64; 2],
}

impl MouseEngine {
    pub fn new(cfg: MouseConfig) -> Self {
        let mut m = Self {
            fy: MouseEuro { mc: 1.0, beta: 0.0, x: None, dx: 0.0 },
            fp: MouseEuro { mc: 1.0, beta: 0.0, x: None, dx: 0.0 },
            cfg,
            scroll_mode: false,
            ang: [0.0; 2],
            vel: (0.0, 0.0),
            target: (0.5, 0.5),
            cur: (0.5, 0.5),
            last_sample: 0.0,
            last_render: None,
            roll_ang: 0.0,
            roll_t0: None,
            roll_latched: false,
            dwell_anchor: None,
            dwell_t0: 0.0,
            dwell_moved: 0.0,
            dwell_fired: true,
            pos_px: [0.0; 2],
        };
        m.configure(m.cfg.clone());
        m
    }

    pub fn configure(&mut self, cfg: MouseConfig) {
        let s = vec::clamp(cfg.smoothing, 0.0, 1.0);
        let mc = 8.0 + (0.6 - 8.0) * s;
        if self.fy.mc != mc || self.fy.beta != cfg.filter_beta {
            self.fy = MouseEuro { mc, beta: cfg.filter_beta, x: None, dx: 0.0 };
            self.fp = MouseEuro { mc, beta: cfg.filter_beta, x: None, dx: 0.0 };
        }
        self.cfg = cfg;
    }

    pub fn recenter(&mut self) {
        self.ang = [0.0; 2];
        self.vel = (0.0, 0.0);
        self.target = (0.5, 0.5);
        self.roll_ang = 0.0;
        self.roll_t0 = None;
        self.roll_latched = false;
    }

    /// Vitesses de la tête (°/s) d'un échantillon. `active` faux : rien ne bouge.
    pub fn on_rates(&mut self, yaw: f64, pitch: f64, roll: f64, dt: f64, now: f64, active: bool, out: &mut Vec<MouseOut>) {
        self.last_sample = now;
        let c = self.cfg.clone();
        let yaw_f = self.fy.apply(yaw, dt);
        let pitch_f = self.fp.apply(pitch, dt);
        let ix = if c.invert_x { -1.0 } else { 1.0 };
        let iy = if c.invert_y { -1.0 } else { 1.0 };
        if !active {
            self.vel = (0.0, 0.0);
            return;
        }
        // roulis : clic par inclinaison rapide (intégrateur à fuite)
        self.roll_ang = self.roll_ang * (-dt / c.roll_click.leak_s.max(1e-3)).exp() + roll * dt;
        if c.roll_click.enabled && self.roll_gesture(now, out) {
            self.vel = (0.0, 0.0);
            return;
        }
        if self.scroll_mode {
            out.push(MouseOut::Scroll(-iy * pitch_f * dt * c.scroll_lines_per_deg * 4.0));
            self.vel = (0.0, 0.0);
            return;
        }
        match c.mode {
            Mode::Souris => {
                let s = &c.souris;
                let vy = shape_rate(soft_deadzone(yaw_f, s.rate_deadzone_dps), s.curve_x, s.curve_ref_dps);
                let vp = shape_rate(soft_deadzone(pitch_f, s.rate_deadzone_dps), s.curve_y, s.curve_ref_dps);
                let speed = vy.hypot(vp);
                let accel = if s.acceleration_enabled { s.acceleration * (speed / s.accel_ref_dps.max(1e-6)).min(s.accel_cap) } else { 0.0 };
                let gain = s.px_per_deg * (1.0 + accel);
                self.vel = (ix * vy * gain * c.sensitivity_x, -iy * vp * gain * c.sensitivity_y);
            }
            Mode::Pointeur => {
                let p = &c.pointeur;
                self.ang[0] += soft_deadzone(yaw_f, p.rate_deadzone_dps) * dt;
                self.ang[1] += soft_deadzone(pitch_f, p.rate_deadzone_dps) * dt;
                if let Some(r) = p.range {
                    let (sx, sy) = (c.sensitivity_x, c.sensitivity_y);
                    let (xl, xr) = (r.x_neg / sx, r.x_pos / sx);
                    let (yu, yd) = (r.y_pos / sy, r.y_neg / sy);
                    self.ang[0] = vec::clamp(self.ang[0], -xl, xr);
                    self.ang[1] = vec::clamp(self.ang[1], -yd, yu);
                    let fx = 0.5 + 0.5 * if self.ang[0] > 0.0 { self.ang[0] / xr } else { self.ang[0] / xl };
                    let fy = 0.5 - 0.5 * if self.ang[1] > 0.0 { self.ang[1] / yu } else { self.ang[1] / yd };
                    self.target = (0.5 + ix * (fx - 0.5), 0.5 + iy * (fy - 0.5));
                } else {
                    let k = p.screen_pct_per_deg / 100.0;
                    let (kx, ky) = (k * c.sensitivity_x, k * c.sensitivity_y * p.vertical_ratio);
                    // pas d'accumulation au-delà des bords
                    let (lx, ly) = (0.5 / kx.max(1e-6), 0.5 / ky.max(1e-6));
                    self.ang[0] = vec::clamp(self.ang[0], -lx, lx);
                    self.ang[1] = vec::clamp(self.ang[1], -ly, ly);
                    self.target = (0.5 + ix * self.ang[0] * kx, 0.5 - iy * self.ang[1] * ky);
                }
            }
        }
        if c.dwell_click.enabled {
            self.dwell(now, out);
        }
    }

    /// À cadence fixe : déplacement du curseur depuis le dernier rendu.
    pub fn render(&mut self, now: f64, active: bool, out: &mut Vec<MouseOut>) {
        let dt = match self.last_render {
            Some(t) => (now - t).min(0.1),
            None => 0.0,
        };
        self.last_render = Some(now);
        if !active {
            return;
        }
        match self.cfg.mode {
            Mode::Souris => {
                // trou dans le flux : on continue sur la lancée, puis on freine
                let s = &self.cfg.souris;
                let age = now - self.last_sample;
                let fade = if age < s.coast_s { 1.0 } else { (1.0 - (age - s.coast_s) / s.fade_s.max(1e-3)).max(0.0) };
                let (dx, dy) = (self.vel.0 * dt * fade, self.vel.1 * dt * fade);
                if dx != 0.0 || dy != 0.0 {
                    out.push(MouseOut::Rel(dx, dy));
                    self.pos_px[0] += dx;
                    self.pos_px[1] += dy;
                }
            }
            Mode::Pointeur => {
                let a = 1.0 - (-dt / self.cfg.pointeur.render_lag_s.max(0.001)).exp();
                let nx = self.cur.0 + (self.target.0 - self.cur.0) * a;
                let ny = self.cur.1 + (self.target.1 - self.cur.1) * a;
                out.push(MouseOut::Abs(nx, ny));
                self.pos_px = [nx * 1920.0, ny * 1200.0];
                self.cur = (nx, ny);
            }
        }
    }

    fn roll_gesture(&mut self, now: f64, out: &mut Vec<MouseOut>) -> bool {
        let rc = &self.cfg.roll_click;
        let a = self.roll_ang.abs();
        if self.roll_latched {
            if a < rc.trigger_deg * 0.5 {
                self.roll_latched = false;
            }
            return true;
        }
        if a > rc.start_deg && self.roll_t0.is_none() {
            self.roll_t0 = Some(now);
        } else if a < rc.rest_deg {
            self.roll_t0 = None;
        }
        if a >= rc.trigger_deg {
            if let Some(t0) = self.roll_t0 {
                if now - t0 <= rc.max_duration_s {
                    let action = if self.roll_ang > 0.0 { rc.right.clone() } else { rc.left.clone() };
                    out.push(MouseOut::Command(action));
                }
                self.roll_latched = true;
                self.roll_t0 = None;
                return true;
            }
        }
        a > rc.start_deg
    }

    fn dwell(&mut self, now: f64, out: &mut Vec<MouseOut>) {
        let dc = &self.cfg.dwell_click;
        let [x, y] = self.pos_px;
        let Some(anchor) = self.dwell_anchor else {
            self.dwell_anchor = Some((x, y));
            self.dwell_t0 = now;
            return;
        };
        let d = (x - anchor.0).hypot(y - anchor.1);
        if d > dc.radius_px {
            self.dwell_moved += d;
            self.dwell_anchor = Some((x, y));
            self.dwell_t0 = now;
            self.dwell_fired = false;
        } else if !self.dwell_fired && self.dwell_moved >= dc.min_move_px && now - self.dwell_t0 >= dc.time_s {
            self.dwell_fired = true;
            self.dwell_moved = 0.0;
            out.push(MouseOut::Command("left_click".into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn souris_moves_right_and_up() {
        let mut m = MouseEngine::new(MouseConfig { smoothing: 0.0, ..Default::default() });
        let mut out = Vec::new();
        for i in 0..50 {
            let t = i as f64 * 0.01;
            m.on_rates(30.0, 10.0, 0.0, 0.01, t, true, &mut out);
            m.render(t, true, &mut out);
        }
        let (dx, dy) = out.iter().fold((0.0, 0.0), |a, o| match o {
            MouseOut::Rel(x, y) => (a.0 + x, a.1 + y),
            _ => a,
        });
        assert!(dx > 100.0 && dy < -20.0, "{dx} {dy}");
    }

    #[test]
    fn curve_and_no_acceleration() {
        let c = SourisConfig { acceleration_enabled: false, ..Default::default() };
        // sans accélération ni courbe : proportionnel
        assert!((cursor_speed(&c, 62.5, 1.0, 1.0) - 60.0 * 20.0).abs() < 1e-9);
        // courbe 2 : sous la référence plus lent, au-dessus plus rapide
        let slow = cursor_speed(&c, 12.5, 2.0, 1.0);
        let fast = cursor_speed(&c, 62.5, 2.0, 1.0);
        assert!(slow < 10.0 * 20.0 && fast > 60.0 * 20.0);
        assert_eq!(shape_rate(-10.0, 1.0, 30.0), -10.0);
    }

    #[test]
    fn deadzone_keeps_still() {
        assert_eq!(soft_deadzone(2.0, 2.5), 0.0);
        assert_eq!(soft_deadzone(-5.0, 2.5), -2.5);
    }

    #[test]
    fn pointer_with_range_reaches_edges() {
        let cfg = MouseConfig {
            mode: Mode::Pointeur,
            smoothing: 0.0,
            pointeur: PointeurConfig { range: Some(AimRange { x_neg: 20.0, x_pos: 22.0, y_pos: 12.0, y_neg: 11.0 }), rate_deadzone_dps: 0.0, ..Default::default() },
            ..Default::default()
        };
        let mut m = MouseEngine::new(cfg);
        let mut out = Vec::new();
        for i in 0..100 {
            m.on_rates(30.0, 0.0, 0.0, 0.01, i as f64 * 0.01, true, &mut out);
        }
        assert!((m.target.0 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn roll_click_fires_once() {
        let cfg = MouseConfig { smoothing: 0.0, roll_click: RollClickConfig { enabled: true, ..Default::default() }, ..Default::default() };
        let mut m = MouseEngine::new(cfg);
        let mut out = Vec::new();
        for i in 0..30 {
            m.on_rates(0.0, 0.0, 100.0, 0.01, i as f64 * 0.01, true, &mut out);
        }
        let clicks = out.iter().filter(|o| matches!(o, MouseOut::Command(c) if c == "left_click")).count();
        assert_eq!(clicks, 1);
    }
}
