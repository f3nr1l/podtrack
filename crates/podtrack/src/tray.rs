//! Icône dans la barre du haut (StatusNotifierItem, extension AppIndicator de GNOME) : état du
//! suivi, batterie, recentrage, marche / arrêt, traqueur de chaque jeu, fenêtre, quitter.

use crate::config::TrackerChoice;
use crate::hub::{Conn, Hub};
use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip, Tray};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const ICONS: [(&str, &str); 4] = [
    ("podtrack-active", include_str!("../data/icons/podtrack-active.svg")),
    ("podtrack-stopped", include_str!("../data/icons/podtrack-stopped.svg")),
    ("podtrack-searching", include_str!("../data/icons/podtrack-searching.svg")),
    ("podtrack", include_str!("../data/icons/podtrack.svg")),
];

/// Installe les icônes dans `~/.local/share/podtrack/icons` (thème propre à podtrack) et dans le
/// thème de l'utilisateur pour l'icône d'application. Renvoie le dossier du thème.
pub fn install_icons() -> Option<PathBuf> {
    let base = dirs::data_dir()?;
    let theme = base.join("podtrack").join("icons");
    let apps = theme.join("hicolor").join("scalable").join("apps");
    std::fs::create_dir_all(&apps).ok()?;
    for (name, svg) in ICONS {
        let _ = std::fs::write(apps.join(format!("{name}.svg")), svg);
    }
    let user_apps = base.join("icons").join("hicolor").join("scalable").join("apps");
    if std::fs::create_dir_all(&user_apps).is_ok() {
        let _ = std::fs::write(user_apps.join("podtrack.svg"), ICONS[3].1);
    }
    Some(theme)
}

#[derive(Clone, PartialEq, Default)]
struct GameLine {
    id: String,
    name: String,
    ours: bool,
}

#[derive(Clone, PartialEq, Default)]
struct Snapshot {
    active: bool,
    mouse: bool,
    receiving: bool,
    label: String,
    device: String,
    battery: String,
    games: Vec<GameLine>,
}

fn snapshot(hub: &Hub) -> Snapshot {
    let s = hub.lock();
    let now = s.now();
    let receiving = s.active && !s.stale(now);
    let label = if !s.active {
        "arrêté".to_string()
    } else {
        match &s.conn {
            Conn::Receiving if receiving => format!("suivi actif, {:.0} Hz", s.rate_hz),
            Conn::Receiving | Conn::NoData => "connecté, pas de données".into(),
            Conn::Connected => "connecté".into(),
            Conn::Searching(_) => "recherche des AirPods".into(),
            Conn::Stopped => "en attente".into(),
        }
    };
    let device = s.device.as_ref().map(|d| format!("{} ({})", d.name, d.model)).unwrap_or_default();
    let battery = s
        .battery
        .iter()
        .filter(|b| b.state != 0x04)
        .map(|b| {
            let n = match b.part {
                0x01 => "seul",
                0x02 => "droit",
                0x04 => "gauche",
                0x08 => "boîtier",
                _ => "?",
            };
            format!("{n} {}%{}", b.level, if b.state == 0x01 { " ⚡" } else { "" })
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let games = s
        .cfg
        .games
        .iter()
        .map(|(id, g)| GameLine { id: id.clone(), name: g.name.clone(), ours: g.enabled && g.tracker == TrackerChoice::Podtrack })
        .collect();
    Snapshot { active: s.active, mouse: s.cfg.mouse.enabled, receiving, label, device, battery, games }
}

fn notify(text: &str) {
    let _ = std::process::Command::new("notify-send").args(["-a", "podtrack", "-i", "podtrack", "podtrack", text]).spawn();
}

struct PodTray {
    hub: Arc<Hub>,
    snap: Snapshot,
    /// Icônes prêtes (actif, arrêté, recherche), en plusieurs tailles.
    pixmaps: Vec<Vec<Icon>>,
}

/// SVG → images ARGB32 (ordre réseau) aux tailles courantes de la barre.
fn rasterize(svg: &str) -> Vec<Icon> {
    use gtk::gdk_pixbuf::{Pixbuf, PixbufLoader};
    use gtk::prelude::PixbufLoaderExt;
    let mut out = Vec::new();
    for size in [16, 22, 24, 32, 48, 64] {
        let loader = PixbufLoader::with_type("svg").ok();
        let Some(loader) = loader else { continue };
        loader.set_size(size, size);
        if loader.write(svg.as_bytes()).is_err() || loader.close().is_err() {
            continue;
        }
        let Some(pb): Option<Pixbuf> = loader.pixbuf() else { continue };
        let Some(pb) = (if pb.has_alpha() { Some(pb) } else { pb.add_alpha(false, 0, 0, 0).ok() }) else { continue };
        let (w, h, stride, n) = (pb.width() as usize, pb.height() as usize, pb.rowstride() as usize, pb.n_channels() as usize);
        let px = pb.read_pixel_bytes();
        let mut data = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let o = y * stride + x * n;
                data.extend_from_slice(&[px[o + 3], px[o], px[o + 1], px[o + 2]]);
            }
        }
        out.push(Icon { width: w as i32, height: h as i32, data });
    }
    out
}

