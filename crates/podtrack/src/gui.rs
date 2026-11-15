//! Fenêtre GNOME (GTK4 + libadwaita) : état et aperçu de la tête, jeux et profils, calibrations
//! (dont la calibration par points à l'écran), sorties, raccourcis, réglages avancés.
//! Elle parle au démon par D-Bus et modifie `config.toml`, que le démon relit à chaud.

use crate::config::{self, Config, GameKind, TrackerChoice};
use crate::dbus;
use adw::prelude::*;
use gtk::{gdk, glib};
use podtrack_core::profile::{Axis, map_axis};
use serde_json::Value;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

const APP_ID: &str = "io.github.f3nr1l.PodTrack";

#[derive(Default)]
struct Shared {
    status: Value,
    cfg_path: PathBuf,
}

type Sh = Rc<RefCell<Shared>>;

fn status_now() -> Value {
    dbus::call::<String>("Status", &()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn toast(overlay: &adw::ToastOverlay, text: &str) {
    overlay.add_toast(adw::Toast::builder().title(glib::markup_escape_text(text)).timeout(4).build());
}

/// Modifie la configuration et prévient le démon.
fn edit_cfg(sh: &Sh, change: impl FnOnce(&mut Config) -> anyhow::Result<()>) -> anyhow::Result<()> {
    let p = sh.borrow().cfg_path.clone();
    config::update(&p, change)?;
    let _ = dbus::call::<()>("Reload", &());
    Ok(())
}

thread_local! {
    static SHOT_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// `shot` : enregistre chaque page en PNG dans ce dossier puis quitte (vérification visuelle).
pub fn run(shot: Option<PathBuf>) -> anyhow::Result<()> {
    SHOT_DIR.with(|d| *d.borrow_mut() = shot);
    let app = adw::Application::builder().application_id(APP_ID).flags(gtk::gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_activate(build);
    app.run_with_args::<&str>(&[]);
    Ok(())
}

fn build(app: &adw::Application) {
    if let Some(w) = app.active_window() {
        w.present();
        return;
    }
    if let Some(theme) = crate::tray::install_icons() {
        if let Some(d) = gdk::Display::default() {
            gtk::IconTheme::for_display(&d).add_search_path(theme);
        }
    }
    let sh: Sh = Rc::new(RefCell::new(Shared { status: Value::Null, cfg_path: config::path() }));
    let overlay = adw::ToastOverlay::new();
    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(&status_page(&sh, &overlay), Some("etat"), "État", "view-reveal-symbolic");
    stack.add_titled_with_icon(&games_page(&sh, &overlay), Some("jeux"), "Jeux", "input-gaming-symbolic");
    stack.add_titled_with_icon(&calib_page(&sh, &overlay), Some("calibration"), "Calibration", "find-location-symbolic");
    stack.add_titled_with_icon(&outputs_page(&sh, &overlay), Some("sorties"), "Sorties", "network-transmit-symbolic");
    stack.add_titled_with_icon(&hotkeys_page(&sh, &overlay), Some("raccourcis"), "Raccourcis", "preferences-desktop-keyboard-shortcuts-symbolic");
    stack.add_titled_with_icon(&advanced_page(&sh, &overlay), Some("avance"), "Avancé", "preferences-other-symbolic");
    overlay.set_child(Some(&stack));

    let header = adw::HeaderBar::new();
    let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();
    header.set_title_widget(Some(&switcher));
    let bar = adw::ViewSwitcherBar::builder().stack(&stack).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_bottom_bar(&bar);
    view.set_content(Some(&overlay));

    let win = adw::ApplicationWindow::builder().application(app).title("podtrack").icon_name("podtrack").default_width(900).default_height(720).content(&view).build();
    // barre du bas seulement quand la fenêtre est étroite
    let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(adw::BreakpointConditionLengthType::MaxWidth, 620.0, adw::LengthUnit::Sp));
    bp.add_setter(&bar, "reveal", Some(&true.to_value()));
    bp.add_setter(&header, "title-widget", Some(&None::<gtk::Widget>.to_value()));
    win.add_breakpoint(bp);

    // état du démon, 20 fois par seconde
    {
        let sh = sh.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            sh.borrow_mut().status = status_now();
            glib::ControlFlow::Continue
        });
    }
    win.present();
    if let Some(dir) = SHOT_DIR.with(|d| d.borrow().clone()) {
        let _ = std::fs::create_dir_all(&dir);
        let pages = ["etat", "jeux", "calibration", "sorties", "raccourcis", "avance"];
        let i = Rc::new(std::cell::Cell::new(0usize));
        let (win2, stack2) = (win.clone(), stack.clone());
        glib::timeout_add_local(Duration::from_millis(1200), move || {
            let k = i.get();
            if k > 0 {
                // la page précédente est affichée : on la capture
                let paintable = gtk::WidgetPaintable::new(Some(&win2));
                let (w, h) = (win2.width() as f64, win2.height() as f64);
                let snap = gtk::Snapshot::new();
                paintable.snapshot(&snap, w, h);
                if let (Some(node), Some(native)) = (snap.to_node(), win2.native()) {
                    if let Some(r) = native.renderer() {
                        let tex = r.render_texture(&node, None);
                        let _ = tex.save_to_png(dir.join(format!("{}-{}.png", k, pages[k - 1])));
                    }
                }
            }
            if k == pages.len() {
                win2.close();
                return glib::ControlFlow::Break;
            }
            stack2.set_visible_child_name(pages[k]);
            i.set(k + 1);
            glib::ControlFlow::Continue
        });
    }
}

// ----------------------------------------------------------------------------------- État

fn draw_head(cr: &gtk::cairo::Context, w: f64, h: f64, head: [f64; 3], cam: Option<[f64; 2]>, live: bool) {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = (w.min(h) / 2.0 - 12.0).max(10.0);
    let fg = if live { (0.85, 0.85, 0.88) } else { (0.5, 0.5, 0.52) };
    // repères : cercles à 30° et 60° d'amplitude, axes
    cr.set_line_width(1.0);
    cr.set_source_rgba(fg.0, fg.1, fg.2, 0.25);
    for k in [1.0, 2.0, 3.0] {
        cr.arc(cx, cy, r * k / 3.0, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
    }
    cr.move_to(cx - r, cy);
    cr.line_to(cx + r, cy);
    cr.move_to(cx, cy - r);
    cr.line_to(cx, cy + r);
    let _ = cr.stroke();
    // direction du regard (lacet → x, tangage → y), échelle ±90°
    let px = cx + (head[0] / 90.0).clamp(-1.0, 1.0) * r;
    let py = cy - (head[1] / 90.0).clamp(-1.0, 1.0) * r;
    // la tête : ellipse inclinée du roulis
    cr.save().ok();
    cr.translate(px, py);
    cr.rotate(head[2].to_radians());
    cr.scale(0.78, 1.0);
    cr.arc(0.0, 0.0, r * 0.22, 0.0, std::f64::consts::TAU);
    cr.restore().ok();
    cr.set_source_rgba(0.21, 0.52, 0.89, if live { 0.9 } else { 0.4 });
    let _ = cr.fill();
    // caméra du jeu, échelle ±180° en lacet, ±90° en tangage
    if let Some([cxw, cyw]) = cam {
        let qx = cx + (cxw / 180.0).clamp(-1.0, 1.0) * r;
        let qy = cy - (cyw / 90.0).clamp(-1.0, 1.0) * r;
        cr.set_source_rgba(0.95, 0.62, 0.1, 0.95);
        cr.set_line_width(2.0);
        cr.arc(qx, qy, 9.0, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        cr.move_to(qx - 14.0, qy);
        cr.line_to(qx + 14.0, qy);
        cr.move_to(qx, qy - 14.0);
        cr.line_to(qx, qy + 14.0);
        let _ = cr.stroke();
    }
}

fn status_page(sh: &Sh, overlay: &adw::ToastOverlay) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let area = gtk::DrawingArea::builder().content_height(300).hexpand(true).build();
    let head_state: Rc<RefCell<([f64; 3], Option<[f64; 2]>, bool)>> = Rc::new(RefCell::new(([0.0; 3], None, false)));
    {
        let hs = head_state.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let (head, cam, live) = *hs.borrow();
            draw_head(cr, w as f64, h as f64, head, cam, live);
        });
    }
    let g0 = adw::PreferencesGroup::builder().title("Tête").description("Point bleu : direction du regard. Cible orange : caméra du jeu actif.").build();
    g0.add(&area);
    let angles = gtk::Label::builder().css_classes(["monospace", "title-4"]).margin_top(6).build();
    g0.add(&angles);
    let buttons = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).margin_top(12).build();
    let recenter = gtk::Button::builder().label("Recentrer").css_classes(["pill", "suggested-action"]).build();
    let toggle = gtk::Button::builder().label("Arrêter").css_classes(["pill"]).build();
    buttons.append(&recenter);
    buttons.append(&toggle);
    g0.add(&buttons);
    page.add(&g0);
    {
        let o = overlay.clone();
        recenter.connect_clicked(move |_| {
            if let Err(e) = dbus::call::<()>("Recenter", &()) {
                toast(&o, &format!("{e:#}"));
            }
        });
    }
    {
        let o = overlay.clone();
        toggle.connect_clicked(move |_| match dbus::call::<bool>("Toggle", &()) {
            Ok(on) => toast(&o, if on { "Suivi démarré" } else { "Suivi arrêté" }),
            Err(e) => toast(&o, &format!("{e:#}")),
        });
    }

    let g1 = adw::PreferencesGroup::builder().title("AirPods").build();
    let row = |t: &str| {
        let r = adw::ActionRow::builder().title(t).subtitle_selectable(true).use_markup(false).build();
        r.add_css_class("property");
        g1.add(&r);
        r
    };
    let conn = row("Connexion");
    let dev = row("Appareil");
    let bat = row("Batterie");
    let prof = row("Profil d'axes");
    let rate = row("Échantillons");
    let err = row("Dernière erreur");
    page.add(&g1);

    let sh = sh.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let v = sh.borrow().status.clone();
        if v.is_null() {
            conn.set_subtitle("démon arrêté — lance « podtrack daemon » ou le service podtrack");
            *head_state.borrow_mut() = ([0.0; 3], None, false);
            area.queue_draw();
            return glib::ControlFlow::Continue;
        }
        let active = v["active"].as_bool() == Some(true);
        toggle.set_label(if active { "Arrêter" } else { "Démarrer" });
        let st = v["connection"]["state"].as_str().unwrap_or("");
        let detail = v["connection"]["detail"].as_str().unwrap_or("");
        let stale = v["stale"].as_bool() == Some(true);
        conn.set_subtitle(&match st {
            _ if !active => "suivi arrêté".to_string(),
            "receiving" if !stale => "reçoit les capteurs".into(),
            "receiving" | "no_data" => "connecté, pas de données (écouteurs dans le boîtier ?)".into(),
            "connected" => "connecté".into(),
            "searching" => format!("recherche : {detail}"),
            _ => "en attente".into(),
        });
        dev.set_subtitle(&v["device"].as_object().map(|d| format!("{} — {}", d["name"].as_str().unwrap_or(""), d["model"].as_str().unwrap_or(""))).unwrap_or("—".into()));
        let b: Vec<String> = v["battery"].as_array().map(|a| {
            a.iter().filter(|p| p["state"].as_u64() != Some(4)).map(|p| {
                let n = match p["part"].as_u64() { Some(1) => "seul", Some(2) => "droit", Some(4) => "gauche", Some(8) => "boîtier", _ => "?" };
                format!("{n} {}%{}", p["level"], if p["state"].as_u64() == Some(1) { " (en charge)" } else { "" })
            }).collect()
        }).unwrap_or_default();
        bat.set_subtitle(&if b.is_empty() { "—".into() } else { b.join(" · ") });
        let calibrated = v["profile_calibrated"].as_bool() == Some(true);
        prof.set_subtitle(&format!("{}{}", v["profile"].as_str().unwrap_or("—"), if calibrated || v["profile"].is_null() { "" } else { " — non calibré : page Calibration" }));
        rate.set_subtitle(&format!("{} Hz · source {} · biais {}", v["rate_hz"], v["source"].as_str().unwrap_or("—"), if v["bias_ok"].as_bool() == Some(true) { "estimé" } else { "en cours" }));
        err.set_subtitle(v["last_error"].as_str().unwrap_or("—"));
        let h = &v["head"];
        let head = [f(&h[0]), f(&h[1]), f(&h[2])];
        let cam = v["games"].as_object().and_then(|g| g.values().next()).map(|g| [f(&g["pose"][3]), f(&g["pose"][4])]);
        angles.set_label(&format!("lacet {:+6.1}°   tangage {:+6.1}°   roulis {:+6.1}°", head[0], head[1], head[2]));
        *head_state.borrow_mut() = (head, cam, active && !stale);
        area.queue_draw();
        glib::ControlFlow::Continue
    });
    page.upcast()
}

