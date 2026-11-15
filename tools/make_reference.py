#!/usr/bin/env python3
"""Rejoue une capture dans le code Python de MousePods et écrit les sorties de référence.

Reproduit Controller.on_block : pas de temps, MotionEngine.step, HeadTracker.update (horloge
remplacée par l'heure de la capture), puis ProfileFilter.apply du profil de jeu.

Usage : make_reference.py <dossier mousepods> <capture.json> <sortie.json> [profil.json]
"""
import json
import sys

src, capture, out_path = sys.argv[1:4]
sys.path.insert(0, src)
import airpods_mouse_controller as m  # noqa: E402
import game_profiles as gp            # noqa: E402
import opentrack_out as ot            # noqa: E402

clock = [0.0]
ot.time.monotonic = lambda: clock[0]          # horloge déterministe = heure de la capture

DEVICE = {"yaw_sign": 1, "lat": [-0.003, 0.1651, -0.9863], "pitch_sign": 1, "roll_sign": -1}
GAME = {"own": True, "smoothing": 0.3, "axes": {
    "yaw": {"edge_neg": 40, "edge_pos": 40, "target": 180, "curve": 1.75},
    "pitch": {"edge_neg": 20, "edge_pos": 20, "target": 90, "curve": 1.75},
    "roll": {"edge_neg": 90, "edge_pos": 90, "target": 90, "curve": 1.0}}}
if len(sys.argv) > 4:
    GAME = json.load(open(sys.argv[4]))

cfg = m.load_config("/nonexistent.json")
eng = m.MotionEngine(cfg["bias"], DEVICE)
trk = ot.HeadTracker(cfg["opentrack"])
flt = gp.ProfileFilter(GAME)
last_ts, last_sample = None, None
rows = []
for s in json.load(open(capture)):
    now = s["t"]
    clock[0] = now
    ts = s["ts"]
    w = tuple(c * cfg["gyro_dps_per_lsb"] for c in s["gyr"])
    g = tuple(float(c) for c in s["grav"])
    dt = (ts - last_ts) / 1e9 if last_ts else 0.0
    if not 0.0 < dt < 0.15:
        dt = min(max(now - last_sample, 0.001), 0.05) if last_sample else 0.01
    last_ts, last_sample = ts, now
    rates = eng.step(w, g, dt)
    p = eng.p
    out = trk.update(*rates, dt, eng.g, eng.lat, (p["pitch_sign"], p["roll_sign"], p["yaw_sign"]), tuple(s["att"]))
    game = flt.apply((0.0, 0.0, 0.0) + tuple(trk.head), dt)
    rows.append({"dt": dt, "rates": list(rates), "bias": list(eng.bias), "bias_ok": eng.bias_ok,
                 "head": list(trk.head), "out": list(out), "used": trk.used, "game": list(game)})
json.dump({"device": DEVICE, "game": GAME, "rows": rows}, open(out_path, "w"))
print(len(rows), "lignes ->", out_path)
