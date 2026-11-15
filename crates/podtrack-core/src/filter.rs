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
}
