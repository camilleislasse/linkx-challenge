#!/usr/bin/env python3
"""Ligne du livre pour la position de départ d'un lot, déduite de ses
positions (un coup plus loin, l'autre camp au trait) : le coup qui mène au
plus mauvais score pour l'autre camp. Prouvée si ce coup mène à une défaite
prouvée de l'autre camp, ou si tous les coups mènent à sa victoire prouvée.

  python3 bench/livre_parent.py "4Tr21 4Lr38 4Lr18" positions.txt lot.tsv >> lot.tsv
"""
import sys

GAGNE = 999_000  # WIN_THRESHOLD du moteur

ligne, positions, lot = sys.argv[1], sys.argv[2], sys.argv[3]
attendues = [p.strip() for p in open(positions, encoding="utf-8") if p.strip()]
prefixe = (ligne + " ") if ligne else ""
enfants = {}
for l in open(lot, encoding="utf-8"):
    champs = l.rstrip("\n").split("\t")
    if len(champs) >= 4 and champs[0] in attendues and champs[0].startswith(prefixe):
        coup = champs[0][len(prefixe):]
        if " " not in coup:
            enfants[coup] = (int(champs[3]), int(champs[2]), len(champs) > 4 and champs[4] == "exact")
if not enfants:
    sys.exit(0)

coup, (score, prof, exact) = min(enfants.items(), key=lambda e: e[1][0])
complet = len(enfants) == len([p for p in attendues if p.startswith(prefixe)])
prouve = (exact and score <= -GAGNE) or (complet and all(e and s >= GAGNE for s, _, e in enfants.values()))
print(f"{ligne}\t{coup}\t{prof + 1}\t{-score}" + ("\texact" if prouve else ""))
