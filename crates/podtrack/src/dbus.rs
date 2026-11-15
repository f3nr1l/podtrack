//! Service D-Bus de session `io.github.f3nr1l.PodTrack` : commandes et état, pour la ligne de
//! commande, l'interface graphique et les raccourcis.

use crate::hub::Hub;
use podtrack_core::calibration::{DeviceCalibration, GameCalibration};
use std::sync::Arc;
use zbus::fdo;

/// Distinct de l'identifiant de la fenêtre GTK (io.github.f3nr1l.PodTrack), qui réclame ce nom-là.
pub const NAME: &str = "io.github.f3nr1l.PodTrack.Daemon";
pub const PATH: &str = "/io/github/f3nr1l/PodTrack";
pub const IFACE: &str = "io.github.f3nr1l.PodTrack1";

pub struct Api {
    hub: Arc<Hub>,
}

fn err(e: anyhow::Error) -> fdo::Error {
    fdo::Error::Failed(format!("{e:#}"))
}

#[zbus::interface(name = "io.github.f3nr1l.PodTrack1")]
impl Api {
    /// Recentre la tête (la position actuelle devient le centre).
    fn recenter(&self) {
        self.hub.lock().recenter();
    }

    /// Démarre le suivi (ouvre le canal des AirPods).
    fn start(&self) {
        self.hub.lock().active = true;
        self.hub.notify();
    }

    /// Arrête le suivi : canal fermé, poses figées (les jeux recentrent).
    fn stop(&self) {
        self.hub.lock().active = false;
        self.hub.notify();
    }

    fn toggle(&self) -> bool {
        let mut s = self.hub.lock();
        s.active = !s.active;
        let a = s.active;
        drop(s);
        self.hub.notify();
        a
    }

    /// Commande nommée : left_click, toggle_scroll, toggle_mouse, recenter…
    fn command(&self, cmd: &str) {
        self.hub.lock().command(cmd);
    }

    fn quit(&self) {
        self.hub.lock().quit = true;
        self.hub.notify();
    }

    /// Relit la configuration tout de suite.
    fn reload(&self) {
        self.hub.lock().reload_if_changed();
    }

    /// État complet en JSON.
    fn status(&self) -> String {
        self.hub.lock().status().to_string()
    }

    /// Lacet, tangage, roulis actuels (°), pour un aperçu rapide.
    fn head(&self) -> (f64, f64, f64) {
        let h = self.hub.lock().head;
        (h[0], h[1], h[2])
    }

    /// Choisit le traqueur d'un jeu (`podtrack` ou `other`) ; le jeu doit être fermé.
    fn set_tracker(&self, game: &str, tracker: &str) -> fdo::Result<String> {
        let mut s = self.hub.lock();
        let msg = crate::games::set_tracker(&mut s, game, tracker).map_err(err)?;
        Ok(msg)
    }

    /// Calibration de l'appareil (7 étapes) : démarrer, état (JSON), annuler.
    fn device_calibration_start(&self) -> fdo::Result<()> {
        let mut s = self.hub.lock();
        if s.profile_key.is_none() {
            return Err(fdo::Error::Failed("AirPods non connectés".into()));
        }
        let now = s.now();
        s.device_calib = Some(DeviceCalibration::new(s.cfg.device_calibration.clone(), now));
        Ok(())
    }

    fn device_calibration_state(&self) -> String {
        let s = self.hub.lock();
        serde_json::to_string(&s.device_calib.as_ref().map(|c| c.state())).unwrap_or_default()
    }

    fn device_calibration_cancel(&self) {
        self.hub.lock().device_calib = None;
    }

    /// Calibration de l'amplitude par points à l'écran ; `target` = `baseline` ou un jeu.
    fn game_calibration_start(&self, target: &str) -> fdo::Result<()> {
        let mut s = self.hub.lock();
        if target != "baseline" && target != "mouse" && !s.cfg.games.contains_key(target) {
            return Err(fdo::Error::Failed(format!("jeu inconnu : {target}")));
        }
        s.tracker_suspend_autocenter(true);
        s.game_calib = Some((target.to_string(), GameCalibration::new(s.cfg.game_calibration.clone())));
        Ok(())
    }

    fn game_calibration_state(&self) -> String {
        let s = self.hub.lock();
        serde_json::to_string(&s.game_calib_state()).unwrap_or_default()
    }

    /// Enregistre le résultat d'une calibration terminée et la clôt.
    fn game_calibration_apply(&self, target_side: f64, target_vert: f64) -> fdo::Result<String> {
        let mut s = self.hub.lock();
        let msg = crate::games::apply_game_calibration(&mut s, target_side, target_vert).map_err(err)?;
        Ok(msg)
    }

    fn game_calibration_cancel(&self) {
        let mut s = self.hub.lock();
        s.game_calib = None;
        s.tracker_suspend_autocenter(false);
    }
}

pub fn serve(hub: Arc<Hub>) -> anyhow::Result<zbus::blocking::Connection> {
    let conn = zbus::blocking::connection::Builder::session()?
        .name(NAME)?
        .serve_at(PATH, Api { hub })?
        .build()
        .map_err(|e| anyhow::anyhow!("service D-Bus {NAME} : {e} (un démon tourne déjà ?)"))?;
    Ok(conn)
}

/// Le démon est-il lancé ? (sans le démarrer par activation D-Bus)
pub fn running() -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else { return false };
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return false };
    zbus::names::BusName::try_from(NAME).ok().and_then(|n| proxy.name_has_owner(n).ok()).unwrap_or(false)
}

/// Démarre le démon : service utilisateur systemd, à défaut processus détaché.
pub fn start_daemon() -> anyhow::Result<()> {
    let ok = crate::sandbox::flatpak_id().is_none()
        && std::process::Command::new("systemctl").args(["--user", "start", "podtrack.service"]).status().is_ok_and(|s| s.success());
    if !ok {
        std::process::Command::new(std::env::current_exe()?).arg("daemon").stdin(std::process::Stdio::null()).spawn()?;
    }
    Ok(())
}

/// Client : appelle une méthode du démon.
pub fn call<R>(method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> anyhow::Result<R>
where
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    let conn = zbus::blocking::Connection::session()?;
    let reply = conn
        .call_method(Some(NAME), PATH, Some(IFACE), method, body)
        .map_err(|e| match e {
            zbus::Error::MethodError(name, Some(msg), _) if name.as_str().ends_with("ServiceUnknown") => {
                anyhow::anyhow!("le démon podtrack ne tourne pas ({msg})")
            }
            zbus::Error::MethodError(_, Some(msg), _) => anyhow::anyhow!("{msg}"),
            e => anyhow::anyhow!("{e}"),
        })?;
    Ok(reply.body().deserialize()?)
}
