# podtrack

Suivi de tête par AirPods sous Linux. Les capteurs de mouvement des AirPods servent de
traqueur de tête pour les jeux (protocoles TrackIR et FreeTrack sous Wine / Proton, opentrack,
manette virtuelle) et de dispositif de pointage par la tête.

podtrack est un portage en Rust de [MousePods](https://github.com/starman-tech/mousepods),
avec une interface GNOME, des courbes de réponse par jeu et la gestion des jeux hors Steam.

## Fonctions

- **Jeux sous Wine / Proton** : DLL TrackIR (`NPClient`) et FreeTrack (`freetrackclient`)
  installées dans le préfixe du jeu, sans programme supplémentaire dans Wine. Les jeux
  « TrackIR Enhanced », qui n'acceptent que des données chiffrées, sont gérés : la table de
  chaque jeu de la liste d'opentrack est écrite dans le préfixe.
- **Jeux Steam et hors Steam** : détection des jeux Proton installés (Steam natif ou Flatpak),
  ou déclaration d'un préfixe Wine quelconque (Lutris, Heroic, installation manuelle).
- **Cohabitation avec un autre traqueur** : le traqueur déjà installé dans un préfixe
  (opentrack, Beam Eye Tracker, TrackIR…) est sauvegardé ; un interrupteur rend ses clés
  d'origine à l'identique.
- **Réponse par jeu** : amplitude de chaque côté, courbe, zone morte, inversion, filtre de
  lissage « One Euro » et passe-bas, modèle du cou facultatif ; graphe en direct.
- **Calibration** : axes de l'écouteur (sept gestes guidés) et amplitude par points à regarder,
  affichés en calque au centre puis aux quatre coins de l'écran (multi-écran possible).
- **opentrack** : sortie « UDP over network » pour les jeux et protocoles qu'opentrack gère.
- **Manette virtuelle** : lacet, tangage et roulis sur des axes de manette, pour les jeux
  Linux natifs qui lient la caméra à une manette.
- **Pointage par la tête** : le curseur suit la tête, en mode relatif (vitesse, accélération
  et courbes réglables) ou absolu (direction visée) ; clics par la tige, par inclinaison ou
  par immobilité.
- **Interface GNOME** (GTK 4, libadwaita) : état et aperçu de la tête, jeux, calibrations,
  sorties, raccourcis globaux, réglages avancés ; icône dans la barre du haut.
- **Tout est réglable** : chaque paramètre est dans `~/.config/podtrack/config.toml`, relu à
  chaud, et modifiable dans l'onglet Avancé.

## Écouteurs pris en charge

| Modèle | Protocole | État |
|---|---|---|
| AirPods Pro 3 | SensorDataWX | vérifié, environ 100 Hz |
| AirPods Pro 2 (Lightning, USB-C) | SensorDataWX | vérifié par MousePods |
| AirPods (3e génération) | RemoteHID | pris en charge par le protocole, non vérifié avec podtrack |
| AirPods 4, AirPods Pro (1re gén.), AirPods Max | — | capteurs présents, non testés |
| AirPods (1re et 2e génération) | — | pas de capteurs de mouvement |

## Prérequis

- Linux avec BlueZ (bus système D-Bus), AirPods appairés.
- Accès à `/dev/uinput` pour le pointage et la manette (accordé à l'utilisateur de la session
  sur la plupart des distributions).
- Pour l'icône de barre sous GNOME : l'extension « AppIndicator and KStatusNotifierItem Support ».

## Installation

### Paquet RPM (Fedora)

```sh
packaging/rpm/build-rpm.sh          # ajouter --dll pour recompiler aussi les DLL Windows
sudo dnf install ./target/rpm/RPMS/x86_64/podtrack-*.rpm
systemctl --user enable --now podtrack.service
```

Le paquet installe `/usr/bin/podtrack`, le service utilisateur `podtrack.service`, l'entrée de
bureau et l'icône.

### Flatpak

En préparation.

### Depuis les sources

```sh
cargo build --release
install -m755 target/release/podtrack ~/.local/bin/
install -Dm644 packaging/podtrack.service ~/.config/systemd/user/podtrack.service
install -Dm644 packaging/io.github.f3nr1l.PodTrack.desktop ~/.local/share/applications/
install -Dm644 crates/podtrack/data/icons/podtrack.svg ~/.local/share/icons/hicolor/scalable/apps/io.github.f3nr1l.PodTrack.svg
systemctl --user daemon-reload && systemctl --user enable --now podtrack.service
```

Dépendances de compilation sous Fedora : `gtk4-devel`, `libadwaita-devel` ; à l'exécution, `librsvg2` (rendu de l'icône de barre).
Les DLL Windows sont fournies dans `crates/podtrack-out/data/dll` ; pour les recompiler :
`crates/podtrack-dll/build.sh` (cibles `x86_64` et `i686-pc-windows-gnu`, MinGW).

