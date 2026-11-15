//! BlueZ par D-Bus (bus système) : appareils Apple appairés, connexion, déconnexion.

use anyhow::Result;
use std::time::Duration;
use zbus::blocking::Connection;
use zbus::blocking::fdo::ObjectManagerProxy;
use zbus::zvariant::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct AppleDevice {
    pub path: String,
    pub address: String,
    pub name: String,
    /// Identifiant produit (hex minuscule), tiré du Modalias `bluetooth:v004Cp<pid>`.
    pub pid: String,
    pub connected: bool,
}

pub struct Bluez {
    conn: Connection,
}

fn as_str(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// `bluetooth:v004Cp2027d242F` → `2027` (Apple seulement).
pub fn apple_pid(modalias: &str) -> Option<String> {
    let rest = modalias.strip_prefix("bluetooth:v")?;
    if !rest.get(..4)?.eq_ignore_ascii_case("004c") {
        return None;
    }
    let p = rest.get(4..9)?;
    let p = p.strip_prefix('p').or_else(|| p.strip_prefix('P'))?;
    p.bytes().all(|c| c.is_ascii_hexdigit()).then(|| p.to_lowercase())
}

impl Bluez {
    pub fn system() -> Result<Self> {
        Ok(Self { conn: Connection::system()? })
    }

    /// Appareils Apple appairés, triés par adresse.
    pub fn apple_devices(&self) -> Result<Vec<AppleDevice>> {
        let om = ObjectManagerProxy::builder(&self.conn).destination("org.bluez")?.path("/")?.build()?;
        let mut out = Vec::new();
        for (path, ifaces) in om.get_managed_objects()? {
            let Some(dev) = ifaces.iter().find(|(k, _)| k.as_str() == "org.bluez.Device1").map(|(_, v)| v) else { continue };
            let get = |k: &str| dev.get(k).map(|v| &**v);
            if get("Paired").and_then(as_bool) != Some(true) {
                continue;
            }
            let Some(pid) = get("Modalias").and_then(as_str).as_deref().and_then(apple_pid) else { continue };
            out.push(AppleDevice {
                path: path.to_string(),
                address: get("Address").and_then(as_str).unwrap_or_default(),
                name: get("Name").and_then(as_str).or_else(|| get("Alias").and_then(as_str)).unwrap_or_default(),
                pid,
                connected: get("Connected").and_then(as_bool).unwrap_or(false),
            });
        }
        out.sort_by(|a, b| a.address.cmp(&b.address));
        Ok(out)
    }

    pub fn device(&self, address: &str) -> Result<Option<AppleDevice>> {
        Ok(self.apple_devices()?.into_iter().find(|d| d.address.eq_ignore_ascii_case(address)))
    }

    fn call(&self, path: &str, method: &str, timeout: Duration) -> bool {
        let conn = self.conn.clone();
        let path = path.to_string();
        let method = method.to_string();
        // l'appel peut durer : borné par un fil et une attente
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let r = conn.call_method(Some("org.bluez"), path.as_str(), Some("org.bluez.Device1"), method.as_str(), &());
            let _ = tx.send(r.is_ok());
        });
        rx.recv_timeout(timeout).unwrap_or(false)
    }

    pub fn connect(&self, dev: &AppleDevice, timeout: Duration) -> bool {
        self.call(&dev.path, "Connect", timeout);
        self.device(&dev.address).ok().flatten().is_some_and(|d| d.connected)
    }

    pub fn disconnect(&self, dev: &AppleDevice, timeout: Duration) {
        self.call(&dev.path, "Disconnect", timeout);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn modalias() {
        assert_eq!(super::apple_pid("bluetooth:v004Cp2027d242F").as_deref(), Some("2027"));
        assert_eq!(super::apple_pid("bluetooth:v004cp200Ad0001").as_deref(), Some("200a"));
        assert_eq!(super::apple_pid("usb:v046Dp0001"), None);
        assert_eq!(super::apple_pid("bluetooth:v0075p0001d0001"), None);
    }
}
