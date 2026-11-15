//! Périphériques virtuels (uinput) : souris (relative et absolue) et manette « tête » pour les
//! jeux Linux natifs qui lient la caméra à des axes de manette (disposition de la sortie
//! « libevdev joystick » d'opentrack).

use anyhow::Result;
use evdev::uinput::VirtualDevice;
use evdev::{AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode, UinputAbsSetup};
use podtrack_core::tracker::Pose6;
use serde::{Deserialize, Serialize};

pub const ABS_MAX: i32 = 32767;

pub struct VirtualMouse {
    rel: VirtualDevice,
    abs: VirtualDevice,
    fx: f64,
    fy: f64,
    wheel: f64,
    last_abs: Option<(i32, i32)>,
}

fn buttons() -> AttributeSet<KeyCode> {
    let mut k = AttributeSet::<KeyCode>::new();
    for c in [KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT, KeyCode::BTN_MIDDLE] {
        k.insert(c);
    }
    k
}

impl VirtualMouse {
    pub fn new(name: &str) -> Result<Self> {
        let mut rels = AttributeSet::<RelativeAxisCode>::new();
        for r in [RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y, RelativeAxisCode::REL_WHEEL, RelativeAxisCode::REL_HWHEEL] {
            rels.insert(r);
        }
        let rel = VirtualDevice::builder()?.name(&format!("{name} (souris)")).with_keys(&buttons())?.with_relative_axes(&rels)?.build()?;
        let info = AbsInfo::new(0, 0, ABS_MAX, 0, 0, 0);
        let abs = VirtualDevice::builder()?
            .name(&format!("{name} (pointeur)"))
            .with_keys(&buttons())?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, info))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, info))?
            .build()?;
        Ok(Self { rel, abs, fx: 0.0, fy: 0.0, wheel: 0.0, last_abs: None })
    }

    /// Déplacement relatif ; les fractions de pixel sont reportées.
    pub fn move_rel(&mut self, dx: f64, dy: f64) -> Result<()> {
        self.fx += dx;
        self.fy += dy;
        let (ix, iy) = (self.fx.trunc() as i32, self.fy.trunc() as i32);
        self.fx -= ix as f64;
        self.fy -= iy as f64;
        let mut ev = Vec::new();
        if ix != 0 {
            ev.push(InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_X.0, ix));
        }
        if iy != 0 {
            ev.push(InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_Y.0, iy));
        }
        if !ev.is_empty() {
            self.rel.emit(&ev)?;
            self.last_abs = None;
        }
        Ok(())
    }

    pub fn move_abs(&mut self, fx: f64, fy: f64) -> Result<()> {
        let ix = (fx.clamp(0.0, 1.0) * ABS_MAX as f64) as i32;
        let iy = (fy.clamp(0.0, 1.0) * ABS_MAX as f64) as i32;
        if self.last_abs == Some((ix, iy)) {
            return Ok(());
        }
        self.last_abs = Some((ix, iy));
        self.abs.emit(&[
            InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_X.0, ix),
            InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Y.0, iy),
        ])?;
        Ok(())
    }

    pub fn click(&mut self, button: &str, press_s: f64) -> Result<()> {
        let code = match button {
            "right" => KeyCode::BTN_RIGHT,
            "middle" => KeyCode::BTN_MIDDLE,
            _ => KeyCode::BTN_LEFT,
        };
        for v in [1, 0] {
            self.rel.emit(&[InputEvent::new(EventType::KEY.0, code.0, v)])?;
            std::thread::sleep(std::time::Duration::from_secs_f64(press_s.max(0.0)));
        }
        Ok(())
    }

    pub fn scroll(&mut self, lines: f64) -> Result<()> {
        self.wheel += lines;
        let n = self.wheel.trunc() as i32;
        if n != 0 {
            self.wheel -= n as f64;
            self.rel.emit(&[InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_WHEEL.0, n)])?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JoystickConfig {
    pub enabled: bool,
    pub name: String,
    /// Pleine échelle des axes : lacet, tangage, roulis (°) et translation (cm).
    pub full_yaw_deg: f64,
    pub full_pitch_deg: f64,
    pub full_roll_deg: f64,
    pub full_cm: f64,
}

impl Default for JoystickConfig {
    fn default() -> Self {
        Self { enabled: false, name: "podtrack head tracker".into(), full_yaw_deg: 180.0, full_pitch_deg: 90.0, full_roll_deg: 180.0, full_cm: 100.0 }
    }
}

pub const AXIS_MAX: i32 = 65535;
pub const CENTER: i32 = 32767;

/// Valeur physique (° ou cm) → position d'axe 0..65535 (bornée).
pub fn axis_value(v: f64, full: f64) -> i32 {
    if !v.is_finite() {
        return CENTER;
    }
    (CENTER as f64 + v / full * CENTER as f64).clamp(0.0, AXIS_MAX as f64).round() as i32
}

pub struct VirtualJoystick {
    dev: VirtualDevice,
    last: [i32; 6],
    pub writes: u64,
}

const JOY_AXES: [AbsoluteAxisCode; 6] = [
    AbsoluteAxisCode::ABS_X,
    AbsoluteAxisCode::ABS_Y,
    AbsoluteAxisCode::ABS_Z,
    AbsoluteAxisCode::ABS_RX,
    AbsoluteAxisCode::ABS_RY,
    AbsoluteAxisCode::ABS_RZ,
];

impl VirtualJoystick {
    pub fn new(name: &str) -> Result<Self> {
        let info = AbsInfo::new(CENTER, 0, AXIS_MAX, 0, 0, 0);
        let mut keys = AttributeSet::<KeyCode>::new();
        keys.insert(KeyCode::BTN_TRIGGER); // pour que udev la classe « manette »
        let mut b = VirtualDevice::builder()?.name(name).with_keys(&keys)?;
        for a in JOY_AXES {
            b = b.with_absolute_axis(&UinputAbsSetup::new(a, info))?;
        }
        Ok(Self { dev: b.build()?, last: [-1; 6], writes: 0 })
    }

    pub fn write(&mut self, p: &Pose6, c: &JoystickConfig) -> Result<()> {
        let fulls = [c.full_cm, c.full_cm, c.full_cm, c.full_yaw_deg, c.full_pitch_deg, c.full_roll_deg];
        let mut ev = Vec::new();
        for i in 0..6 {
            let v = axis_value(p[i], fulls[i]);
            if v != self.last[i] {
                self.last[i] = v;
                ev.push(InputEvent::new(EventType::ABSOLUTE.0, JOY_AXES[i].0, v));
            }
        }
        if !ev.is_empty() {
            self.dev.emit(&ev)?;
            self.writes += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn axis_scale() {
        assert_eq!(super::axis_value(0.0, 180.0), super::CENTER);
        assert_eq!(super::axis_value(180.0, 180.0), 65534);
        assert_eq!(super::axis_value(-500.0, 180.0), 0);
        assert_eq!(super::axis_value(f64::NAN, 180.0), super::CENTER);
    }
}