impl Tray for PodTray {
    fn id(&self) -> String {
        "io.github.f3nr1l.PodTrack".into()
    }

    fn title(&self) -> String {
        "podtrack".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let i = if !self.snap.active {
            1
        } else if self.snap.receiving {
            0
        } else {
            2
        };
        self.pixmaps.get(i).cloned().unwrap_or_default()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip { title: format!("podtrack — {}", self.snap.label), description: self.snap.battery.clone(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        open_window();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut m: Vec<MenuItem<Self>> = vec![StandardItem { label: format!("podtrack — {}", self.snap.label), enabled: false, ..Default::default() }.into()];
        if !self.snap.device.is_empty() {
            m.push(StandardItem { label: self.snap.device.clone(), enabled: false, ..Default::default() }.into());
        }
        if !self.snap.battery.is_empty() {
            m.push(StandardItem { label: format!("Batterie : {}", self.snap.battery), enabled: false, ..Default::default() }.into());
        }
        m.push(MenuItem::Separator);
        m.push(
            CheckmarkItem {
                label: "Suivi de tête".into(),
                checked: self.snap.active,
                activate: Box::new(|t: &mut Self| {
                    let mut s = t.hub.lock();
                    s.active = !s.active;
                    drop(s);
                    t.hub.notify();
                    t.snap = snapshot(&t.hub);
                }),
                ..Default::default()
            }
            .into(),
        );
        m.push(
            CheckmarkItem {
                label: "Souris par la tête".into(),
                checked: self.snap.mouse,
                activate: Box::new(|t: &mut Self| {
                    t.hub.lock().command("toggle_mouse");
                    t.snap = snapshot(&t.hub);
                }),
                ..Default::default()
            }
            .into(),
        );
        m.push(
            StandardItem {
                label: "Recentrer".into(),
                activate: Box::new(|t: &mut Self| t.hub.lock().recenter()),
                ..Default::default()
            }
            .into(),
        );
        if !self.snap.games.is_empty() {
            let items = self
                .snap
                .games
                .iter()
                .map(|g| {
                    let id = g.id.clone();
                    let name = g.name.clone();
                    let ours = g.ours;
                    CheckmarkItem {
                        label: format!("{name} : AirPods"),
                        checked: ours,
                        activate: Box::new(move |t: &mut Self| {
                            let want = if ours { "other" } else { "podtrack" };
                            let res = {
                                let mut s = t.hub.lock();
                                crate::games::set_tracker(&mut s, &id, want)
                            };
                            match res {
                                Ok(msg) => notify(&msg),
                                Err(e) => notify(&format!("{name} : {e:#}")),
                            }
                            t.snap = snapshot(&t.hub);
                        }),
                        ..Default::default()
                    }
                    .into()
                })
                .collect();
            m.push(SubMenu { label: "Jeux (cocher = suivi par les AirPods)".into(), submenu: items, ..Default::default() }.into());
        }
        m.push(MenuItem::Separator);
        m.push(StandardItem { label: "Ouvrir podtrack…".into(), activate: Box::new(|_: &mut Self| open_window()), ..Default::default() }.into());
        m.push(
            StandardItem {
                label: "Quitter".into(),
                activate: Box::new(|t: &mut Self| {
                    t.hub.lock().quit = true;
                    t.hub.notify();
                }),
                ..Default::default()
            }
            .into(),
        );
        m
    }
}

/// Ouvre la fenêtre (`podtrack gui`, processus séparé).
pub fn open_window() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("gui").spawn();
    }
}

/// Affiche l'icône et la tient à jour. Sans extension AppIndicator, le démon continue sans elle.
pub fn spawn(hub: Arc<Hub>) {
    install_icons();
    let pixmaps = ICONS[..3].iter().map(|(_, svg)| rasterize(svg)).collect();
    let tray = PodTray { snap: snapshot(&hub), hub: hub.clone(), pixmaps };
    let handle = match tray.assume_sni_available(true).spawn() {
        Ok(h) => h,
        Err(e) => {
            log::warn!("icône de barre indisponible ({e}) : extension AppIndicator absente ?");
            return;
        }
    };
    std::thread::spawn(move || {
        let mut last = Snapshot::default();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if hub.lock().quit {
                handle.shutdown();
                return;
            }
            let now = snapshot(&hub);
            if now != last {
                let n = now.clone();
                handle.update(move |t: &mut PodTray| t.snap = n);
                last = now;
            }
        }
    });
}
