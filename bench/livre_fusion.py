#!/usr/bin/env python3
"""Fusionne des résultats du livre profond : une ligne par position, la
meilleure (prouvée d'abord, puis la plus profonde), triée par position.

  python3 bench/livre_fusion.py resultats/*.tsv > livre.tsv
"""
import sys

meilleures = {}
for chemin in sys.argv[1:]:
    for ligne in open(chemin, encoding="utf-8"):
        champs = ligne.rstrip("\n").split("\t")
        if len(champs) < 4:
            continue
        rang = (len(champs) > 4 and champs[4] == "exact", int(champs[2]))
        if champs[0] not in meilleures or rang > meilleures[champs[0]][0]:
            meilleures[champs[0]] = (rang, "\t".join(champs))

for position in sorted(meilleures):
    print(meilleures[position][1])
