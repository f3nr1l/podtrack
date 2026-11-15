# Données de test

- `airpods3_motion_capture.json` : capture réelle d'AirPods 3 (immobile, lacet, tangage, roulis),
  reprise de MousePods (`captures/`), © Matteo Gaudioso (starman-tech), CC BY 4.0,
  https://github.com/starman-tech/mousepods — fichier inchangé.
- `reference_*.json` : sorties de MousePods (Python) sur ces captures, produites par
  `tools/make_reference.py`. Les tests Rust doivent les retrouver.
