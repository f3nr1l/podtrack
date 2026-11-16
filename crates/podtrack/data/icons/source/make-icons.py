#!/usr/bin/env python3
"""Génère les icônes de podtrack depuis le logo source (version blanche, pour fond sombre).

Usage : python3 make-icons.py [source.svg]   (défaut : PodtrackB2.svg), depuis ce dossier.
Écrit dans le dossier parent : podtrack-active / -stopped / -searching (icône de barre, toujours
sur la barre sombre de GNOME) et podtrack.svg (icône d'application, sur carré gris foncé).
"""
import os
import re
import sys
import xml.etree.ElementTree as ET

SVG = "http://www.w3.org/2000/svg"
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.dirname(HERE)


def drawing(path):
    """Contenu dessiné du SVG, sans les métadonnées d'Inkscape ; et sa viewBox."""
    root = ET.parse(path).getroot()

    def clean(e):
        for k in list(e.attrib):
            if k.startswith("{") and not k.startswith("{" + SVG):
                del e.attrib[k]
        for c in list(e):
            if not c.tag.startswith("{" + SVG) or (c.tag.endswith("defs") and len(c) == 0):
                e.remove(c)
            else:
                clean(c)

    clean(root)
    ET.register_namespace("", SVG)
    body = "".join(ET.tostring(c, encoding="unicode") for c in root)
    body = body.replace(' xmlns="http://www.w3.org/2000/svg"', "")
    return re.sub(r'\s+id="[^"]*"', "", body), root.get("viewBox")


def write(name, body, vb):
    with open(os.path.join(OUT, name), "w") as f:
        f.write(f'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="{vb}">{body}</svg>\n')


def main():
    src = os.path.join(HERE, sys.argv[1] if len(sys.argv) > 1 else "PodtrackB2.svg")
    body, vb = drawing(src)
    write("podtrack-active.svg", body, vb)
    write("podtrack-stopped.svg", f'<g opacity="0.45">{body}</g>', vb)
    write("podtrack-searching.svg", f'<g opacity="0.8">{body}</g>'
          '<circle cx="55" cy="55" r="9.5" fill="#f6a700" stroke="#2b2b2b" stroke-width="1.5"/>', vb)
    write("podtrack.svg", '<rect width="65" height="65" rx="14" fill="#29292e"/>'
          f'<g transform="translate(6.5 6.5) scale(0.8)">{body}</g>', "0 0 65 65")


if __name__ == "__main__":
    main()
