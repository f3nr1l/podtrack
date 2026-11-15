//! Démon : fil Bluetooth (session AACP, relances, reconnexion), fil de sortie à cadence fixe,
//! service D-Bus. S'arrête sur `quit` (D-Bus, signal).

use crate::hub::{Conn, Hub, model_info};
use podtrack_bt::bluez::Bluez;
use podtrack_bt::session::{self, Session};
use podtrack_core::protocol::{Event, Place};
use std::sync::Arc;
use std::time::Duration;

fn bt_loop(hub: Arc<Hub>) {
    let mut backoff = 0.0;
    let mut hint: Option<String> = None;
    // réouvertures consécutives sans aucune donnée
    let mut dry_reopens: u32 = 0;
    loop {
        let (active, quit, cfg) = {
            let s = hub.lock();
            (s.active, s.quit, s.cfg.bluetooth.clone())
        };
        if quit {
            return;
        }
        if !active {
            {
                let mut s = hub.lock();
                s.conn = Conn::Stopped;
            }
            let g = hub.lock();
            let _ = hub.wake.wait_timeout(g, Duration::from_millis(500));
            continue;
        }
        if cfg.bt_reset_after_reopens > 0 && dry_reopens >= cfg.bt_reset_after_reopens {
            dry_reopens = 0;
            if let (Ok(bz), Some(addr)) = (Bluez::system(), hint.as_deref()) {
                if let Ok(Some(dev)) = bz.device(addr) {
                    log::info!("capteurs muets malgré les réouvertures : déconnexion et reconnexion Bluetooth de {}", dev.address);
                    bz.disconnect(&dev, Duration::from_secs(5));
                    std::thread::sleep(Duration::from_secs(2));
                    bz.connect(&dev, Duration::from_secs_f64(cfg.bt_connect_timeout_s));
                }
            }
        }
        let opened = Bluez::system().and_then(|bz| Session::open(&bz, &cfg, hint.as_deref()));
        let mut sess = match opened {
            Ok(s) => s,
            Err(e) => {
                backoff = session::next_backoff(backoff, &cfg);
                let msg = format!("{e:#}");
                log::info!("connexion impossible ({msg}), nouvel essai dans {backoff:.0} s");
                {
                    let mut s = hub.lock();
                    s.conn = Conn::Searching(msg);
                }
                let g = hub.lock();
                let _ = hub.wake.wait_timeout(g, Duration::from_secs_f64(backoff));
                continue;
            }
        };
        backoff = 0.0;
        hint = Some(sess.device.address.clone());
        {
            let mut s = hub.lock();
            s.set_device(Some(model_info(&sess.device)));
            s.conn = Conn::Connected;
            s.last_error = None;
        }
        sess.start_tracking();
        let mut last_start = hub.lock().now();
        let mut last_motion = last_start;
        let mut got_motion = false;
        let err = loop {
            let events = match sess.poll(Duration::from_millis(200)) {
                Ok(ev) => ev,
                Err(e) => break Some(format!("{e:#}")),
            };
            let mut s = hub.lock();
            let now = s.now();
            if events.iter().any(|e| matches!(e, Event::Motion(b) if !b.is_empty())) {
                last_motion = now;
                got_motion = true;
                dry_reopens = 0;
                s.conn = Conn::Receiving;
            }
            s.on_events(events, now);
            if s.quit || !s.active {
                break None;
            }
            let wait = if s.place == Some(Place::Case) { s.cfg.bluetooth.relaunch_case_s } else { s.cfg.bluetooth.relaunch_s };
            // relances sans effet : on rouvre le canal (sans couper la liaison Bluetooth ni l'audio)
            let reopen = s.cfg.bluetooth.reopen_after_s;
            if reopen > 0.0 && s.place != Some(Place::Case) && now - last_motion > reopen {
                if !got_motion {
                    dry_reopens += 1;
                }
                break Some(format!("pas de données depuis {:.0} s : réouverture du canal", now - last_motion));
            }
            let restart = std::mem::take(&mut s.restart_stream);
            if restart || (now - last_motion > wait && now - last_start > wait) {
                if !restart {
                    log::info!("pas de données de mouvement : relance du flux");
                    s.conn = Conn::NoData;
                }
                sess.cfg = s.cfg.bluetooth.clone();
                drop(s);
                sess.start_tracking();
                last_start = hub.lock().now();
            }
        };
        sess.close();
        let mut s = hub.lock();
        if let Some(e) = err {
            log::warn!("canal perdu : {e}");
            s.last_error = Some(e);
        }
        s.conn = Conn::Stopped;
        drop(s);
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn out_loop(hub: Arc<Hub>) {
    let mut next_reload = 0.0;
    let mut next = 0.0;
    loop {
        let (period, quit) = {
            let mut s = hub.lock();
            if s.quit {
                return;
            }
            let now = s.now();
            if now >= next_reload {
                if s.reload_if_changed() {
                    hub.notify();
                }
                next_reload = now + 1.0;
            }
            if s.device_calib.is_some() {
                s.device_calib_tick(now);
            }
            s.tick_outputs(now);
            (1.0 / s.cfg.output.rate_hz.clamp(1.0, 1000.0), s.quit)
        };
        if quit {
            return;
        }
        // échéance fixe, sans rafale de rattrapage après un retard
        let now = hub.lock().now();
        next += period;
        if next <= now || next > now + period {
            next = now + period;
        }
        std::thread::sleep(Duration::from_secs_f64((next - now).max(0.0005)));
    }
}

pub fn run(hub: Arc<Hub>, with_tray: bool) -> anyhow::Result<()> {
    // avant tout autre fil : ils héritent du masque, seul le fil dédié reçoit les signaux
    crate::signals::on_terminate({
        let h = hub.clone();
        move || {
            h.lock().quit = true;
            h.notify();
        }
    });
    let _conn = crate::dbus::serve(hub.clone())?;
    let hk = hub.lock().cfg.hotkeys.clone();
    if let Err(e) = crate::hotkeys::apply(&hk) {
        log::warn!("raccourcis non écrits : {e:#}");
    }
    let bt = { let h = hub.clone(); std::thread::Builder::new().name("bluetooth".into()).spawn(move || bt_loop(h))? };
    let out = { let h = hub.clone(); std::thread::Builder::new().name("sorties".into()).spawn(move || out_loop(h))? };
    if with_tray {
        crate::tray::spawn(hub.clone());
    }
    let _ = bt.join();
    let _ = out.join();
    log::info!("arrêté");
    Ok(())
}