// ----------------------------------------------------------------------------------- Jeux

fn spin(title: &str, sub: &str, lo: f64, hi: f64, step: f64, digits: u32, value: f64) -> adw::SpinRow {
    let r = adw::SpinRow::with_range(lo, hi, step);
    r.set_title(title);
    r.set_subtitle(sub);
    r.set_digits(digits);
    r.set_value(value);
    r
}

fn draw_curve(cr: &gtk::cairo::Context, w: f64, h: f64, axis: &Axis, now: Option<f64>) {
    let pad = 8.0;
    let xmax = (axis.edge_pos.max(axis.edge_neg) * 1.25).max(5.0);
    let ymax = axis.target.max(1.0);
    let x = |deg: f64| pad + deg / xmax * (w - 2.0 * pad);
    let y = |out: f64| h - pad - out / ymax * (h - 2.0 * pad);
    cr.set_source_rgba(0.5, 0.5, 0.5, 0.3);
    cr.set_line_width(1.0);
    cr.rectangle(pad, pad, w - 2.0 * pad, h - 2.0 * pad);
    let _ = cr.stroke();
    let mut a = axis.clone();
    a.enabled = true;
    a.invert = false;
    cr.set_source_rgba(0.21, 0.52, 0.89, 1.0);
    cr.set_line_width(2.0);
    let n = 120;
    for i in 0..=n {
        let d = xmax * i as f64 / n as f64;
        let o = map_axis(d.max(1e-9), &a).abs();
        if i == 0 { cr.move_to(x(d), y(o)) } else { cr.line_to(x(d), y(o)) }
    }
    let _ = cr.stroke();
    if let Some(d) = now {
        let d = d.abs().min(xmax);
        let o = map_axis(d.max(1e-9), &a).abs();
        cr.set_source_rgba(0.95, 0.62, 0.1, 1.0);
        cr.arc(x(d), y(o), 5.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
}

fn axis_editor(sh: &Sh, overlay: &adw::ToastOverlay, game: &str, name: &'static str, label: &str, axis: &Axis, idx: usize) -> adw::ExpanderRow {
    let exp = adw::ExpanderRow::builder().title(label).subtitle(format!("{:.0}° de tête → {:.0}° en jeu, courbe {:.2}", axis.edge_pos, axis.target, axis.curve)).build();
    let cur = Rc::new(RefCell::new(axis.clone()));
    let area = gtk::DrawingArea::builder().content_height(140).margin_top(6).margin_bottom(6).margin_start(12).margin_end(12).build();
    {
        let cur = cur.clone();
        let sh2 = sh.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let v = &sh2.borrow().status;
            let now = v["head"].get(idx).and_then(Value::as_f64);
            draw_curve(cr, w as f64, h as f64, &cur.borrow(), now);
        });
    }
    {
        let area = area.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            area.queue_draw();
            glib::ControlFlow::Continue
        });
    }
    let row = gtk::ListBoxRow::builder().child(&area).activatable(false).build();
    exp.add_row(&row);

    let enabled = adw::SwitchRow::builder().title("Axe actif").active(axis.enabled).build();
    let invert = adw::SwitchRow::builder().title("Inverser").active(axis.invert).build();
    let edge_neg = spin("Bord négatif (°)", "angle de tête pour atteindre le maximum (gauche / bas)", 2.0, 180.0, 1.0, 1, axis.edge_neg);
    let edge_pos = spin("Bord positif (°)", "angle de tête pour atteindre le maximum (droite / haut)", 2.0, 180.0, 1.0, 1, axis.edge_pos);
    let target = spin("Amplitude en jeu (°)", "angle de la caméra en bout de course", 0.0, 180.0, 5.0, 0, axis.target);
    let curve = spin("Courbe", "1 = linéaire ; plus grand = centre plus calme", 1.0, 3.0, 0.05, 2, axis.curve);
    let dz = spin("Zone morte (°)", "en dessous, la caméra ne bouge pas", 0.0, 45.0, 0.5, 1, axis.deadzone);
    for w in [enabled.upcast_ref::<gtk::Widget>(), invert.upcast_ref(), edge_neg.upcast_ref(), edge_pos.upcast_ref(), target.upcast_ref(), curve.upcast_ref(), dz.upcast_ref()] {
        exp.add_row(w);
    }
    let save = {
        let sh = sh.clone();
        let o = overlay.clone();
        let game = game.to_string();
        let cur = cur.clone();
        let exp = exp.clone();
        let (enabled, invert, edge_neg, edge_pos, target, curve, dz) = (enabled.clone(), invert.clone(), edge_neg.clone(), edge_pos.clone(), target.clone(), curve.clone(), dz.clone());
        Rc::new(move || {
            {
                let mut a = cur.borrow_mut();
                a.enabled = enabled.is_active();
                a.invert = invert.is_active();
                a.edge_neg = edge_neg.value();
                a.edge_pos = edge_pos.value();
                a.target = target.value();
                a.curve = curve.value();
                a.deadzone = dz.value();
                exp.set_subtitle(&format!("{:.0}° de tête → {:.0}° en jeu, courbe {:.2}", a.edge_pos, a.target, a.curve));
            }
            let a = cur.borrow().clone();
            let game = game.clone();
            if let Err(e) = edit_cfg(&sh, move |c| {
                let p = if game == "opentrack" { &mut c.opentrack.profile } else { &mut c.games.get_mut(&game).ok_or_else(|| anyhow::anyhow!("jeu disparu"))?.profile };
                p.own = true;
                match name {
                    "yaw" => p.axes.yaw = a,
                    "pitch" => p.axes.pitch = a,
                    _ => p.axes.roll = a,
                }
                Ok(())
            }) {
                toast(&o, &format!("{e:#}"));
            }
        })
    };
    for s in [&edge_neg, &edge_pos, &target, &curve, &dz] {
        let save = save.clone();
        s.connect_value_notify(move |_| save());
    }
    for s in [&enabled, &invert] {
        let save = save.clone();
        s.connect_active_notify(move |_| save());
    }
    exp
}

