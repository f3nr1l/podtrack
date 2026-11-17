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

Depuis le dépôt COPR (Fedora 44 et suivantes, x86_64 et aarch64), avec les mises à jour par `dnf` :

```sh
sudo dnf copr enable f3nr1l/podtrack
sudo dnf install podtrack
systemctl --user enable --now podtrack.service
```

Ou en construisant le paquet :

```sh
packaging/rpm/build-rpm.sh          # ajouter --dll pour recompiler aussi les DLL Windows
sudo dnf install ./target/rpm/RPMS/x86_64/podtrack-*.rpm
systemctl --user enable --now podtrack.service
```

Le paquet installe `/usr/bin/podtrack`, le service utilisateur `podtrack.service`, l'entrée de
bureau et l'icône.

Le script fabrique d'abord un paquet source (`target/rpm/SRPMS/`, seul avec `--srpm`) qui
contient les crates de `Cargo.lock` : il se reconstruit sans réseau, dans mock ou sur COPR.
Prérequis : `rpm-build`, `cargo-vendor-filterer`, `gtk4-devel`, `libadwaita-devel`.

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

podtrack est sous licence Apache-2.0. Les mentions exigées par les licences des projets dont il
dérive sont dans `NOTICE`.

### MousePods et starman-tech

podtrack n'existerait pas sans [MousePods](https://github.com/starman-tech/mousepods), de
Matteo Gaudioso ([starman-tech](https://github.com/starman-tech)). podtrack en est la réécriture en
Rust, et lui doit l'essentiel de ce qui le fait marcher :

- **l'accès aux capteurs des AirPods** sous Linux, sans appareil Apple : le canal AACP sur L2CAP,
  la séquence qui ouvre le flux de mouvement, le format SensorDataWX des AirPods Pro, et la
  première documentation publique du protocole RemoteHID des AirPods (3e génération). Ce travail
  de rétro-ingénierie est publié dans la documentation de MousePods
  ([RemoteHID](https://github.com/starman-tech/mousepods/blob/main/docs/REMOTEHID-PROTOCOL.md),
  [capteurs des AirPods 3](https://github.com/starman-tech/mousepods/blob/main/docs/AIRPODS3-SENSORS.md),
  [découvertes](https://github.com/starman-tech/mousepods/blob/main/docs/DISCOVERIES.md)), sous
  licence CC BY 4.0 ;
- **le traitement du mouvement** : estimation de la gravité et du biais du gyroscope, suivi de tête
  par quaternion ou par gyroscope avec correction de dérive, filtres « One Euro », profils de
  réponse par jeu, calibration des axes de l'écouteur et calibration par points à l'écran.
  podtrack reprend ces algorithmes à l'identique : une capture réelle rejouée dans les deux
  programmes donne les mêmes valeurs ;
- **le pont pour les jeux Wine / Proton**, apporté à MousePods par
  [Henning Norden](https://github.com/henningnorden) : des DLL TrackIR et FreeTrack placées dans
  le préfixe du jeu, qui lisent la pose dans un fichier, sans programme dans Wine.

Sous Windows, sur un autre bureau que GNOME, ou pour ses autres fonctions (souris par la tête, démo
cockpit, parole silencieuse, montres Garmin), MousePods est le bon choix.

### Autres projets

- [opentrack](https://github.com/opentrack/opentrack) (Stanislaw Halik et contributeurs, ISC) : le
  code NPClient d'origine, la liste des jeux TrackIR et leurs clés, et le décodage des tables
  « Enhanced » ;
- [linux-track](https://github.com/uglyDwarf/linuxtrack) (uglyDwarf, MIT), dont vient le NPClient
  d'opentrack.

AirPods est une marque d'Apple Inc. Ce projet n'est pas affilié à Apple.
