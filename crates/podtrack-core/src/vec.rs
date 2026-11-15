//! Petits outils de géométrie 3D, écrits pour reproduire au bit près les calculs de MousePods
//! (même ordre des opérations, mêmes cas limites), afin que la comparaison avec la référence
//! Python reste exacte.

pub type V3 = [f64; 3];
pub type M3 = [[f64; 3]; 3];

#[inline]
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[inline]
pub fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}

#[inline]
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// Normalisation façon `vnorm` (contrôleur) : vecteur inchangé si sa norme est ≤ 1e-9.
#[inline]
pub fn vnorm(a: V3) -> V3 {
    let n = dot(a, a).sqrt();
    if n > 1e-9 { [a[0] / n, a[1] / n, a[2] / n] } else { a }
}

/// Normalisation façon `_norm` (opentrack_out) : division par 1 si la norme est nulle.
#[inline]
pub fn norm1(a: V3) -> V3 {
    let mut n = dot(a, a).sqrt();
    if n == 0.0 {
        n = 1.0;
    }
    [a[0] / n, a[1] / n, a[2] / n]
}

/// Composante de `v` perpendiculaire à `axis` (unitaire).
#[inline]
pub fn reject(v: V3, axis: V3) -> V3 {
    sub(v, scale(axis, dot(v, axis)))
}

#[inline]
pub fn mv(m: &M3, v: V3) -> V3 {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Produit matriciel. La somme part de 0.0 comme `sum()` en Python (un zéro négatif devient
/// positif), ce qui compte pour le signe d'un `atan2` sur une valeur exactement nulle.
pub fn mm(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = 0.0 + a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    r
}

pub fn mt(a: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[j][i];
        }
    }
    r
}

/// Moyenne composante par composante, sommée dans l'ordre comme en Python.
pub fn mean<'a, I>(items: I) -> V3
where
    I: IntoIterator<Item = &'a V3>,
    I::IntoIter: Clone,
{
    let it = items.into_iter();
    let n = it.clone().count() as f64;
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut s = 0.0;
        for v in it.clone() {
            s += v[k];
        }
        *o = s / n;
    }
    out
}

/// Reste flottant avec la sémantique de Python (signe du diviseur), y compris pour zéro.
pub fn py_mod(a: f64, b: f64) -> f64 {
    let mut r = a % b;
    if r != 0.0 {
        if (r < 0.0) != (b < 0.0) {
            r += b;
        }
    } else {
        r = 0.0_f64.copysign(b);
    }
    r
}

#[inline]
pub fn wrap180(a: f64) -> f64 {
    py_mod(a + 180.0, 360.0) - 180.0
}

#[inline]
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    // min(max(v, lo), hi) de Python, sans paniquer si lo > hi
    let v = if v > lo { v } else { lo };
    if v < hi { v } else { hi }
}

/// Arrondi décimal correct (comme `round(x, n)` en Python : valeur binaire exacte, demi au pair).
pub fn round_dec(x: f64, digits: usize) -> f64 {
    format!("{:.*}", digits, x).parse().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_modulo() {
        assert_eq!(py_mod(-1.0, 360.0), 359.0);
        assert_eq!(py_mod(361.0, 360.0), 1.0);
        assert_eq!(wrap180(190.0), -170.0);
        assert_eq!(wrap180(-190.0), 170.0);
        assert_eq!(wrap180(180.0), -180.0);
    }

    #[test]
    fn rounding_matches_python() {
        assert_eq!(round_dec(0.123_46, 4), 0.1235);
        assert_eq!(round_dec(-0.986_34, 4), -0.9863);
        assert_eq!(round_dec(2.5, 0), 2.0); // demi au pair, comme Python
    }
}