fn profile_group(sh: &Sh, overlay: &adw::ToastOverlay, id: &str, title: &str, p: &podtrack_core::profile::GameProfile) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder().title(title).build();
    for (i, (name, label)) in [("yaw", "Gauche / droite (lacet)"), ("pitch", "Haut / bas (tangage)"), ("roll", "Inclinaison (roulis)")].into_iter().enumerate() {
        g.add(&axis_editor(sh, overlay, id, name, label, p.axes.get(i), i));
    }
    let smoothing = spin("Lissage", "filtre « One Euro » (Casiez 2012), lisse à l'arrêt, réactif en mouvement : 0 = aucun, 0,3 ≈ 2,5 Hz, 1 = très lisse", 0.0, 1.0, 0.05, 2, p.smoothing);
    let lowpass = spin("Passe-bas (Hz)", "filtre supplémentaire ; 0 = aucun", 0.0, 100.0, 0.5, 1, p.lowpass_hz);
    g.add(&smoothing);
    g.add(&lowpass);
    for (row, key) in [(smoothing, "smoothing"), (lowpass, "lowpass")] {
        let sh = sh.clone();
        let o = overlay.clone();
        let id = id.to_string();
        row.connect_value_notify(move |r| {
            let v = r.value();
            let id = id.clone();
            if let Err(e) = edit_cfg(&sh, move |c| {
                let p = if id == "opentrack" { &mut c.opentrack.profile } else { &mut c.games.get_mut(&id).ok_or_else(|| anyhow::anyhow!("jeu disparu"))?.profile };
                if key == "smoothing" { p.smoothing = v } else { p.lowpass_hz = v }
                Ok(())
            }) {
                toast(&o, &format!("{e:#}"));
            }
        });
    }
    g
}

