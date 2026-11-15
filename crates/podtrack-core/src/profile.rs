//! Profils de suivi par jeu : lissage, passe-bas, courbe de réponse, zone morte et amplitude par
//! axe, modèle du cou. Repris de `game_profiles.py` (MousePods), avec en plus la zone morte, le
//! passe-bas et une courbe libre par points.
//!
//! Pur et déterministe : le temps arrive en `dt`. Angles en degrés, convention opentrack ;
//! translation en cm (x droite, y haut, z avant). Le jeu voit X + = gauche (les DLL inversent le
//! lacet mais pas X), d'où `GAME_X`.

use crate::filter::{LowPass, OneEuro, OneEuroParams};
use crate::tracker::Pose6;
use crate::vec;
use serde::{Deserialize, Serialize};

pub const AXES: [&str; 3] = ["yaw", "pitch", "roll"];
pub const GAME_X: f64 = -1.0;
const CURVE_PIVOT_DEG: f64 = 40.0;

/// Bornes appliquées à chaque profil (comme MousePods).
pub mod bounds {
    pub const EDGE: (f64, f64) = (2.0, 180.0);
    pub const TARGET: (f64, f64) = (0.0, 180.0);
    pub const CURVE: (f64, f64) = (1.0, 3.0);
    pub const SMOOTHING: (f64, f64) = (0.0, 1.0);
    pub const STRENGTH: (f64, f64) = (0.0, 2.0);
    pub const DEADZONE: (f64, f64) = (0.0, 45.0);
    pub const LOWPASS: (f64, f64) = (0.0, 100.0);
}

