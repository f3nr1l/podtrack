//! Comparaison différentielle avec MousePods : la capture réelle rejouée dans le code Python
//! (`tools/make_reference.py`) et dans ce portage doit donner les mêmes nombres.

use podtrack_core::engine::{DeviceProfile, EngineConfig, MotionEngine, SampleClock};
use podtrack_core::profile::{GameProfile, ProfileFilter};
use podtrack_core::tracker::{HeadTracker, TrackerConfig, Used};
use serde_json::Value;

fn load(name: &str) -> Value {
    let p = format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn v3(v: &Value) -> [f64; 3] {
    [v[0].as_f64().unwrap(), v[1].as_f64().unwrap(), v[2].as_f64().unwrap()]
}

/// Écart relatif toléré : les deux côtés appellent la même libm ; on laisse une marge d'arrondi.
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

#[test]
fn replay_airpods3_capture_matches_python() {
    let cap = load("airpods3_motion_capture.json");
    let reference = load("reference_airpods3.json");
    let device: DeviceProfile = serde_json::from_value(reference["device"].clone()).unwrap();
    let game: GameProfile = serde_json::from_value(reference["game"].clone()).unwrap();
    let rows = reference["rows"].as_array().unwrap();

    let mut eng = MotionEngine::new(EngineConfig::default(), device);
    let mut trk = HeadTracker::new(TrackerConfig::default());
    let mut flt = ProfileFilter::new(game);
    let mut clock = SampleClock::new();
    let mut worst = 0.0_f64;
    let mut quat = 0;

    for (i, (s, r)) in cap.as_array().unwrap().iter().zip(rows).enumerate() {
        let now = s["t"].as_f64().unwrap();
        let ts = s["ts"].as_u64().unwrap();
        let gyr = v3(&s["gyr"]);
        let w = [gyr[0] * 0.122, gyr[1] * 0.122, gyr[2] * 0.122];
        let g = v3(&s["grav"]);
        let att = [s["att"][0].as_i64().unwrap() as i16, s["att"][1].as_i64().unwrap() as i16, s["att"][2].as_i64().unwrap() as i16];

        let dt = clock.dt(ts, now);
        assert!(close(dt, r["dt"].as_f64().unwrap()), "ligne {i} : dt {dt} / {}", r["dt"]);
        let rates = eng.step(w, g, dt);
        let out = trk.update(rates, dt, eng.g, Some(eng.lat()), eng.signs(), Some(att), now);
        let head = trk.head;
        let pose = flt.apply([0.0, 0.0, 0.0, head[0], head[1], head[2]], dt);

        let checks: [(&str, &[f64], &Value); 5] = [
            ("rates", &rates, &r["rates"]),
            ("bias", &eng.bias, &r["bias"]),
            ("head", &head, &r["head"]),
            ("out", &out, &r["out"]),
            ("game", &pose, &r["game"]),
        ];
        for (name, got, want) in checks {
            for (k, g) in got.iter().enumerate() {
                let w = want[k].as_f64().unwrap();
                worst = worst.max((g - w).abs());
                assert!(close(*g, w), "ligne {i} {name}[{k}] : rust {g} / python {w}");
            }
        }
        assert_eq!(eng.bias_ok, r["bias_ok"].as_bool().unwrap(), "ligne {i} bias_ok");
        let used = match trk.used { Some(Used::Quaternion) => "quaternion", Some(Used::Gyro) => "gyro", None => "" };
        if used == "quaternion" {
            quat += 1;
        }
        assert_eq!(Some(used), r["used"].as_str().or(Some("")), "ligne {i} source");
    }
    eprintln!("{} lignes, {} par quaternion, plus grand écart absolu {worst:e}", rows.len(), quat);
    assert!(quat > 1000);
}