fn games_page(sh: &Sh, overlay: &adw::ToastOverlay) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let cfg = config::load(&sh.borrow().cfg_path).unwrap_or_default();
    if cfg.games.is_empty() {
        let g = adw::PreferencesGroup::builder()
            .title("Aucun jeu")
            .description("Ajoute un jeu : « podtrack game add-wine <id> <nom> <préfixe> » ou « podtrack game add-steam <id> <appid> ».")
            .build();
        page.add(&g);
    }
    for (id, game) in &cfg.games {
        let where_ = match &game.kind {
            GameKind::Wine { prefix } => format!("Wine : {}", prefix.display()),
            GameKind::Steam { appid, flatpak } => format!("Steam {appid}{}", if *flatpak { " (Flatpak)" } else { "" }),
        };
        let g = adw::PreferencesGroup::builder().title(&game.name).description(&where_).build();
        let sw = adw::SwitchRow::builder()
            .title("Suivi par les AirPods")
            .subtitle("désactivé : les clés du traqueur d'origine (Beam…) sont rendues ; jeu fermé")
            .active(game.enabled && game.tracker == TrackerChoice::Podtrack)
            .build();
        {
            let o = overlay.clone();
            let id = id.clone();
            let busy = Rc::new(std::cell::Cell::new(false));
            sw.connect_active_notify(move |s| {
                if busy.get() {
                    return;
                }
                let want = if s.is_active() { "podtrack" } else { "other" };
                match dbus::call::<String>("SetTracker", &(id.as_str(), want)) {
                    Ok(m) => toast(&o, &m),
                    Err(e) => {
                        toast(&o, &format!("{e:#}"));
                        busy.set(true);
                        s.set_active(!s.is_active());
                        busy.set(false);
                    }
                }
            });
        }
        g.add(&sw);
        page.add(&g);
        page.add(&profile_group(sh, overlay, id, &format!("{} — réponse", game.name), &game.profile));
    }
    page.upcast()
}

// ----------------------------------------------------------------------------------- Calibration

fn step_text(step: &str) -> &'static str {
    match step {
        "still" => "Regarde l'écran et ne bouge plus",
        "yaw" => "Tourne la tête vers la GAUCHE",
        "back" => "Reviens en face de l'écran et ne bouge plus",
        "pitch" => "Lève la tête vers le HAUT",
        "roll" => "Penche la tête vers l'épaule DROITE",
        _ => "",
    }
}

/// Fenêtres plein écran de la calibration par points, une par moniteur choisi.
struct PointWindows {
    wins: Vec<(gtk::Window, gtk::DrawingArea)>,
    state: Rc<RefCell<(String, f64, String)>>,
}

fn monitors() -> Vec<gdk::Monitor> {
    let Some(d) = gdk::Display::default() else { return vec![] };
    let list = d.monitors();
    let mut out: Vec<gdk::Monitor> = (0..list.n_items()).filter_map(|i| list.item(i).and_downcast::<gdk::Monitor>()).collect();
    out.sort_by_key(|m| m.geometry().x());
    out
}

