//! Tests physiques repris de MousePods (tests/test_engine.py) : l'écouteur est simulé dans des
//! orientations aléatoires et tourne réellement ; la gravité mesurée suit.

use podtrack_core::calibration::{profile_from_calibration, CalibAccumulator, DeviceCalibConfig, DeviceCalibration};
use podtrack_core::engine::{DeviceProfile, EngineConfig, MotionEngine};
use podtrack_core::vec::{self, M3, V3};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
    fn gauss(&mut self, s: f64) -> f64 {
        let (u1, u2) = (self.next().max(1e-12), self.next());
        s * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

fn rot(axis: V3, ang: f64) -> M3 {
    let [x, y, z] = vec::vnorm(axis);
    let a = ang.to_radians();
    let (c, s, cc) = (a.cos(), a.sin(), 1.0 - a.cos());
    [
        [c + x * x * cc, x * y * cc - z * s, x * z * cc + y * s],
        [y * x * cc + z * s, c + y * y * cc, y * z * cc - x * s],
        [z * x * cc - y * s, z * y * cc + x * s, c + z * z * cc],
    ]
}

fn to_body(r: &M3, v: V3) -> V3 {
    [0, 1, 2].map(|j| r[0][j] * v[0] + r[1][j] * v[1] + r[2][j] * v[2])
}

fn random_orientation(rng: &mut Rng) -> M3 {
    let axis = [rng.uniform(-1.0, 1.0), rng.uniform(-1.0, 1.0), rng.uniform(-1.0, 1.0)];
    rot(axis, rng.uniform(0.0, 180.0))
}

const UP: V3 = [0.0, 0.0, 1.0];
const LAT: V3 = [0.0, 1.0, 0.0];
const FWD: V3 = [1.0, 0.0, 0.0];

fn body_profile(r: &M3) -> DeviceProfile {
    DeviceProfile { yaw_sign: 1.0, lat: to_body(r, LAT), pitch_sign: 1.0, roll_sign: 1.0 }
}

fn simulate(mut r: M3, w_world: V3, secs: f64, bias: V3, noise: f64, rng: &mut Rng) -> (MotionEngine, V3) {
    let mut eng = MotionEngine::new(EngineConfig::default(), body_profile(&r));
    let speed = vec::len(w_world);
    let mut last = [0.0; 3];
    for _ in 0..(secs * 100.0) as usize {
        let g = vec::scale(to_body(&r, UP), 1000.0);
        let mut w = to_body(&r, w_world);
        if noise > 0.0 {
            w = w.map(|c| c + rng.gauss(noise));
        }
        last = eng.step([w[0] + bias[0], w[1] + bias[1], w[2] + bias[2]], g, 0.01);
        if speed > 0.0 {
            r = vec::mm(&rot(w_world, speed * 0.01), &r);
        }
    }
    (eng, last)
}

#[test]
fn yaw_pitch_roll_independent_of_orientation() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for _ in 0..25 {
        let r = random_orientation(&mut rng);
        let (_, o) = simulate(r, vec::scale(UP, 30.0), 0.5, [0.0; 3], 0.0, &mut rng);
        assert!((o[0] - 30.0).abs() < 0.5 && o[1].abs() < 0.5 && o[2].abs() < 0.5, "{o:?}");
        let (_, o) = simulate(r, vec::scale(LAT, -20.0), 0.5, [0.0; 3], 0.0, &mut rng);
        assert!((o[1] + 20.0).abs() < 0.5 && o[0].abs() < 0.5, "{o:?}");
        let (_, o) = simulate(r, vec::scale(FWD, 25.0), 0.5, [0.0; 3], 0.0, &mut rng);
        assert!((o[2].abs() - 25.0).abs() < 0.5 && o[1].abs() < 0.5, "{o:?}");
    }
}

