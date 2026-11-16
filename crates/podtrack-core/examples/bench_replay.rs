//! Chronomètre le pipeline (moteur, suivi, profil) sur la capture de référence, échantillon par
//! échantillon, pour comparer avec MousePods. Usage : cargo run --release --example bench_replay [reps]

use podtrack_core::engine::{DeviceProfile, EngineConfig, MotionEngine, SampleClock};
use podtrack_core::profile::{GameProfile, ProfileFilter};
use podtrack_core::tracker::{HeadTracker, TrackerConfig};
use serde_json::Value;
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let reps: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(20);
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/");
    let read = |f: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(format!("{dir}{f}")).unwrap()).unwrap() };
    let cap = read("airpods3_motion_capture.json");
    let reference = read("reference_airpods3.json");
    let device: DeviceProfile = serde_json::from_value(reference["device"].clone()).unwrap();
    let mut game: GameProfile = serde_json::from_value(reference["game"].clone()).unwrap();
    game.axes.yaw.curve = 1.3;
    game.axes.pitch.curve = 1.3;
    let v3 = |v: &Value| [v[0].as_f64().unwrap(), v[1].as_f64().unwrap(), v[2].as_f64().unwrap()];
    let pre: Vec<_> = cap.as_array().unwrap().iter().map(|s| {
        let gy = v3(&s["gyr"]);
        let a = |i: usize| s["att"][i].as_i64().unwrap() as i16;
        (s["t"].as_f64().unwrap(), s["ts"].as_u64().unwrap(), [gy[0] * 0.122, gy[1] * 0.122, gy[2] * 0.122], v3(&s["grav"]), [a(0), a(1), a(2)])
    }).collect();
    let mut per = Vec::with_capacity(pre.len() * reps);
    for _ in 0..reps {
        let mut eng = MotionEngine::new(EngineConfig::default(), device.clone());
        let mut trk = HeadTracker::new(TrackerConfig::default());
        let mut flt = ProfileFilter::new(game.clone());
        let mut clock = SampleClock::new();
        for &(now, ts, w, g, att) in &pre {
            let t0 = Instant::now();
            let dt = clock.dt(ts, now);
            let rates = eng.step(w, g, dt);
            trk.update(rates, dt, eng.g, Some(eng.lat()), eng.signs(), Some(att), now);
            let h = trk.head;
            black_box(flt.apply([0.0, 0.0, 0.0, h[0], h[1], h[2]], dt));
            per.push(t0.elapsed().as_nanos() as f64);
        }
    }
    per.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = per.len();
    println!("{{\"impl\": \"rust\", \"samples\": {n}, \"mean_us\": {:.3}, \"p50_us\": {:.3}, \"p99_us\": {:.3}, \"max_us\": {:.3}}}",
        per.iter().sum::<f64>() / n as f64 / 1e3, per[n / 2] / 1e3, per[n * 99 / 100] / 1e3, per[n - 1] / 1e3);
}