## Prise en main

1. Appairer les AirPods dans les réglages Bluetooth, puis les porter.
2. Lancer le service (voir plus haut) : l'icône apparaît dans la barre du haut.
3. Ouvrir la fenêtre (`podtrack gui` ou menu de l'icône), onglet **Calibration** :
   calibrer les axes de l'écouteur.
4. Onglet **Jeux** : ajouter un jeu Steam ou un préfixe Wine, puis activer « Suivi par les
   AirPods » (jeu fermé).
5. Onglet **Calibration** : régler l'amplitude du jeu en regardant les points affichés.
6. Onglet **Raccourcis** : choisir une touche de recentrage.

## Ligne de commande

```text
podtrack daemon              démon (Bluetooth, sorties, D-Bus, icône)
podtrack gui                 fenêtre
podtrack status [--json]     état
podtrack watch               angles en direct
podtrack recenter            recentrer
podtrack start | stop        démarrer / arrêter le suivi
podtrack tracker <jeu> podtrack|other
podtrack game list | steam | add-steam <id> <appid> | add-wine <id> <nom> <préfixe> | remove <id>
podtrack calibrate device | game <baseline|mouse|jeu>
podtrack import-mousepods <config.json> [--map ancien=nouveau]
podtrack config path | show | defaults
```

## Fonctionnement

```text
AirPods ──L2CAP (AACP)──► démon podtrack ──► fichier de pose (/dev/shm) ──► DLL dans le jeu (Wine)
                              │                                        └──► opentrack (UDP)
                              ├──► manette et pointage (uinput)
                              └──► D-Bus ◄── fenêtre, icône, ligne de commande, raccourcis
```

Les AirPods calculent eux-mêmes leur orientation (quaternion) ; podtrack l'exprime dans un
repère lié à la tête, appris à la calibration, puis applique le profil du jeu. La DLL lit la
pose dans un fichier en mémoire partagée vu par le lecteur `Z:` de Wine : rien ne tourne dans
le préfixe. Sans nouvelle pose depuis 1,5 s, la DLL renvoie le centre au lieu de figer la caméra.

Le comportement numérique est identique à MousePods : une capture réelle rejouée dans les deux
implémentations donne les mêmes valeurs (écart maximal de l'ordre de 1e-13).

## Dépannage

- `podtrack status` et `journalctl --user -u podtrack` donnent l'état et les erreurs.
- « non calibré » : faire la calibration des axes de l'écouteur.
- Pas de données (écouteurs dans le boîtier) : le flux est relancé automatiquement.
- Débit des capteurs divisé par deux (environ 55 Hz au lieu de 100) : le son passe par les
  AirPods et partage la liaison Bluetooth. Un autre périphérique audio rend le plein débit.
- La calibration par points affiche le résultat avant de l'appliquer ; « Revenir au
  précédent » rétablit les réglages remplacés.
- La caméra ne bouge pas dans un jeu : vérifier dans les options du jeu que le suivi de tête
  TrackIR est activé ; l'option `bridge.dll_debug` crée un journal `C:\podtrack\npclient.log`
  dans le préfixe.
- Le registre d'un préfixe n'est modifié que jeu fermé ; une copie datée de `user.reg` est
  faite avant chaque modification.

## Licence et remerciements

Apache-2.0. podtrack dérive de MousePods (© Matteo Gaudioso, starman-tech), dont les travaux
documentent l'accès aux capteurs des AirPods ; le pont TrackIR / FreeTrack dérive de la
contribution de Henning Norden à MousePods, elle-même issue d'opentrack (ISC) et de
linux-track (MIT). Détails dans `NOTICE`.

AirPods est une marque d'Apple Inc. Ce projet n'est pas affilié à Apple.
