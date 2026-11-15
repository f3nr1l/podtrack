//! Moteur de mouvement : vitesses lacet / tangage / roulis (°/s) indépendantes de l'orientation
//! de l'écouteur. Repris de `MotionEngine` (MousePods).
//!
//! * lacet = rotation autour de la verticale (donnée par la gravité) ;
//! * tangage = rotation autour de l'axe latéral `lat` (appris en calibration), projeté à l'horizontale ;
//! * roulis = rotation autour de l'axe avant (vertical × latéral).
//!
//! Le biais du gyroscope est réestimé dès que l'écouteur est immobile.

use crate::vec::{self, V3};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Profil d'axes d'un écouteur, issu de la calibration de l'appareil.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceProfile {
    pub yaw_sign: f64,
    /// Axe de « lever la tête », repère de l'écouteur.
    pub lat: V3,
    pub pitch_sign: f64,
    pub roll_sign: f64,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self { yaw_sign: 1.0, lat: [0.0, 0.0, 1.0], pitch_sign: 1.0, roll_sign: 1.0 }
    }
}

/// Réglages de l'estimation du biais et de la projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// Étendue maximale du gyroscope (°/s) dans la fenêtre pour juger l'écouteur immobile.
    pub still_range_dps: f64,
    /// Durée d'immobilité (s) avant de mettre à jour le biais.
    pub still_time_s: f64,
    /// Taux de mise à jour du biais une fois le premier biais accepté.
    pub rate: f64,
    /// Taille de la fenêtre glissante (échantillons).
    pub window: usize,
    /// Nombre minimal d'échantillons dans la fenêtre.
    pub min_samples: usize,
    /// Rotation maximale de la gravité (°) dans la fenêtre pour juger l'écouteur immobile.
    pub still_gravity_deg: f64,
    /// Écart maximal (°/s) accepté pour le premier biais, puis pour les suivants.
    pub first_bias_max_dps: f64,
    pub bias_jump_max_dps: f64,
    /// Coefficient du filtre de la gravité (0..1, plus grand = plus réactif).
    pub gravity_alpha: f64,
    /// Norme minimale de l'axe latéral projeté (axe presque vertical en dessous).
    pub lat_min: f64,
    /// Norme minimale de la gravité brute pour être utilisée.
    pub gravity_min: f64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            still_range_dps: 3.0,
            still_time_s: 0.4,
            rate: 0.03,
            window: 40,
            min_samples: 8,
            still_gravity_deg: 1.5,
            first_bias_max_dps: 20.0,
            bias_jump_max_dps: 5.0,
            gravity_alpha: 0.15,
            lat_min: 0.25,
            gravity_min: 1e-3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MotionEngine {
    pub cfg: EngineConfig,
    profile: DeviceProfile,
    lat: V3,
    pub bias: V3,
    /// Gravité filtrée, normalisée (repère de l'écouteur).
    pub g: Option<V3>,
    win: VecDeque<V3>,
    gwin: VecDeque<V3>,
    still_t: f64,
    pub bias_ok: bool,
    lat_h: Option<V3>,
}

impl MotionEngine {
    pub fn new(cfg: EngineConfig, profile: DeviceProfile) -> Self {
        let mut e = Self {
            cfg,
            profile: DeviceProfile::default(),
            lat: [0.0, 0.0, 1.0],
            bias: [0.0; 3],
            g: None,
            win: VecDeque::new(),
            gwin: VecDeque::new(),
            still_t: 0.0,
            bias_ok: false,
            lat_h: None,
        };
        e.set_profile(profile);
        e
    }

    pub fn set_profile(&mut self, p: DeviceProfile) {
        self.lat = vec::vnorm(p.lat);
        self.profile = p;
        self.lat_h = None;
    }

    pub fn profile(&self) -> &DeviceProfile {
        &self.profile
    }

    /// Axe latéral normalisé (repère de l'écouteur), tel que le tracker l'attend.
    pub fn lat(&self) -> V3 {
        self.lat
    }

    pub fn signs(&self) -> (f64, f64, f64) {
        (self.profile.pitch_sign, self.profile.roll_sign, self.profile.yaw_sign)
    }

