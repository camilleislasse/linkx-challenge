#!/usr/bin/env python3
"""Prochain lot du livre, écrit dans positions.txt, avec « ligne=… » et
« nom=… » pour GitHub Actions (rien s'il n'y a plus rien à faire).

Le lot est la première ligne de livre-plan.txt pas encore faite (pas de
fichier dans DONNEES/resultats) et dont chaque « + » se résout par un coup
connu (dans DONNEES/livre.tsv ou le livre du bot). S'y ajoutent les positions
perdues des lignes déjà faites (machine coupée par GitHub) : celles qui n'ont
de résultat dans aucun fichier.

  python3 bench/livre_suivant.py bench/livre-plan.txt donnees bot/data/book-live.tsv deepbook
"""
import os, re, subprocess, sys

plan, donnees, livre_bot, deepbook = sys.argv[1:5]

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

def positions(ligne):
    sortie = subprocess.run([deepbook, "--list", ligne], capture_output=True, text=True, check=True).stdout
    return [p for p in sortie.splitlines() if p.strip()]

def lignes():
    """Les lignes du plan dont chaque « + » se résout, dans l'ordre."""
    coups = connus()
    for brute in open(plan, encoding="utf-8"):
        brute = brute.split("#")[0].strip()
        if not brute:
            continue
        ligne = []
        for j in [] if brute == "-" else brute.split():
            if j == "+":
                coup = coups.get(" ".join(ligne))
                if coup is None:
                    break
                ligne.append(coup)
            else:
                ligne.append(j)
        else:
            yield " ".join(ligne)

faites, a_faire = [], []
for ligne in lignes():
    fait = os.path.exists(os.path.join(donnees, "resultats", nom(ligne) + ".tsv"))
    (faites if fait else a_faire).append(ligne)

dossier = os.path.join(donnees, "resultats")
trouvees = set()
for fichier in os.listdir(dossier) if os.path.isdir(dossier) else []:
    trouvees |= {l.split("\t")[0] for l in open(os.path.join(dossier, fichier), encoding="utf-8")}
perdues = [p for ligne in faites for p in positions(ligne) if p not in trouvees]

if a_faire:
    ligne, liste = a_faire[0], perdues + positions(a_faire[0])
elif perdues:
    ligne, liste = "reprise", perdues
else:
    sys.exit(0)
open("positions.txt", "w", encoding="utf-8").write("".join(p + "\n" for p in liste))
print(f"ligne={ligne}")
print(f"nom={nom(ligne) if a_faire else 'reprise_' + str(len(os.listdir(dossier)))}")
