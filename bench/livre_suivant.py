#!/usr/bin/env python3
"""Prochain lot du livre : la première ligne de livre-plan.txt qui n'est pas
encore faite (pas de fichier dans DONNEES/resultats) et dont chaque « + » se
résout par un coup connu (dans DONNEES/livre.tsv ou le livre du bot).
Écrit « ligne=… » et « nom=… » pour GitHub Actions ; rien s'il n'y a plus rien.

  python3 bench/livre_suivant.py bench/livre-plan.txt donnees bot/data/book-live.tsv
"""
import os, re, sys

plan, donnees, livre_bot = sys.argv[1], sys.argv[2], sys.argv[3]

def connus():
    coups = {}
    for chemin in (livre_bot, os.path.join(donnees, "livre.tsv")):
        if os.path.exists(chemin):
            for ligne in open(chemin, encoding="utf-8"):
                champs = ligne.rstrip("\n").split("\t")
                if len(champs) >= 2:
                    coups[champs[0].strip()] = champs[1].strip()
    return coups

def nom(ligne):
    return re.sub(r"[^A-Za-z0-9]+", "_", ligne).strip("_") or "premier"

coups = connus()
for brute in open(plan, encoding="utf-8"):
    brute = brute.split("#")[0].strip()
    if not brute:
        continue
    jetons = [] if brute == "-" else brute.split()
    ligne = []
    for j in jetons:
        if j == "+":
            coup = coups.get(" ".join(ligne))
            if coup is None:
                break
            ligne.append(coup)
        else:
            ligne.append(j)
    else:
        resolue = " ".join(ligne)
        n = nom(resolue)
        if os.path.exists(os.path.join(donnees, "resultats", n + ".tsv")):
            continue
        print(f"ligne={resolue}")
        print(f"nom={n}")
        sys.exit(0)