    /// Un échantillon : renvoie (lacet, tangage, roulis) en °/s, signes du profil appliqués.
    pub fn step(&mut self, w: V3, g: V3, dt: f64) -> V3 {
        let c = &self.cfg;
        if vec::len(g) > c.gravity_min {
            let gn = vec::vnorm(g);
            self.g = Some(match self.g {
                None => gn,
                Some(a) => vec::vnorm([
                    a[0] + c.gravity_alpha * (gn[0] - a[0]),
                    a[1] + c.gravity_alpha * (gn[1] - a[1]),
                    a[2] + c.gravity_alpha * (gn[2] - a[2]),
                ]),
            });
        }
        let Some(up) = self.g else { return [0.0; 3] };

        // biais : fenêtre glissante ; étendue faible assez longtemps -> immobile
        self.win.push_back(w);
        self.gwin.push_back(up);
        while self.win.len() > c.window.max(1) {
            self.win.pop_front();
            self.gwin.pop_front();
        }
        let mut rng = f64::NEG_INFINITY;
        for k in 0..3 {
            let mx = self.win.iter().map(|s| s[k]).fold(f64::NEG_INFINITY, f64::max);
            let mn = self.win.iter().map(|s| s[k]).fold(f64::INFINITY, f64::min);
            rng = rng.max(mx - mn);
        }
        let g0 = self.gwin[0];
        let g1 = self.gwin[self.gwin.len() - 1];
        let g_move = vec::clamp(vec::dot(g0, g1), -1.0, 1.0).acos().to_degrees();
        if rng < c.still_range_dps && g_move < c.still_gravity_deg && self.win.len() >= c.min_samples {
            self.still_t += dt;
        } else {
            self.still_t = 0.0;
        }
        if self.still_t >= c.still_time_s {
            let mean = vec::mean(self.win.iter());
            // une rotation lente et régulière n'est pas un biais : seuls les écarts plausibles passent
            let jump = vec::len(vec::sub(mean, self.bias));
            let max = if self.bias_ok { c.bias_jump_max_dps } else { c.first_bias_max_dps };
            if jump < max {
                let r = if self.bias_ok { c.rate } else { 1.0 };
                for k in 0..3 {
                    self.bias[k] += r * (mean[k] - self.bias[k]);
                }
                self.bias_ok = true;
            }
        }
        let wc = vec::sub(w, self.bias);

        // projection
        let yaw = vec::dot(wc, up);
        let mut lat_h = vec::reject(self.lat, up);
        if vec::len(lat_h) < c.lat_min {
            lat_h = self.lat_h.unwrap_or(lat_h);
        }
        let lat_h = vec::vnorm(lat_h);
        self.lat_h = Some(lat_h);
        let wh = vec::reject(wc, up);
        let pitch = vec::dot(wh, lat_h);
        let roll = vec::dot(wh, vec::cross(up, lat_h));
        [yaw * self.profile.yaw_sign, pitch * self.profile.pitch_sign, roll * self.profile.roll_sign]
    }
}

/// Pas de temps d'un échantillon d'après l'horodatage du capteur, avec repli sur l'horloge
/// locale quand l'horodatage est absent ou incohérent (reconnexion, saut).
#[derive(Debug, Clone, Default)]
pub struct SampleClock {
    last_ts: Option<u64>,
    last_now: Option<f64>,
    pub max_dt: f64,
    pub fallback_min: f64,
    pub fallback_max: f64,
    pub first_dt: f64,
}

impl SampleClock {
    pub fn new() -> Self {
        Self { last_ts: None, last_now: None, max_dt: 0.15, fallback_min: 0.001, fallback_max: 0.05, first_dt: 0.01 }
    }

    /// `ts_ns` : horodatage du capteur ; `now` : horloge monotone locale (s).
    pub fn dt(&mut self, ts_ns: u64, now: f64) -> f64 {
        // différence entière exacte puis conversion, comme en Python ; un horodatage nul vaut absence
        let mut dt = match self.last_ts {
            Some(l) if l != 0 => (ts_ns as i128 - l as i128) as f64 / 1e9,
            _ => 0.0,
        };
        if !(0.0 < dt && dt < self.max_dt) {
            dt = match self.last_now {
                Some(l) => vec::clamp(now - l, self.fallback_min, self.fallback_max),
                None => self.first_dt,
            };
        }
        self.last_ts = Some(ts_ns);
        self.last_now = Some(now);
        dt
    }
}
