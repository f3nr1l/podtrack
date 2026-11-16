//! Filtres temporels : 1€ (Casiez 2012) et passe-bas du premier ordre.

use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[inline]
fn alpha(cutoff: f64, dt: f64) -> f64 {
    let tau = 1.0 / (2.0 * PI * cutoff);
    1.0 / (1.0 + tau / dt)
}

/// Réglages d'un filtre 1€ : fréquence de coupure au repos, réactivité à la vitesse, coupure
/// de la dérivée.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OneEuroParams {
    pub min_cutoff_hz: f64,
    pub beta: f64,
    pub d_cutoff_hz: f64,
}

impl Default for OneEuroParams {
    fn default() -> Self {
        Self { min_cutoff_hz: 1.0, beta: 0.05, d_cutoff_hz: 1.0 }
    }
}

/// Filtre 1€ : lisse au repos, peu de retard pendant les mouvements rapides.
#[derive(Debug, Clone)]
pub struct OneEuro {
    pub p: OneEuroParams,
    x: Option<f64>,
    dx: f64,
}

impl OneEuro {
    pub fn new(p: OneEuroParams) -> Self {
        Self { p, x: None, dx: 0.0 }
    }

    pub fn apply(&mut self, x: f64, dt: f64) -> f64 {
        let Some(prev) = self.x.filter(|_| dt > 0.0) else {
            self.x = Some(x);
            self.dx = 0.0;
            return x;
        };
        let dx = (x - prev) / dt;
        self.dx += alpha(self.p.d_cutoff_hz, dt) * (dx - self.dx);
        let nx = prev + alpha(self.p.min_cutoff_hz + self.p.beta * self.dx.abs(), dt) * (x - prev);
        self.x = Some(nx);
        nx
    }

    pub fn reset(&mut self) {
        self.x = None;
        self.dx = 0.0;
    }
}

/// Passe-bas du premier ordre (coupure en Hz). Coupure ≤ 0 : sans effet.
#[derive(Debug, Clone)]
pub struct LowPass {
    pub cutoff_hz: f64,
    y: Option<f64>,
}

impl LowPass {
    pub fn new(cutoff_hz: f64) -> Self {
        Self { cutoff_hz, y: None }
    }

    pub fn apply(&mut self, x: f64, dt: f64) -> f64 {
        if self.cutoff_hz <= 0.0 {
            return x;
        }
        let y = match self.y {
            Some(prev) if dt > 0.0 => prev + alpha(self.cutoff_hz, dt) * (x - prev),
            _ => x,
        };
        self.y = Some(y);
        y
    }

    pub fn reset(&mut self) {
        self.y = None;
    }
}

/// Prolonge la pose entre deux paquets Bluetooth : les échantillons arrivent groupés, la
/// pose resterait figée entre deux paquets puis sauterait. On poursuit le mouvement à la
/// vitesse mesurée (lissée), pendant au plus `max_s` après le dernier échantillon.
#[derive(Debug, Clone, Default)]
pub struct Extrapolator {
    last: Option<crate::vec::V3>,
    vel: crate::vec::V3,
    t_last: f64,
}

impl Extrapolator {
    /// Nouvel échantillon `head` (°), `dt` d'horloge capteur (s), reçu à `now` (s).
    /// `alpha` : poids de la nouvelle vitesse (0..1). Un écart de plus de `jump_deg` entre deux
    /// échantillons (recentrage, réinitialisation de fusion) remet la vitesse à zéro.
    pub fn update(&mut self, head: crate::vec::V3, dt: f64, now: f64, alpha: f64, jump_deg: f64) {
        if let Some(prev) = self.last {
            let d = [0, 1, 2].map(|i| crate::vec::wrap180(head[i] - prev[i]));
            if d.iter().any(|x| x.abs() > jump_deg) {
                self.vel = [0.0; 3];
            } else if dt > 0.0 {
                for i in 0..3 {
                    self.vel[i] += alpha.clamp(0.0, 1.0) * (d[i] / dt - self.vel[i]);
                }
            }
        }
        self.last = Some(head);
        self.t_last = now;
    }

    /// Pose prolongée à `now` ; la dernière pose telle quelle si `max_s` ≤ 0.
    pub fn predict(&self, now: f64, max_s: f64) -> Option<crate::vec::V3> {
        let h = self.last?;
        let age = (now - self.t_last).clamp(0.0, max_s.max(0.0));
        Some([0, 1, 2].map(|i| h[i] + self.vel[i] * age))
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_euro_first_sample_passes_through() {
        let mut f = OneEuro::new(OneEuroParams { min_cutoff_hz: 2.0, ..Default::default() });
        assert_eq!(f.apply(5.0, 0.01), 5.0);
        let y = f.apply(6.0, 0.01);
        assert!(y > 5.0 && y < 6.0);
    }

    #[test]
    fn lowpass_settles() {
        let mut f = LowPass::new(5.0);
        let mut y = 0.0;
        f.apply(0.0, 0.01);
        for _ in 0..500 {
            y = f.apply(1.0, 0.01);
        }
        assert!((y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn extrapolator_continues_motion_and_caps() {
        let mut e = Extrapolator::default();
        e.update([0.0, 0.0, 0.0], 0.01, 1.00, 1.0, 20.0);
        e.update([1.0, -0.5, 0.0], 0.01, 1.01, 1.0, 20.0); // 100 °/s en lacet
        let p = e.predict(1.02, 0.05).unwrap();
        assert!((p[0] - 2.0).abs() < 1e-9 && (p[1] + 1.0).abs() < 1e-9);
        let capped = e.predict(2.0, 0.05).unwrap();
        assert!((capped[0] - 6.0).abs() < 1e-9);
        assert_eq!(e.predict(1.5, 0.0).unwrap()[0], 1.0);
        e.update([40.0, 0.0, 0.0], 0.01, 1.02, 1.0, 20.0); // saut : vitesse oubliée
        assert_eq!(e.predict(1.05, 0.05).unwrap()[0], 40.0);
    }
}
