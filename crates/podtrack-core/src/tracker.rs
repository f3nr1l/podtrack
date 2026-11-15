//! Angles absolus de la tête (°), relatifs au dernier recentrage. Repris de `HeadTracker`
//! (MousePods, `opentrack_out.py`).
//!
//! Deux sources :
//! * `quaternion` (défaut) : orientation calculée par les AirPods. Au recentrage on fige ce repère
//!   et la base tête (latéral calibré, avant, vertical) ; ensuite lacet / tangage / roulis sont la
//!   décomposition lacet→tangage→roulis de la rotation depuis le recentrage. Une réinitialisation
//!   de la fusion des AirPods (saut sans rotation au gyroscope) est recalée.
//! * `gyro` : intégration des vitesses du moteur, avec correction de dérive du tangage et du
//!   roulis par la gravité. Sert aussi tant que le quaternion est absent.
//!
//! Conventions : lacet + = regarder à droite, tangage + = en haut, roulis + = oreille droite vers
//! l'épaule droite.

use crate::filter::{OneEuro, OneEuroParams};
use crate::vec::{self, M3, V3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Quaternion,
    Gyro,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackerConfig {
    pub source: Source,
    /// Lissage global 0..1 (0 = aucun filtre). Coupure du 1€ = max_cutoff + (min_cutoff − max_cutoff)·s.
    pub smoothing: f64,
    pub smoothing_max_cutoff_hz: f64,
    pub smoothing_min_cutoff_hz: f64,
    pub filter_beta: f64,
    pub filter_d_cutoff_hz: f64,
    pub drift_correction: bool,
    pub drift_tau_s: f64,
    pub drift_tau_min_s: f64,
    /// Retour lent du lacet au centre (constante de temps, s ; 0 = désactivé).
    pub yaw_autocenter_s: f64,
    /// Vitesse de lacet (°/s) sous laquelle l'auto-centrage agit.
    pub autocenter_max_rate_dps: f64,
    pub scale: V3,
    pub invert: [bool; 3],
    pub lean_experimental: bool,
    pub neck_cm: f64,
    /// Pas de temps accepté : 0 < dt < max_dt.
    pub max_dt: f64,
    /// Verticale « monde » de la fusion : |z| au-dessus de ce seuil, on la prend exactement verticale.
    pub up_snap: f64,
    /// Norme minimale de l'axe latéral projeté.
    pub lat_min: f64,
    /// Réinitialisation de la fusion : saut > max(min, k·rotation vue + add) en moins de gap_s.
    pub fusion_jump_min_deg: f64,
    pub fusion_jump_k: f64,
    pub fusion_jump_add_deg: f64,
    pub fusion_gap_s: f64,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            source: Source::Quaternion,
            smoothing: 0.2,
            smoothing_max_cutoff_hz: 8.0,
            smoothing_min_cutoff_hz: 0.6,
            filter_beta: 0.05,
            filter_d_cutoff_hz: 1.0,
            drift_correction: true,
            drift_tau_s: 2.0,
            drift_tau_min_s: 0.05,
            yaw_autocenter_s: 0.0,
            autocenter_max_rate_dps: 5.0,
            scale: [1.0, 1.0, 1.0],
            invert: [false; 3],
            lean_experimental: false,
            neck_cm: 10.0,
            max_dt: 0.5,
            up_snap: 0.8,
            lat_min: 0.25,
            fusion_jump_min_deg: 25.0,
            fusion_jump_k: 4.0,
            fusion_jump_add_deg: 10.0,
            fusion_gap_s: 0.1,
        }
    }
}

/// Pose à six degrés : x, y, z (cm), lacet, tangage, roulis (°).
pub type Pose6 = [f64; 6];

/// Quaternion unitaire (w, x, y, z) depuis la partie vectorielle Q15, w ≥ 0 ; None si le capteur
/// n'est pas prêt.
pub fn quat_from_att(att: [i16; 3]) -> Option<[f64; 4]> {
    if att == [0, 0, 0] {
        return None;
    }
    let x = att[0] as f64 / 32768.0;
    let y = att[1] as f64 / 32768.0;
    let z = att[2] as f64 / 32768.0;
    let n2 = x * x + y * y + z * z;
    if n2 > 1.01 {
        return None;
    }
    Some([(1.0 - n2).max(0.0).sqrt(), x, y, z])
}