fn open_point_windows(chosen: &[gdk::Monitor], main: usize, on_cancel: Rc<dyn Fn()>) -> PointWindows {
    let state = Rc::new(RefCell::new((String::from("center"), 0.0, String::new())));
    let n = chosen.len();
    let mut wins = Vec::new();
    for (i, m) in chosen.iter().enumerate() {
        let area = gtk::DrawingArea::new();
        let st = state.clone();
        let is_main = i == main;
        let (first, last) = (i == 0, i + 1 == n);
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            cr.set_source_rgb(0.07, 0.07, 0.08);
            let _ = cr.paint();
            let (point, progress, hint) = st.borrow().clone();
            let pos = match point.as_str() {
                "center" if is_main => Some((w / 2.0, h / 2.0)),
                "top" if is_main => Some((w / 2.0, h * 0.06)),
                "bottom" if is_main => Some((w / 2.0, h * 0.94)),
                "left" if first => Some((w * 0.04, h / 2.0)),
                "right" if last => Some((w * 0.96, h / 2.0)),
                _ => None,
            };
            if let Some((x, y)) = pos {
                cr.set_source_rgba(0.21, 0.52, 0.89, 1.0);
                cr.arc(x, y, 14.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                cr.set_source_rgba(0.95, 0.62, 0.1, 1.0);
                cr.set_line_width(5.0);
                cr.arc(x, y, 30.0, -std::f64::consts::FRAC_PI_2, -std::f64::consts::FRAC_PI_2 + progress * std::f64::consts::TAU);
                let _ = cr.stroke();
            }
            if is_main {
                let text = match point.as_str() {
                    "center" => "Regarde le point au centre et tiens la position",
                    "left" => "Regarde le point à GAUCHE (là où la caméra doit aller au maximum)",
                    "right" => "Regarde le point à DROITE",
                    "top" => "Regarde le point en HAUT",
                    "bottom" => "Regarde le point en BAS",
                    _ => "",
                };
                cr.set_source_rgba(0.9, 0.9, 0.92, 1.0);
                cr.select_font_face("Sans", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
                cr.set_font_size(26.0);
                let line = if hint.is_empty() { text.to_string() } else { format!("{text} — {hint}") };
                if let Ok(ext) = cr.text_extents(&line) {
                    cr.move_to(w / 2.0 - ext.width() / 2.0, h * 0.62);
                    let _ = cr.show_text(&line);
                }
                cr.set_font_size(16.0);
                let sub = "Échap pour annuler";
                if let Ok(ext) = cr.text_extents(sub) {
                    cr.move_to(w / 2.0 - ext.width() / 2.0, h * 0.62 + 34.0);
                    let _ = cr.show_text(sub);
                }
            }
        });
        let win = gtk::Window::builder().decorated(false).child(&area).build();
        let keys = gtk::EventControllerKey::new();
        {
            let cancel = on_cancel.clone();
            keys.connect_key_pressed(move |_, k, _, _| {
                if k == gdk::Key::Escape {
                    cancel();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
        }
        win.add_controller(keys);
        win.fullscreen_on_monitor(m);
        win.present();
        wins.push((win, area));
    }
    PointWindows { wins, state }
}

fn calib_page(sh: &Sh, overlay: &adw::ToastOverlay) -> gtk::Widget {
    let page = adw::PreferencesPage::new();

    // appareil
    let g = adw::PreferencesGroup::builder()
        .title("Axes de l'écouteur")
        .description("À faire une fois par position de port (oreilles, boîtier, main). Sept étapes : immobile, gauche, retour, haut, retour, épaule droite, retour.")
        .build();
    let label = gtk::Label::builder().label("Prêt").css_classes(["title-2"]).wrap(true).justify(gtk::Justification::Center).margin_top(12).margin_bottom(12).build();
    let bar = gtk::ProgressBar::builder().margin_start(12).margin_end(12).build();
    let start = gtk::Button::builder().label("Calibrer les axes").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).margin_top(12).build();
    g.add(&label);
    g.add(&bar);
    g.add(&start);
    page.add(&g);
    let running = Rc::new(std::cell::Cell::new(false));
    {
        let o = overlay.clone();
        let running = running.clone();
        start.connect_clicked(move |b| {
            if running.get() {
                let _ = dbus::call::<()>("DeviceCalibrationCancel", &());
                running.set(false);
                b.set_label("Calibrer les axes");
                return;
            }
            match dbus::call::<()>("DeviceCalibrationStart", &()) {
                Ok(()) => {
                    running.set(true);
                    b.set_label("Annuler");
                }
                Err(e) => toast(&o, &format!("{e:#}")),
            }
        });
    }
    {
        let o = overlay.clone();
        let running = running.clone();
        let start = start.clone();
        glib::timeout_add_local(Duration::from_millis(80), move || {
            if running.get() {
                let st: Value = dbus::call::<String>("DeviceCalibrationState", &()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
                if st.is_null() {
                    running.set(false);
                    start.set_label("Calibrer les axes");
                    label.set_label("Calibration terminée et enregistrée");
                    bar.set_fraction(1.0);
                    toast(&o, "Profil d'axes enregistré");
                } else {
                    let i = st["index"].as_u64().unwrap_or(0);
                    label.set_label(&format!("Étape {}/7 — {}", i + 1, step_text(st["step"].as_str().unwrap_or(""))));
                    bar.set_fraction(f(&st["progress"]));
                }
            }
            glib::ControlFlow::Continue
        });
    }

    // amplitude par points
    let g2 = adw::PreferencesGroup::builder()
        .title("Amplitude par points à l'écran")
        .description("Un point s'affiche à la fois : centre, gauche, droite, haut, bas. Regarde-le et tiens la position deux secondes. L'angle de ta tête à chaque bord devient l'amplitude : à ce bord, la caméra atteint son maximum.")
        .build();
    let cfg = config::load(&sh.borrow().cfg_path).unwrap_or_default();
    let mut targets: Vec<(String, String)> = vec![("baseline".into(), "Référence (jeux sans réglage propre)".into())];
    targets.extend(cfg.games.iter().map(|(id, g)| (id.clone(), g.name.clone())));
    let model = gtk::StringList::new(&targets.iter().map(|t| t.1.as_str()).collect::<Vec<_>>());
    let target_row = adw::ComboRow::builder().title("Pour").model(&model).build();
    g2.add(&target_row);
    let mons = monitors();
    let mon_rows: Vec<adw::SwitchRow> = mons
        .iter()
        .map(|m| {
            let g = m.geometry();
            let r = adw::SwitchRow::builder()
                .title(format!("Moniteur {}", m.connector().map(|c| c.to_string()).unwrap_or_default()))
                .subtitle(format!("{}×{} à x={}", g.width(), g.height(), g.x()))
                .active(mons.len() == 1)
                .build();
            g2.add(&r);
            r
        })
        .collect();
    let side = spin("Amplitude gauche / droite en jeu (°)", "", 10.0, 180.0, 5.0, 0, 180.0);
    let vert = spin("Amplitude haut / bas en jeu (°)", "", 10.0, 180.0, 5.0, 0, 90.0);
    g2.add(&side);
    g2.add(&vert);
    let go = gtk::Button::builder().label("Commencer").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).margin_top(12).build();
    g2.add(&go);
    page.add(&g2);

    let active: Rc<RefCell<Option<PointWindows>>> = Rc::new(RefCell::new(None));
    let close_all = {
        let active = active.clone();
        Rc::new(move || {
            if let Some(pw) = active.borrow_mut().take() {
                for (w, _) in pw.wins {
                    w.close();
                }
            }
        })
    };
    {
        let o = overlay.clone();
        let active = active.clone();
        let close_all = close_all.clone();
        go.connect_clicked(move |_| {
            let mut chosen: Vec<gdk::Monitor> = mons.iter().zip(&mon_rows).filter(|(_, r)| r.is_active()).map(|(m, _)| m.clone()).collect();
            if chosen.is_empty() {
                if let Some(m) = mons.first() {
                    chosen.push(m.clone());
                }
            }
            let target = targets.get(target_row.selected() as usize).map(|t| t.0.clone()).unwrap_or("baseline".into());
            if let Err(e) = dbus::call::<()>("GameCalibrationStart", &(target.as_str())) {
                toast(&o, &format!("{e:#}"));
                return;
            }
            let cancel: Rc<dyn Fn()> = {
                let close_all = close_all.clone();
                Rc::new(move || {
                    let _ = dbus::call::<()>("GameCalibrationCancel", &());
                    close_all();
                })
            };
            let main = chosen.len() / 2;
            *active.borrow_mut() = Some(open_point_windows(&chosen, main, cancel));
            let (o, active, close_all, side, vert) = (o.clone(), active.clone(), close_all.clone(), side.clone(), vert.clone());
            glib::timeout_add_local(Duration::from_millis(50), move || {
                if active.borrow().is_none() {
                    return glib::ControlFlow::Break;
                }
                let st: Value = dbus::call::<String>("GameCalibrationState", &()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
                let s = &st[1];
                if let Some(end) = s["end"].as_str() {
                    if end == "done" {
                        match dbus::call::<String>("GameCalibrationApply", &(side.value(), vert.value())) {
                            Ok(m) => toast(&o, &m),
                            Err(e) => toast(&o, &format!("{e:#}")),
                        }
                    } else {
                        let _ = dbus::call::<()>("GameCalibrationCancel", &());
                        toast(&o, &format!("Calibration {end} {}", s["hint"].as_str().unwrap_or("")));
                    }
                    close_all();
                    return glib::ControlFlow::Break;
                }
                if st.is_null() {
                    close_all();
                    return glib::ControlFlow::Break;
                }
                let hint = match s["hint"].as_str() {
                    Some("too_close") => "trop près du centre, va plus loin",
                    Some("wrong_side") => "mauvais côté",
                    _ => "",
                };
                if let Some(pw) = active.borrow().as_ref() {
                    *pw.state.borrow_mut() = (s["point"].as_str().unwrap_or("").to_string(), f(&s["progress"]), hint.to_string());
                    for (_, a) in &pw.wins {
                        a.queue_draw();
                    }
                }
                glib::ControlFlow::Continue
            });
        });
    }
    page.upcast()
}

// ----------------------------------------------------------------------------------- Sorties

fn outputs_page(sh: &Sh, overlay: &adw::ToastOverlay) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let cfg = config::load(&sh.borrow().cfg_path).unwrap_or_default();
    let g = adw::PreferencesGroup::builder()
        .title("opentrack")
        .description("Envoie la pose à opentrack (entrée « UDP over network ») pour les jeux qu'il gère : X-Plane, jeux natifs, autres protocoles.")
        .build();
    let on = adw::SwitchRow::builder().title("Envoyer à opentrack").active(cfg.opentrack.enabled).build();
    let host = adw::EntryRow::builder().title("Hôte").text(&cfg.opentrack.host).show_apply_button(true).build();
    let port = spin("Port", "", 1.0, 65535.0, 1.0, 0, cfg.opentrack.port as f64);
    g.add(&on);
    g.add(&host);
    g.add(&port);
    page.add(&g);
    {
        let (sh, o) = (sh.clone(), overlay.clone());
        on.connect_active_notify(move |s| {
            let v = s.is_active();
            if let Err(e) = edit_cfg(&sh, move |c| { c.opentrack.enabled = v; Ok(()) }) { toast(&o, &format!("{e:#}")) }
        });
    }
    {
        let (sh, o) = (sh.clone(), overlay.clone());
        host.connect_apply(move |e| {
            let v = e.text().to_string();
            if let Err(e) = edit_cfg(&sh, move |c| { c.opentrack.host = v; Ok(()) }) { toast(&o, &format!("{e:#}")) }
        });
    }
    {
        let (sh, o) = (sh.clone(), overlay.clone());
        port.connect_value_notify(move |r| {
            let v = r.value() as u16;
            if let Err(e) = edit_cfg(&sh, move |c| { c.opentrack.port = v; Ok(()) }) { toast(&o, &format!("{e:#}")) }
        });
    }
    page.add(&profile_group(sh, overlay, "opentrack", "opentrack — réponse", &cfg.opentrack.profile));

    // souris par la tête
    let gm = adw::PreferencesGroup::builder()
        .title("Souris par la tête")
        .description("Mode souris : la vitesse de la tête déplace le curseur. Mode pointeur : la direction visée donne la position. En pause automatiquement pendant un jeu suivi (réglable dans Avancé).")
        .build();
    let mon = adw::SwitchRow::builder().title("Activer").active(cfg.mouse.enabled).build();
    let modes = gtk::StringList::new(&["Souris (relatif)", "Pointeur (absolu)"]);
    let mode = adw::ComboRow::builder().title("Mode").model(&modes).selected(if cfg.mouse.mode == podtrack_core::mouse::Mode::Pointeur { 1 } else { 0 }).build();
    let sx = spin("Sensibilité horizontale", "", 0.1, 10.0, 0.1, 2, cfg.mouse.sensitivity_x);
    let sy = spin("Sensibilité verticale", "", 0.1, 10.0, 0.1, 2, cfg.mouse.sensitivity_y);
    let stem = adw::SwitchRow::builder().title("Clics par la tige").subtitle("simple = clic gauche, double = droit, triple = défilement, long = pause (Avancé pour changer)").active(cfg.bluetooth.stem_events).build();
    for w in [mon.upcast_ref::<gtk::Widget>(), mode.upcast_ref(), sx.upcast_ref(), sy.upcast_ref(), stem.upcast_ref()] {
        gm.add(w);
    }
    page.add(&gm);
    {
        let (sh, o) = (sh.clone(), overlay.clone());
        mon.connect_active_notify(move |r| { let v = r.is_active(); if let Err(e) = edit_cfg(&sh, move |c| { c.mouse.enabled = v; Ok(()) }) { toast(&o, &format!("{e:#}")) } });
    }
    {
        let (sh, o) = (sh.clone(), overlay.clone());
        mode.connect_selected_notify(move |r| {
            let v = if r.selected() == 1 { podtrack_core::mouse::Mode::Pointeur } else { podtrack_core::mouse::Mode::Souris };
            if let Err(e) = edit_cfg(&sh, move |c| { c.mouse.mode = v; Ok(()) }) { toast(&o, &format!("{e:#}")) }
        });
    }
    for (r, x) in [(sx, true), (sy, false)] {
        let (sh, o) = (sh.clone(), overlay.clone());
        r.connect_value_notify(move |r| { let v = r.value(); if let Err(e) = edit_cfg(&sh, move |c| { if x { c.mouse.sensitivity_x = v } else { c.mouse.sensitivity_y = v } Ok(()) }) { toast(&o, &format!("{e:#}")) } });
    }
    {
        let (sh, o) = (sh.clone(), overlay.clone());
        stem.connect_active_notify(move |r| { let v = r.is_active(); if let Err(e) = edit_cfg(&sh, move |c| { c.bluetooth.stem_events = v; Ok(()) }) { toast(&o, &format!("{e:#}")) } else { toast(&o, "Pris en compte à la prochaine connexion des AirPods") } });
    }

    // manette virtuelle
    let gj = adw::PreferencesGroup::builder()
        .title("Manette virtuelle")
        .description("Pour les jeux Linux natifs qui lient la caméra à des axes de manette (Euro Truck Simulator 2, War Thunder…) : lacet, tangage et roulis sur RX, RY et RZ. Attention : comme toute manette ajoutée, elle peut décaler la numérotation des manettes dans un jeu (Star Citizen…).")
        .build();
    let jon = adw::SwitchRow::builder().title("Activer").active(cfg.joystick.enabled).build();
    gj.add(&jon);
    page.add(&gj);
    {
        let (sh, o) = (sh.clone(), overlay.clone());
        jon.connect_active_notify(move |r| { let v = r.is_active(); if let Err(e) = edit_cfg(&sh, move |c| { c.joystick.enabled = v; Ok(()) }) { toast(&o, &format!("{e:#}")) } });
    }
    page.upcast()
}

// ----------------------------------------------------------------------------------- Raccourcis

/// `<Super>XF86AudioMute` → libellé lisible (« Super+Muet » selon la langue).
fn accel_label(binding: &str) -> String {
    if binding.is_empty() {
        return "aucun".into();
    }
    match gtk::accelerator_parse(binding) {
        Some((k, m)) => gtk::accelerator_get_label(k, m).to_string(),
        None => binding.to_string(),
    }
}

fn hotkeys_page(sh: &Sh, overlay: &adw::ToastOverlay) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let cfg = config::load(&sh.borrow().cfg_path).unwrap_or_default();
    let g = adw::PreferencesGroup::builder()
        .title("Raccourcis globaux")
        .description("Enregistrés comme raccourcis personnalisés de GNOME : ils marchent même en jeu, mais la touche n'arrive plus au jeu. Choisis une combinaison que le jeu n'utilise pas.")
        .build();
    for (key, title, value) in [("recenter", "Recentrer", cfg.hotkeys.recenter.clone()), ("toggle", "Démarrer / arrêter", cfg.hotkeys.toggle.clone())] {
        let row = adw::ActionRow::builder().title(title).use_markup(false).build();
        row.set_subtitle(&accel_label(&value));
        let set = gtk::Button::builder().label("Changer…").valign(gtk::Align::Center).build();
        let clear = gtk::Button::builder().icon_name("edit-clear-symbolic").valign(gtk::Align::Center).tooltip_text("Aucun raccourci").css_classes(["flat"]).build();
        row.add_suffix(&set);
        row.add_suffix(&clear);
        g.add(&row);
        let apply = {
            let (sh, o, row) = (sh.clone(), overlay.clone(), row.clone());
            Rc::new(move |binding: String| {
                if !binding.is_empty() {
                    if let Some(other) = crate::hotkeys::conflict(&binding) {
                        toast(&o, &format!("{binding} est déjà pris par « {other} »"));
                        return;
                    }
                }
                let b2 = binding.clone();
                let res = edit_cfg(&sh, move |c| {
                    if key == "recenter" { c.hotkeys.recenter = b2 } else { c.hotkeys.toggle = b2 }
                    crate::hotkeys::apply(&c.hotkeys)
                });
                match res {
                    Ok(()) => {
                        row.set_subtitle(&accel_label(&binding));
                        toast(&o, "Raccourci enregistré");
                    }
                    Err(e) => toast(&o, &format!("{e:#}")),
                }
            })
        };
        {
            let apply = apply.clone();
            clear.connect_clicked(move |_| apply(String::new()));
        }
        set.connect_clicked(move |b| {
            let dlg = adw::AlertDialog::builder().heading("Nouveau raccourci").body("Appuie sur la combinaison voulue (Échap pour annuler).").build();
            dlg.add_response("cancel", "Annuler");
            let keys = gtk::EventControllerKey::new();
            let apply = apply.clone();
            let d2 = dlg.clone();
            keys.connect_key_pressed(move |_, k, _, state| {
                if k == gdk::Key::Escape {
                    d2.close();
                    return glib::Propagation::Stop;
                }
                let mods = state & gtk::accelerator_get_default_mod_mask();
                // une touche de modification seule ne suffit pas
                if matches!(k, gdk::Key::Shift_L | gdk::Key::Shift_R | gdk::Key::Control_L | gdk::Key::Control_R | gdk::Key::Alt_L | gdk::Key::Alt_R | gdk::Key::Super_L | gdk::Key::Super_R | gdk::Key::Meta_L | gdk::Key::Meta_R) {
                    return glib::Propagation::Stop;
                }
                let name = gtk::accelerator_name(k, mods).to_string();
                d2.close();
                apply(name);
                glib::Propagation::Stop
            });
            dlg.add_controller(keys);
            dlg.present(Some(b));
        });
    }
    page.add(&g);
    page.upcast()
}

// ----------------------------------------------------------------------------------- Avancé

/// Libellés français des réglages courants ; les autres gardent leur nom de clé.
fn label_for(path: &str) -> Option<&'static str> {
    Some(match path {
        "/bluetooth/address" => "Adresse des AirPods (auto = premiers connectés)",
        "/bluetooth/auto_connect" => "Connecter automatiquement",
        "/bluetooth/sensor_interval_us" => "Intervalle des capteurs (µs, 10000 = 100 Hz)",
        "/bluetooth/gyro_dps_per_lsb" => "Échelle du gyroscope (°/s par unité)",
        "/bluetooth/relaunch_s" => "Relance du flux sans données (s)",
        "/bluetooth/relaunch_case_s" => "Relance dans le boîtier (s)",
        "/bluetooth/reconnect_min_s" => "Reconnexion : premier délai (s)",
        "/bluetooth/reconnect_max_s" => "Reconnexion : délai maximal (s)",
        "/bluetooth/stem_events" => "Recevoir les appuis sur la tige",
        "/bluetooth/imtu" => "MTU de réception L2CAP",
        "/bluetooth/handshake_pauses_s" => "Pauses de la poignée de main (s)",
        "/bluetooth/activity_warmup_us" => "Flux d'activité avant le mouvement (µs)",
        "/bluetooth/activity_pause_s" => "Pause après le flux d'activité (s)",
        "/bluetooth/l2cap_timeout_s" => "Délai d'ouverture du canal (s)",
        "/bluetooth/bt_connect_timeout_s" => "Délai de connexion Bluetooth (s)",
        "/bluetooth/bt_probe_timeout_s" => "Délai d'essai de connexion (s)",
        "/bluetooth/close_pause_s" => "Pause à la fermeture (s)",
        "/bluetooth/psm" => "PSM L2CAP",
        "/bluetooth/security" => "Niveau de sécurité Bluetooth",
        "/engine/window" => "Biais : fenêtre (échantillons)",
        "/engine/min_samples" => "Biais : échantillons minimum",
        "/engine/still_gravity_deg" => "Immobilité : rotation max. de la gravité (°)",
        "/engine/first_bias_max_dps" => "Biais : écart max. du premier (°/s)",
        "/engine/bias_jump_max_dps" => "Biais : écart max. ensuite (°/s)",
        "/engine/lat_min" => "Axe latéral : norme minimale",
        "/engine/gravity_min" => "Gravité : norme minimale",
        "/tracker/scale" => "Échelle globale lacet / tangage / roulis",
        "/tracker/invert" => "Inversion globale lacet / tangage / roulis",
        "/tracker/smoothing_max_cutoff_hz" => "Lissage : coupure à 0 (Hz)",
        "/tracker/smoothing_min_cutoff_hz" => "Lissage : coupure à 1 (Hz)",
        "/tracker/filter_beta" => "Lissage : réactivité à la vitesse",
        "/tracker/filter_d_cutoff_hz" => "Lissage : coupure de la dérivée (Hz)",
        "/tracker/fusion_jump_min_deg" => "Réinitialisation de la fusion : saut min. (°)",
        "/tracker/fusion_gap_s" => "Réinitialisation de la fusion : écart max. (s)",
        "/tracker/lean_experimental" => "Translation estimée (expérimental)",
        "/output/shm_dir" => "Dossier de mémoire partagée",
        "/output/max_dt" => "Pas de temps max. des filtres (s)",
        "/bridge/dir_name" => "Dossier des DLL dans drive_c",
        "/bridge/games_csv" => "Liste des jeux opentrack (CSV)",
        "/bridge/dll_dir" => "Dossier des DLL à installer",
        "/steam/roots" => "Racine Steam",
        "/steam/flatpak_roots" => "Racine Steam (Flatpak)",
        "/engine/still_range_dps" => "Immobilité : étendue max. du gyroscope (°/s)",
        "/engine/still_time_s" => "Immobilité : durée avant estimation du biais (s)",
        "/engine/rate" => "Biais : vitesse de mise à jour",
        "/engine/gravity_alpha" => "Filtre de la gravité",
        "/tracker/source" => "Source de l'orientation (quaternion / gyro)",
        "/tracker/smoothing" => "Lissage global (0..1)",
        "/tracker/drift_correction" => "Correction de dérive (source gyro)",
        "/tracker/drift_tau_s" => "Constante de la correction de dérive (s)",
        "/tracker/yaw_autocenter_s" => "Recentrage automatique du lacet (s, 0 = non)",
        "/output/rate_hz" => "Cadence des sorties (Hz)",
        "/output/stale_after_s" => "Sans données depuis (s) : les jeux recentrent",
        "/bridge/dll_debug" => "Journal des DLL dans le préfixe",
        "/place/debounce_s" => "Changement oreille / boîtier pris en compte après (s)",
        "/game_calibration/hold_s" => "Calibration par points : maintien (s)",
        "/game_calibration/still_deg" => "Calibration par points : immobilité (°)",
        "/game_calibration/min_deg" => "Calibration par points : écart minimal (°)",
        "/device_calibration/step_deg" => "Calibration des axes : rotation par étape (°)",
        _ => return None,
    })
}