/// Angle de tête auquel l'ancien modèle gain/courbe/limite atteignait sa limite.
pub fn legacy_edge(gain: f64, curve: f64, limit: f64) -> f64 {
    CURVE_PIVOT_DEG * (limit / (gain * CURVE_PIVOT_DEG)).powf(1.0 / curve)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Axis {
    pub enabled: bool,
    pub invert: bool,
    /// Angle de tête (°) auquel la caméra atteint `target`, de chaque côté.
    pub edge_neg: f64,
    pub edge_pos: f64,
    /// Angle de la caméra (°) en bout de course.
    pub target: f64,
    /// Exposant de la courbe (1 = linéaire, plus grand = centre plus calme).
    pub curve: f64,
    /// Zone morte autour du centre (°) : en dessous, la caméra ne bouge pas.
    pub deadzone: f64,
    /// Courbe libre facultative : points (entrée 0..1, sortie 0..1) ; remplace `curve` si non vide.
    pub points: Vec<[f64; 2]>,
}

impl Axis {
    fn legacy(gain: f64, curve: f64, limit: f64) -> Axis {
        let edge = if limit > 0.0 {
            vec::round_dec(vec::clamp(legacy_edge(gain, curve, limit), bounds::EDGE.0, bounds::EDGE.1), 1)
        } else {
            40.0
        };
        Axis {
            enabled: limit > 0.0,
            invert: false,
            edge_neg: edge,
            edge_pos: edge,
            target: limit,
            curve,
            deadzone: 0.0,
            points: Vec::new(),
        }
    }

    pub fn default_for(name: &str) -> Axis {
        match name {
            "yaw" => Axis::legacy(2.0, 1.4, 180.0),
            "pitch" => Axis::legacy(1.5, 1.4, 90.0),
            _ => Axis::legacy(1.0, 1.0, 90.0),
        }
    }

    /// Ancien format MousePods (gain, courbe, limite autour d'un pivot de 40°).
    pub fn from_legacy(gain: f64, curve: f64, limit: f64, enabled: bool, invert: bool) -> Axis {
        let mut a = Axis::legacy(gain, curve, limit);
        a.enabled = enabled && limit > 0.0;
        a.invert = invert;
        a
    }

    fn normalize(&mut self) {
        self.edge_neg = clampf(self.edge_neg, bounds::EDGE, 40.0);
        self.edge_pos = clampf(self.edge_pos, bounds::EDGE, 40.0);
        self.target = clampf(self.target, bounds::TARGET, 90.0);
        self.curve = clampf(self.curve, bounds::CURVE, 1.0);
        // la zone morte reste en deçà du bord, sinon la course serait vide
        let dz_max = (self.edge_neg.min(self.edge_pos) - 1.0).max(0.0);
        self.deadzone = clampf(self.deadzone, (bounds::DEADZONE.0, bounds::DEADZONE.1.min(dz_max)), 0.0);
        self.points.retain(|p| p[0].is_finite() && p[1].is_finite());
        for p in self.points.iter_mut() {
            p[0] = vec::clamp(p[0], 0.0, 1.0);
            p[1] = vec::clamp(p[1], 0.0, 1.0);
        }
        self.points.sort_by(|a, b| a[0].total_cmp(&b[0]));
    }

    /// Forme de la réponse sur [0, 1] → [0, 1].
    pub fn shape(&self, x: f64) -> f64 {
        if self.points.is_empty() {
            return x.powf(self.curve);
        }
        // interpolation linéaire entre (0,0), les points, et (1,1)
        let mut prev = [0.0, 0.0];
        for p in self.points.iter().chain(std::iter::once(&[1.0, 1.0])) {
            if x <= p[0] {
                let span = p[0] - prev[0];
                return if span <= 0.0 { p[1] } else { prev[1] + (p[1] - prev[1]) * (x - prev[0]) / span };
            }
            prev = *p;
        }
        1.0
    }
}

impl Default for Axis {
    fn default() -> Self {
        Axis::default_for("roll")
    }
}

/// Clamp façon MousePods : NaN prend la valeur par défaut.
fn clampf(v: f64, (lo, hi): (f64, f64), default: f64) -> f64 {
    if v.is_nan() { default } else { vec::clamp(v, lo, hi) }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Axes {
    pub yaw: Axis,
    pub pitch: Axis,
    pub roll: Axis,
}

impl Default for Axes {
    fn default() -> Self {
        Self { yaw: Axis::default_for("yaw"), pitch: Axis::default_for("pitch"), roll: Axis::default_for("roll") }
    }
}

impl Axes {
    pub fn get(&self, i: usize) -> &Axis {
        match i {
            0 => &self.yaw,
            1 => &self.pitch,
            _ => &self.roll,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Neck {
    pub enabled: bool,
    pub strength: f64,
    pub lean_forward: bool,
    /// Position des yeux par rapport au pivot du cou (cm) : droite, haut, avant.
    pub eyes_cm: [f64; 3],
}

impl Default for Neck {
    fn default() -> Self {
        Self { enabled: false, strength: 1.0, lean_forward: false, eyes_cm: [0.0, 8.0, 9.0] }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Monitor {
    pub connector: String,
    pub main: bool,
}

impl Default for Monitor {
    fn default() -> Self {
        Self { connector: String::new(), main: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameProfile {
    /// Lissage 0..1 (0 = aucun). Coupure du 1€ = 10^(1 − 2·s) Hz.
    pub smoothing: f64,
    /// Bêta et coupure de dérivée du 1€.
    pub filter_beta: f64,
    pub filter_d_cutoff_hz: f64,
    /// Passe-bas supplémentaire après le 1€ (Hz ; 0 = aucun).
    pub lowpass_hz: f64,
    pub neck: Neck,
    pub axes: Axes,
    /// Moniteurs choisis à la calibration, de gauche à droite.
    pub monitors: Vec<Monitor>,
    /// Faux : l'amplitude du lacet et du tangage vient de la calibration de référence.
    pub own: bool,
}

impl Default for GameProfile {
    fn default() -> Self {
        Self {
            smoothing: 0.3,
            filter_beta: 0.05,
            filter_d_cutoff_hz: 1.0,
            lowpass_hz: 0.0,
            neck: Neck::default(),
            axes: Axes::default(),
            monitors: Vec::new(),
            own: false,
        }
    }
}

pub const MAX_MONITORS: usize = 8;

impl GameProfile {
    /// Borne chaque nombre et nettoie la liste des moniteurs.
    pub fn normalized(mut self) -> Self {
        self.smoothing = clampf(self.smoothing, bounds::SMOOTHING, 0.3);
        self.lowpass_hz = clampf(self.lowpass_hz, bounds::LOWPASS, 0.0);
        self.neck.strength = clampf(self.neck.strength, bounds::STRENGTH, 1.0);
        for a in [&mut self.axes.yaw, &mut self.axes.pitch, &mut self.axes.roll] {
            a.normalize();
        }
        self.monitors.retain(|m| !m.connector.is_empty());
        self.monitors.truncate(MAX_MONITORS);
        for m in self.monitors.iter_mut() {
            m.connector.truncate(64);
        }
        if !self.monitors.is_empty() {
            let main = self.monitors.iter().position(|m| m.main).unwrap_or(0);
            for (i, m) in self.monitors.iter_mut().enumerate() {
                m.main = i == main;
            }
        }
        self
    }

    /// Le profil effectif : ses propres valeurs, ou l'amplitude de la calibration de référence.
    pub fn effective(&self, baseline: Option<&Baseline>) -> GameProfile {
        let mut p = self.clone();
        if self.own {
            return p;
        }
        if let Some(b) = baseline {
            p.axes.yaw.edge_neg = b.yaw.neg;
            p.axes.yaw.edge_pos = b.yaw.pos;
            p.axes.yaw.target = b.target_side;
            p.axes.pitch.edge_neg = b.pitch.neg;
            p.axes.pitch.edge_pos = b.pitch.pos;
            p.axes.pitch.target = b.target_vert;
        }
        p
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Range {
    pub neg: f64,
    pub pos: f64,
}

/// Calibration de référence (page Calibration) : angle de tête à chaque bord, amplitude caméra.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Baseline {
    pub yaw: Range,
    pub pitch: Range,
    pub target_side: f64,
    pub target_vert: f64,
    pub monitors: Vec<Monitor>,
}

impl Default for Baseline {
    fn default() -> Self {
        Self { yaw: Range::default(), pitch: Range::default(), target_side: 180.0, target_vert: 90.0, monitors: Vec::new() }
    }
}

impl Baseline {
    /// Bornée ; None si inutilisable (un bord manquant ou non fini).
    pub fn normalized(mut self) -> Option<Self> {
        for r in [&mut self.yaw, &mut self.pitch] {
            if !r.neg.is_finite() || !r.pos.is_finite() {
                return None;
            }
            r.neg = vec::clamp(r.neg, bounds::EDGE.0, bounds::EDGE.1);
            r.pos = vec::clamp(r.pos, bounds::EDGE.0, bounds::EDGE.1);
        }
        self.target_side = clampf(self.target_side, bounds::TARGET, 180.0);
        self.target_vert = clampf(self.target_vert, bounds::TARGET, 90.0);
        Some(self)
    }
}

/// Angle de tête → angle en jeu : au bord, la caméra est à `target`.
pub fn map_axis(deg: f64, axis: &Axis) -> f64 {
    if !axis.enabled || deg == 0.0 {
        return 0.0;
    }
    let edge = if deg > 0.0 { axis.edge_pos } else { axis.edge_neg };
    let a = deg.abs();
    let x = if axis.deadzone > 0.0 {
        if a <= axis.deadzone {
            return 0.0;
        }
        ((a - axis.deadzone) / (edge - axis.deadzone)).min(1.0)
    } else {
        (a / edge).min(1.0)
    };
    let out = (axis.target * axis.shape(x)).copysign(deg);
    if axis.invert { -out } else { out }
}

/// Déplacement des yeux quand la tête tourne autour du pivot du cou (cm). Calculé, pas mesuré.
pub fn neck_offset(yaw: f64, pitch: f64, roll: f64, strength: f64, eyes: [f64; 3]) -> [f64; 3] {
    if strength == 0.0 || (yaw == 0.0 && pitch == 0.0 && roll == 0.0) {
        return [0.0; 3];
    }
    let (y, p, r) = (yaw.to_radians(), pitch.to_radians(), roll.to_radians());
    let [vx, vy, vz] = eyes;
    let x1 = vx * r.cos() + vy * r.sin();
    let y1 = -vx * r.sin() + vy * r.cos();
    let z1 = vz;
    let y2 = y1 * p.cos() + z1 * p.sin();
    let z2 = -y1 * p.sin() + z1 * p.cos();
    let x2 = x1;
    let x3 = x2 * y.cos() + z2 * y.sin();
    let z3 = -x2 * y.sin() + z2 * y.cos();
    let y3 = y2;
    [(x3 - vx) * strength, (y3 - vy) * strength, (z3 - vz) * strength]
}

/// 0 → pas de filtre ; 0,3 → ~2,5 Hz ; 1 → 0,1 Hz.
pub fn smoothing_cutoff(s: f64) -> Option<f64> {
    if s <= 0.0 { None } else { Some(10f64.powf(1.0 - 2.0 * s)) }
}

pub fn sensitivity_to_curve(s: f64) -> f64 {
    3.0 - 2.0 * vec::clamp(s, 0.0, 1.0)
}

pub fn curve_to_sensitivity(c: f64) -> f64 {
    vec::clamp((3.0 - c) / 2.0, 0.0, 1.0)
}

/// Filtre d'un jeu : 1€ et passe-bas sur les angles, puis courbes, puis cou.
#[derive(Debug, Clone)]
pub struct ProfileFilter {
    pub profile: GameProfile,
    filters: Option<[OneEuro; 3]>,
    lowpass: [LowPass; 3],
}

impl ProfileFilter {
    pub fn new(profile: GameProfile) -> Self {
        let profile = profile.normalized();
        let filters = smoothing_cutoff(profile.smoothing).map(|mc| {
            let p = OneEuroParams { min_cutoff_hz: mc, beta: profile.filter_beta, d_cutoff_hz: profile.filter_d_cutoff_hz };
            [OneEuro::new(p), OneEuro::new(p), OneEuro::new(p)]
        });
        let lp = profile.lowpass_hz;
        Self { profile, filters, lowpass: [LowPass::new(lp), LowPass::new(lp), LowPass::new(lp)] }
    }

    pub fn apply(&mut self, pose: Pose6, dt: f64) -> Pose6 {
        let [mut x, mut y, mut z, yaw, pitch, roll] = pose;
        let mut ang = [yaw, pitch, roll];
        if let Some(f) = self.filters.as_mut() {
            for i in 0..3 {
                ang[i] = f[i].apply(ang[i], dt);
            }
        }
        for i in 0..3 {
            ang[i] = self.lowpass[i].apply(ang[i], dt);
        }
        let axes = &self.profile.axes;
        let out = [map_axis(ang[0], axes.get(0)), map_axis(ang[1], axes.get(1)), map_axis(ang[2], axes.get(2))];
        let neck = &self.profile.neck;
        if neck.enabled {
            let d = neck_offset(ang[0], ang[1], ang[2], neck.strength, neck.eyes_cm);
            // une vraie tête recule un peu en tournant ; lean_forward avance au contraire
            let sz = if neck.lean_forward { -1.0 } else { 1.0 };
            x += GAME_X * d[0];
            y += d[1];
            z += sz * d[2];
        }
        [x, y, z, out[0], out[1], out[2]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(edge_neg: f64, edge_pos: f64, target: f64, curve: f64) -> Axis {
        Axis { enabled: true, invert: false, edge_neg, edge_pos, target, curve, deadzone: 0.0, points: vec![] }
    }

    #[test]
    fn map_axis_like_mousepods() {
        let a = axis(10.0, 25.0, 180.0, 1.0);
        assert_eq!(map_axis(25.0, &a), 180.0);
        assert_eq!(map_axis(-10.0, &a), -180.0);
        assert_eq!(map_axis(-5.0, &a), -90.0);
        assert_eq!(map_axis(50.0, &a), 180.0);
        let mut b = axis(40.0, 40.0, 90.0, 2.0);
        b.invert = true;
        assert_eq!(map_axis(20.0, &b), -22.5);
        assert_eq!(map_axis(60.0, &b), -90.0);
    }

    #[test]
    fn defaults_match_legacy() {
        let d = Axes::default();
        assert_eq!(d.yaw.edge_pos, 71.4);
        assert_eq!(d.pitch.edge_pos, 53.4);
        assert_eq!(d.roll.edge_pos, 90.0);
        assert_eq!(d.roll.curve, 1.0);
    }

    #[test]
    fn clamps() {
        let mut p = GameProfile { smoothing: 5.0, ..Default::default() };
        p.axes.yaw.edge_neg = 500.0;
        p.axes.yaw.edge_pos = 0.0;
        p.axes.yaw.target = 999.0;
        p.axes.yaw.curve = 0.2;
        p.neck.strength = 7.0;
        let p = p.normalized();
        assert_eq!(p.smoothing, 1.0);
        assert_eq!(p.axes.yaw.edge_neg, 180.0);
        assert_eq!(p.axes.yaw.edge_pos, 2.0);
        assert_eq!(p.axes.yaw.target, 180.0);
        assert_eq!(p.axes.yaw.curve, 1.0);
        assert_eq!(p.neck.strength, 2.0);
    }

    #[test]
    fn deadzone_and_points() {
        let mut a = axis(40.0, 40.0, 180.0, 1.0);
        a.deadzone = 4.0;
        assert_eq!(map_axis(3.0, &a), 0.0);
        assert_eq!(map_axis(40.0, &a), 180.0);
        assert!((map_axis(22.0, &a) - 90.0).abs() < 1e-9);
        a.deadzone = 0.0;
        a.points = vec![[0.5, 0.25]];
        assert!((map_axis(20.0, &a) - 45.0).abs() < 1e-9);
        assert!((map_axis(30.0, &a) - 112.5).abs() < 1e-9);
    }

    #[test]
    fn effective_profile_and_baseline() {
        let b = Baseline { yaw: Range { neg: 18.0, pos: 17.0 }, pitch: Range { neg: 8.0, pos: 11.0 }, target_side: 150.0, target_vert: 70.0, monitors: vec![] };
        let p = GameProfile::default().effective(Some(&b));
        assert_eq!((p.axes.yaw.edge_neg, p.axes.yaw.edge_pos, p.axes.yaw.target), (18.0, 17.0, 150.0));
        assert_eq!((p.axes.pitch.edge_neg, p.axes.pitch.edge_pos, p.axes.pitch.target), (8.0, 11.0, 70.0));
        let own = GameProfile { own: true, ..Default::default() }.effective(Some(&b));
        assert_eq!(own.axes.yaw.edge_pos, 71.4);
    }

    #[test]
    fn no_smoothing_passes_through() {
        let mut p = GameProfile { smoothing: 0.0, ..Default::default() };
        p.axes.yaw = axis(180.0, 180.0, 180.0, 1.0);
        p.axes.pitch = axis(180.0, 180.0, 180.0, 1.0);
        p.axes.roll = axis(180.0, 180.0, 180.0, 1.0);
        let mut f = ProfileFilter::new(p);
        let o = f.apply([0.0, 0.0, 0.0, 12.5, -3.0, 1.5], 0.01);
        assert_eq!(o, [0.0, 0.0, 0.0, 12.5, -3.0, 1.5]);
    }

    #[test]
    fn neck_moves_eyes() {
        let mut p = GameProfile::default();
        p.neck.enabled = true;
        let mut f = ProfileFilter::new(p.clone());
        let o = f.apply([0.0, 0.0, 0.0, 30.0, 0.0, 0.0], 0.0);
        assert!(o[0] < -1.0 && o[2] < 0.0);
        p.neck.lean_forward = true;
        let mut f = ProfileFilter::new(p);
        let o2 = f.apply([0.0, 0.0, 0.0, 30.0, 0.0, 0.0], 0.0);
        assert!((o2[2] + o[2]).abs() < 1e-12);
    }

    #[test]
    fn sensitivity() {
        assert_eq!(sensitivity_to_curve(0.0), 3.0);
        assert_eq!(sensitivity_to_curve(1.0), 1.0);
    }
}