/// Quaternion → matrice (corps → monde).
pub fn quat_matrix(q: [f64; 4]) -> M3 {
    let [w, x, y, z] = q;
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - w * z), 2.0 * (x * z + w * y)],
        [2.0 * (x * y + w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - w * x)],
        [2.0 * (x * z - w * y), 2.0 * (y * z + w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

fn rot_angle(m: &M3) -> f64 {
    vec::clamp((m[0][0] + m[1][1] + m[2][2] - 1.0) / 2.0, -1.0, 1.0).acos().to_degrees()
}

/// Rotation exprimée dans la base tête (x latéral, y avant, z haut) → (lacet, tangage, roulis).
pub fn head_angles(m: &M3) -> V3 {
    let pitch = vec::clamp(m[2][1], -1.0, 1.0).asin();
    let roll = (-m[2][0]).atan2(m[2][2]);
    let yaw = (-m[0][1]).atan2(m[1][1]);
    [yaw.to_degrees(), pitch.to_degrees(), roll.to_degrees()]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Used {
    Quaternion,
    Gyro,
}

#[derive(Debug, Clone)]
pub struct HeadTracker {
    pub cfg: TrackerConfig,
    pub suspend_autocenter: bool,
    filters: Option<[OneEuro; 3]>,
    yaw: f64,
    pitch: f64,
    roll: f64,
    reference: Option<(V3, V3, V3)>,
    grav0: (f64, f64),
    qref: Option<(M3, M3)>,
    q_prev: Option<M3>,
    t_prev: f64,
    q_off: V3,
    yaw_off: f64,
    ycont: Option<f64>,
    pub out: Pose6,
    /// Lacet, tangage, roulis avant l'échelle et l'inversion globales (ce que lisent les jeux).
    pub head: V3,
    pub used: Option<Used>,
}

impl HeadTracker {
    pub fn new(cfg: TrackerConfig) -> Self {
        let mut t = Self {
            cfg,
            suspend_autocenter: false,
            filters: None,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            reference: None,
            grav0: (0.0, 0.0),
            qref: None,
            q_prev: None,
            t_prev: 0.0,
            q_off: [0.0; 3],
            yaw_off: 0.0,
            ycont: None,
            out: [0.0; 6],
            head: [0.0; 3],
            used: None,
        };
        t.make_filters();
        t.recenter();
        t
    }

    pub fn configure(&mut self, cfg: TrackerConfig) {
        let refilter = cfg.smoothing != self.cfg.smoothing
            || cfg.smoothing_max_cutoff_hz != self.cfg.smoothing_max_cutoff_hz
            || cfg.smoothing_min_cutoff_hz != self.cfg.smoothing_min_cutoff_hz
            || cfg.filter_beta != self.cfg.filter_beta
            || cfg.filter_d_cutoff_hz != self.cfg.filter_d_cutoff_hz;
        self.cfg = cfg;
        if refilter {
            self.make_filters();
        }
    }

    fn make_filters(&mut self) {
        let s = vec::clamp(self.cfg.smoothing, 0.0, 1.0);
        let c = &self.cfg;
        let mc = c.smoothing_max_cutoff_hz + (c.smoothing_min_cutoff_hz - c.smoothing_max_cutoff_hz) * s;
        let p = OneEuroParams { min_cutoff_hz: mc, beta: c.filter_beta, d_cutoff_hz: c.filter_d_cutoff_hz };
        self.filters = if s > 0.0 { Some([OneEuro::new(p), OneEuro::new(p), OneEuro::new(p)]) } else { None };
    }

    pub fn recenter(&mut self) {
        self.yaw = 0.0;
        self.pitch = 0.0;
        self.roll = 0.0;
        self.reference = None;
        self.qref = None;
        self.q_prev = None;
        self.q_off = [0.0; 3];
        self.yaw_off = 0.0;
        self.ycont = None;
        if let Some(f) = self.filters.as_mut() {
            for x in f.iter_mut() {
                x.reset();
            }
        }
        self.out = [0.0; 6];
        self.head = [0.0; 3];
        self.used = None;
    }

    fn gravity_angles(&self, up: V3, signs: (f64, f64)) -> (f64, f64) {
        let (up0, lat0, fwd0) = self.reference.expect("repère gravité");
        let u = vec::norm1(up);
        let c = vec::dot(u, up0);
        let gp = vec::dot(u, fwd0).atan2(c).to_degrees() * signs.0;
        let gr = -vec::dot(u, lat0).atan2(c).to_degrees() * signs.1;
        (gp, gr)
    }

    /// Un échantillon. `signs` = (tangage, roulis, lacet) du profil calibré ; `now` = horloge
    /// monotone (s), pour juger l'écart entre deux échantillons.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        rates: V3,
        dt: f64,
        up: Option<V3>,
        lat: Option<V3>,
        signs: (f64, f64, f64),
        att: Option<[i16; 3]>,
        now: f64,
    ) -> Pose6 {
        if !(0.0 < dt && dt < self.cfg.max_dt) {
            return self.out;
        }
        let q = if self.cfg.source == Source::Quaternion { att.and_then(quat_from_att) } else { None };
        if let (Some(q), Some(up), Some(lat)) = (q, up, lat) {
            if let Some(ang) = self.from_quat(q, up, lat, signs, dt, rates, now) {
                self.used = Some(Used::Quaternion);
                self.yaw = ang[0];
                self.pitch = ang[1];
                self.roll = ang[2];
                return self.finish(ang, dt, rates[0]);
            }
        }
        if self.used == Some(Used::Quaternion) {
            self.qref = None; // quaternion perdu : on continue en intégrant
        }
        self.used = Some(Used::Gyro);
        let ang = self.from_rates(rates, dt, up, lat, signs);
        self.finish(ang, dt, rates[0])
    }

    #[allow(clippy::too_many_arguments)]
    fn from_quat(&mut self, q: [f64; 4], up: V3, lat: V3, signs: (f64, f64, f64), dt: f64, rates: V3, now: f64) -> Option<V3> {
        let c = &self.cfg;
        let rq = quat_matrix(q);
        if self.qref.is_none() {
            let mut up_w = vec::norm1(vec::mv(&rq, vec::norm1(up)));
            if up_w[2].abs() > c.up_snap {
                up_w = [0.0, 0.0, 1.0_f64.copysign(up_w[2])];
            }
            let lw0 = vec::mv(&rq, lat);
            let d = vec::dot(lw0, up_w);
            let lw = [lw0[0] - d * up_w[0], lw0[1] - d * up_w[1], lw0[2] - d * up_w[2]];
            if vec::dot(lw, lw).sqrt() < c.lat_min {
                return None; // axe latéral presque vertical : inutilisable
            }
            let lat_w = vec::norm1(lw);
            let fwd_w = vec::cross(up_w, lat_w);
            let e: M3 = [
                [lat_w[0], fwd_w[0], up_w[0]],
                [lat_w[1], fwd_w[1], up_w[1]],
                [lat_w[2], fwd_w[2], up_w[2]],
            ];
            self.qref = Some((rq, e));
            self.q_off = [self.yaw, self.pitch, self.roll];
        } else if let Some(prev) = self.q_prev {
            // réinitialisation de la fusion : grand saut sans rotation au gyroscope, entre deux
            // échantillons rapprochés (après une coupure, un grand écart est normal)
            let jump = rot_angle(&vec::mm(&rq, &vec::mt(&prev)));
            let seen = (rates[0] * rates[0] + rates[1] * rates[1] + rates[2] * rates[2]).sqrt() * dt;
            let gap = now - self.t_prev;
            if jump > c.fusion_jump_min_deg.max(c.fusion_jump_k * seen + c.fusion_jump_add_deg) && gap < c.fusion_gap_s {
                let (r0, e) = self.qref.unwrap();
                let rel_prev = vec::mm(&prev, &vec::mt(&r0));
                self.qref = Some((vec::mm(&vec::mt(&rel_prev), &rq), e));
            }
        }
        self.q_prev = Some(rq);
        self.t_prev = now;
        let (r0, e) = self.qref.unwrap();
        let m = vec::mm(&vec::mt(&e), &vec::mm(&vec::mm(&rq, &vec::mt(&r0)), &e));
        let [y, p, r] = head_angles(&m);
        Some([
            vec::wrap180(y * signs.2 + self.q_off[0]),
            p * signs.0 + self.q_off[1],
            r * signs.1 + self.q_off[2],
        ])
    }

    fn from_rates(&mut self, rates: V3, dt: f64, up: Option<V3>, lat: Option<V3>, signs: (f64, f64, f64)) -> V3 {
        self.yaw = vec::wrap180(self.yaw + rates[0] * dt);
        self.pitch += rates[1] * dt;
        self.roll += rates[2] * dt;
        if self.cfg.drift_correction {
            if let (Some(up), Some(lat)) = (up, lat) {
                if self.reference.is_none() {
                    let up0 = vec::norm1(up);
                    let d = vec::dot(lat, up0);
                    let l = [lat[0] - d * up0[0], lat[1] - d * up0[1], lat[2] - d * up0[2]];
                    if vec::dot(l, l).sqrt() > self.cfg.lat_min {
                        let lat0 = vec::norm1(l);
                        self.reference = Some((up0, lat0, vec::cross(up0, lat0)));
                        let g0 = self.gravity_angles(up, (signs.0, signs.1));
                        // angles déjà intégrés depuis le recentrage : on garde la même origine
                        self.grav0 = (g0.0 - self.pitch, g0.1 - self.roll);
                    }
                }
                if self.reference.is_some() {
                    let (mut gp, mut gr) = self.gravity_angles(up, (signs.0, signs.1));
                    gp -= self.grav0.0;
                    gr -= self.grav0.1;
                    let k = dt / (self.cfg.drift_tau_s.max(self.cfg.drift_tau_min_s) + dt);
                    self.pitch += k * (gp - self.pitch);
                    self.roll += k * (gr - self.roll);
                }
            }
        }
        [self.yaw, self.pitch, self.roll]
    }

    fn finish(&mut self, ang: V3, dt: f64, yaw_rate: f64) -> Pose6 {
        let c = self.cfg.clone();
        let [mut yaw, mut pitch, mut roll] = ang;
        let ac = c.yaw_autocenter_s;
        if ac > 0.0 && yaw_rate.abs() < c.autocenter_max_rate_dps && !self.suspend_autocenter {
            self.yaw_off += vec::wrap180(yaw - self.yaw_off) * (1.0 - (-dt / ac).exp());
        }
        yaw = vec::wrap180(yaw - self.yaw_off);
        // lacet déroulé pour le filtre (pas de saut de 360° à ±180°)
        let yc = match self.ycont {
            None => yaw,
            Some(prev) => prev + vec::wrap180(yaw - prev),
        };
        self.ycont = Some(yc);
        yaw = yc;
        if let Some(f) = self.filters.as_mut() {
            yaw = f[0].apply(yaw, dt);
            pitch = f[1].apply(pitch, dt);
            roll = f[2].apply(roll, dt);
        }
        yaw = vec::wrap180(yaw);
        self.head = [yaw, pitch, roll];
        yaw *= c.scale[0] * if c.invert[0] { -1.0 } else { 1.0 };
        pitch *= c.scale[1] * if c.invert[1] { -1.0 } else { 1.0 };
        roll *= c.scale[2] * if c.invert[2] { -1.0 } else { 1.0 };
        let (x, y, z) = if c.lean_experimental {
            (c.neck_cm * roll.to_radians().sin(), 0.0, -c.neck_cm * pitch.to_radians().sin())
        } else {
            (0.0, 0.0, 0.0)
        };
        self.out = [x, y, z, yaw, pitch, roll];
        self.out
    }
}
