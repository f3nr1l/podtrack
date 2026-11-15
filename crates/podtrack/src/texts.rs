//! Textes des consignes de calibration, partagés par la fenêtre et la ligne de commande.

/// Étape de la calibration des axes de l'écouteur.
pub fn step(step: &str) -> &'static str {
    match step {
        "still" => "Regarder l'écran sans bouger",
        "yaw" => "Tourner la tête vers la GAUCHE",
        "back" => "Revenir face à l'écran sans bouger",
        "pitch" => "Lever la tête vers le HAUT",
        "roll" => "Pencher la tête vers l'épaule DROITE",
        _ => "",
    }
}

/// Point à regarder pendant la calibration de l'amplitude.
pub fn point(p: &str) -> &'static str {
    match p {
        "center" => "Regarder le CENTRE de l'écran",
        "left" => "Regarder le bord GAUCHE de l'écran",
        "right" => "Regarder le bord DROIT de l'écran",
        "top" => "Regarder le HAUT de l'écran",
        "bottom" => "Regarder le BAS de l'écran",
        "top_left" => "Regarder le coin EN HAUT À GAUCHE",
        "top_right" => "Regarder le coin EN HAUT À DROITE",
        "bottom_left" => "Regarder le coin EN BAS À GAUCHE",
        "bottom_right" => "Regarder le coin EN BAS À DROITE",
        _ => "",
    }
}

/// Indication après une position refusée.
pub fn hint(h: &str) -> &'static str {
    match h {
        "too_close" => "trop près du centre, aller plus loin",
        "wrong_side" => "mauvais côté",
        "no_signal" => "plus de données des écouteurs",
        _ => "",
    }
}