#[test]
fn bias_removed_when_still_and_not_corrupted_by_motion() {
    let mut rng = Rng(3);
    let r = random_orientation(&mut rng);
    let (eng, o) = simulate(r, [0.0; 3], 3.0, [4.0, -6.0, 2.5], 0.3, &mut rng);
    assert!(eng.bias_ok);
    assert!(o.iter().all(|v| v.abs() < 1.0), "{o:?}");

    let r = random_orientation(&mut rng);
    let mut eng = MotionEngine::new(EngineConfig::default(), body_profile(&r));
    let g = vec::scale(to_body(&r, UP), 1000.0);
    for i in 0..300 {
        let w = to_body(&r, vec::scale(UP, 60.0 * (i as f64 / 10.0).sin()));
        eng.step(w, g, 0.01);
    }
    assert!(vec::len(eng.bias) < 1.0);
}

#[test]
fn profile_from_simulated_gestures() {
    let mut rng = Rng(5);
    for _ in 0..20 {
        let r = random_orientation(&mut rng);
        let up_b = to_body(&r, UP);
        let bias = [rng.uniform(-5.0, 5.0), rng.uniform(-5.0, 5.0), rng.uniform(-5.0, 5.0)];
        let integrate = |w_world: V3| {
            let mut acc = CalibAccumulator::new(bias, up_b);
            let w = to_body(&r, w_world);
            for _ in 0..60 {
                acc.add([w[0] + bias[0], w[1] + bias[1], w[2] + bias[2]], 0.01);
            }
            acc
        };
        let left = integrate(vec::scale(UP, 50.0));
        let up = integrate(vec::scale(LAT, -40.0));
        let roll = integrate(vec::scale(FWD, 40.0));
        let prof = profile_from_calibration(up_b, left.yaw, up.horiz, roll.horiz);
        let mut eng = MotionEngine::new(EngineConfig::default(), prof);
        let g = vec::scale(up_b, 1000.0);
        let mut rates = |w_world: V3| {
            let mut o = [0.0; 3];
            for _ in 0..10 {
                o = eng.step(to_body(&r, w_world), g, 0.01);
            }
            o
        };
        assert!(rates(vec::scale(UP, 30.0))[0] < -25.0);
        assert!(rates(vec::scale(LAT, -30.0))[1] > 25.0);
        assert!(rates(vec::scale(FWD, 30.0))[2] > 25.0);
    }
}

/// La machine à états de la calibration de l'appareil, menée de bout en bout.
#[test]
fn device_calibration_state_machine() {
    let mut rng = Rng(11);
    let r = random_orientation(&mut rng);
    let g = vec::scale(to_body(&r, UP), 1000.0);
    let bias = [1.5, -0.7, 0.4];
    let mut cal = DeviceCalibration::new(DeviceCalibConfig::default(), 0.0);
    let mut t = 0.0;
    let mut run = |cal: &mut DeviceCalibration, w_world: V3, secs: f64| {
        let w = to_body(&r, w_world);
        let mut st = cal.state();
        for i in 0..(secs * 100.0) as usize {
            t += 0.01;
            cal.feed(t, 0.01, [w[0] + bias[0], w[1] + bias[1], w[2] + bias[2]], g);
            if i % 4 == 0 {
                st = cal.tick(t);
            }
        }
        st
    };
    run(&mut cal, [0.0; 3], 1.6); // immobile
    run(&mut cal, vec::scale(UP, 40.0), 0.7); // gauche
    run(&mut cal, [0.0; 3], 1.6);
    run(&mut cal, vec::scale(LAT, -40.0), 0.7); // haut
    run(&mut cal, [0.0; 3], 1.6);
    run(&mut cal, vec::scale(FWD, 40.0), 0.7); // épaule droite
    let st = run(&mut cal, [0.0; 3], 1.6);
    assert!(st.done, "{st:?}");
    let prof = st.profile.unwrap();
    assert_eq!(prof.yaw_sign, -1.0);
    let lat_true = vec::vnorm(vec::scale(to_body(&r, LAT), -1.0));
    assert!(vec::dot(vec::vnorm(prof.lat), lat_true) > 0.99, "{prof:?}");
}