const ADVANCED: [(&str, &str); 11] = [
    ("mouse", "Souris par la tête"),
    ("joystick", "Manette virtuelle"),
    ("bluetooth", "Bluetooth"),
    ("engine", "Moteur de mouvement"),
    ("tracker", "Orientation de la tête"),
    ("output", "Sorties"),
    ("bridge", "Pont TrackIR (préfixes Wine)"),
    ("steam", "Steam"),
    ("place", "Position des écouteurs"),
    ("device_calibration", "Calibration des axes"),
    ("game_calibration", "Calibration par points"),
];

fn set_pointer(sh: &Sh, overlay: &adw::ToastOverlay, path: String, value: Value) {
    let res = edit_cfg(sh, move |c| {
        let mut v = serde_json::to_value(&*c)?;
        *v.pointer_mut(&path).ok_or_else(|| anyhow::anyhow!("réglage inconnu {path}"))? = value;
        *c = serde_json::from_value(v)?;
        Ok(())
    });
    if let Err(e) = res {
        toast(overlay, &format!("{e:#}"));
    }
}

fn leaf_rows(sh: &Sh, overlay: &adw::ToastOverlay, g: &adw::PreferencesGroup, path: String, cur: &Value, def: &Value) {
    let title = label_for(&path).map(str::to_string).unwrap_or_else(|| {
        let mut parts = path.rsplit('/');
        let last = parts.next().unwrap_or("");
        match last.parse::<usize>() {
            // élément de tableau : nom de la clé parente et rang
            Ok(i) => format!("{} [{}]", label_for(&path[..path.len() - last.len() - 1]).map(str::to_string).unwrap_or_else(|| parts.next().unwrap_or("").replace('_', " ")), i + 1),
            Err(_) => last.replace('_', " "),
        }
    });
    let reset = gtk::Button::builder().icon_name("edit-undo-symbolic").valign(gtk::Align::Center).css_classes(["flat"]).tooltip_text(format!("Valeur par défaut : {def}")).build();
    match cur {
        Value::Bool(b) => {
            let r = adw::SwitchRow::builder().title(&title).subtitle(&path).active(*b).build();
            r.add_suffix(&reset);
            let (sh2, o, p) = (sh.clone(), overlay.clone(), path.clone());
            r.connect_active_notify(move |s| set_pointer(&sh2, &o, p.clone(), Value::Bool(s.is_active())));
            let (r2, d) = (r.clone(), def.as_bool().unwrap_or(false));
            reset.connect_clicked(move |_| r2.set_active(d));
            g.add(&r);
        }
        Value::Number(n) => {
            let is_int = n.is_u64() || n.is_i64();
            let v = n.as_f64().unwrap_or(0.0);
            let hi = (v.abs() * 10.0).max(def.as_f64().unwrap_or(0.0).abs() * 10.0).max(if is_int { 100_000.0 } else { 100.0 });
            let lo = if v < 0.0 || def.as_f64().unwrap_or(0.0) < 0.0 { -hi } else { 0.0 };
            let step = if is_int { 1.0 } else { (v.abs() / 20.0).max(0.001) };
            let r = spin(&title, &path, lo, hi, step, if is_int { 0 } else { 4 }, v);
            r.add_suffix(&reset);
            let (sh2, o, p) = (sh.clone(), overlay.clone(), path.clone());
            r.connect_value_notify(move |s| {
                let val = if is_int { Value::from(s.value().round() as i64) } else { Value::from(s.value()) };
                set_pointer(&sh2, &o, p.clone(), val);
            });
            let (r2, d) = (r.clone(), def.as_f64().unwrap_or(0.0));
            reset.connect_clicked(move |_| r2.set_value(d));
            g.add(&r);
        }
        Value::String(s) => {
            let r = adw::EntryRow::builder().title(format!("{title} ({path})")).text(s).show_apply_button(true).build();
            r.add_suffix(&reset);
            let (sh2, o, p) = (sh.clone(), overlay.clone(), path.clone());
            r.connect_apply(move |e| set_pointer(&sh2, &o, p.clone(), Value::String(e.text().to_string())));
            let (r2, d) = (r.clone(), def.as_str().unwrap_or("").to_string());
            reset.connect_clicked(move |_| {
                r2.set_text(&d);
                r2.emit_by_name::<()>("apply", &[]);
            });
            g.add(&r);
        }
        Value::Null => {
            // chemin facultatif : vide = valeur fournie
            let r = adw::EntryRow::builder().title(format!("{title} ({path}, vide = défaut)")).show_apply_button(true).build();
            let (sh2, o, p) = (sh.clone(), overlay.clone(), path.clone());
            r.connect_apply(move |e| {
                let t = e.text().to_string();
                set_pointer(&sh2, &o, p.clone(), if t.is_empty() { Value::Null } else { Value::String(t) });
            });
            g.add(&r);
        }
        Value::Array(items) => {
            for (i, it) in items.iter().enumerate() {
                leaf_rows(sh, overlay, g, format!("{path}/{i}"), it, def.get(i).unwrap_or(&Value::Null));
            }
        }
        Value::Object(m) => {
            for (k, it) in m {
                leaf_rows(sh, overlay, g, format!("{path}/{k}"), it, def.get(k).unwrap_or(&Value::Null));
            }
        }
    }
}

fn advanced_page(sh: &Sh, overlay: &adw::ToastOverlay) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let path = sh.borrow().cfg_path.clone();
    let cur = serde_json::to_value(config::load(&path).unwrap_or_default()).unwrap_or(Value::Null);
    let def = serde_json::to_value(Config::default()).unwrap_or(Value::Null);
    let top = adw::PreferencesGroup::builder()
        .title("Fichier de configuration")
        .description(format!("{} — relu à chaud par le démon. Chaque ligne a un bouton de retour à sa valeur par défaut.", path.display()))
        .build();
    let open = gtk::Button::builder().label("Ouvrir le fichier").css_classes(["pill"]).halign(gtk::Align::Start).build();
    {
        let p = path.clone();
        open.connect_clicked(move |_| {
            let _ = std::process::Command::new("xdg-open").arg(&p).spawn();
        });
    }
    top.add(&open);
    page.add(&top);
    for (key, title) in ADVANCED {
        let g = adw::PreferencesGroup::builder().title(title).build();
        leaf_rows(sh, overlay, &g, format!("/{key}"), &cur[key], &def[key]);
        page.add(&g);
    }
    page.upcast()
}
