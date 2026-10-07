# Versions

Détail des essais : [`README.md`](../README.md).

- **v1 à v10** : binaires Mac seulement (`labo/versions/`).
- **Depuis v11** : prod sur le serveur de Julien ; une étiquette git par version (`git checkout vN`, voir `DEPLOIEMENT.md`).

| Version | Contenu | Contre la maison (200 ms, 200 parties) |
| --- | --- | --- |
| v1 | Alpha-bêta PVS, table de transposition, tueurs, historique. Évaluation : distance bord à bord sur deux axes, chemins morts faute de réserve, plus grande zone pondérée par le remplissage, tempo. | 36 % (Elo −100, [−153, −51]), 0 faute |
| v2 | v1 avec zone par case 8 (au lieu de 3) et second axe 600 (au lieu de 100). Environ +115 Elo sur v1. | non mesurée |
| v3 | v2 avec largeur du chemin (poids 100, plafond 8), compilée pour le M4. | **47,5 %** (Elo −17, [−66, +31]), 0 faute |
| v4 | v3 avec tri des coups par cases des plus courts chemins. +24 Elo sur v3. | 45,5 % (Elo −31), 400 parties |
| v5 | v4 avec poids réglés par Texel et trois critères nouveaux : mobilité (219 par coup légal d'écart), réserve (163 par case), grandes pièces (−1 185 par pièce). +35 Elo sur v4. | **53,0 % (Elo +21)**, 400 parties |
| v6 | v5 re-réglée par Texel sur ses propres parties, avec urgence (−166), menace imminente (13) et nombre de groupes (−657). +38 Elo sur v5. | 53,6 % (Elo +25), 400 parties ; **54,4 % (Elo +31 ± 12), 3 000 parties à 100 ms** |
| v7 | v6 à l'identique, deux fois plus rapide. +17 Elo sur v6. | non mesurée |
| v8 | v7 avec table de transposition compacte partagée et recherche multicœur (Lazy SMP). À 2 fils : +23 Elo sur v7 à 1 fil. | à temps égal (4,5 s) : 50,7 % |
| v9 | v8 avec usage du temps à 70 % et itération partielle gardée. | non mesurée |
| v10 | v9 + vitesse (voisinage, coupure par la réserve, cache d'évaluation, parcours par composantes) + corrections de la relecture (historique, table, fils auxiliaires, positions résolues, serveur) + LMR logarithmique (+45). | base corrigée sans LMR : **57,5 % à temps égal (4,5 s)** |
| v11 | v10 + ProbCut (dès la profondeur 5, réduction 4, marge 2 000). +25 à 500 ms, ~+28 à 2 s. | bout en bout, conditions du tournoi : **31-3 (91,2 %)**, max 4,29 s, 0 faute. **En ligne pour la vague du 1ᵉʳ octobre**, livre : 17 départs recalculés par v11 à 90 s (profondeur annoncée 16 à 20, contre 12 à 16 en direct) + 3 victoires prouvées. L'ancien livre de réponses (profondeur 11, moteur v10) est écarté : v11 le dépasse en direct. |
| v12 | v11 à coups identiques, **×1,97** (profils de relief, tri sans allocation et premiers coups choisis un à un, groupes réutilisés, préchargement mémoire) + table de 64 Mo. Au temps contre v11 : **+25 Elo [+7, +44]** (1 400 parties, version ×1,6). | **En ligne pour la vague du 1ᵉʳ octobre** sur le serveur de Julien (`oro.multimod.ovh`) ; partie signée de bout en bout : max 4,43 s. |
| v13 | v12 + composantes mises à jour depuis le nœud parent des feuilles, chute calculée directement pour les coups légaux, relief sans branchement, profils sans boucle. **×2,2 sur v11** (+10 % sur v12), coups identiques (13 tests, empreinte). | Étiquetée le 30 septembre au soir ; deux parties signées simultanées : max 4,21 s, 0 faute. +3 à 4 Elo attendus sur v12. |
| v14 | v13 (moteur inchangé, même empreinte) + **livre profond** calculé sur GitHub Actions : 14 932 positions, la plupart prouvées. Contre les T, nos anciennes défenses étaient prouvées perdantes : `4Tr24` → **`4Lr31`**, `4Tr21` → **`4Lsr17`**, `4Lr35` → **`4Lsr12`**, `4Tr23` → `4Tr27`. En bleu, `4Lsr14` gardé (`4Lsr14 3Ir15 3Ir14` : victoire prouvée). Serveur : réflexion pendant le tour adverse même quand une autre partie cherche (`LINKX_PONDER_MAX_ACTIVE=0` pour revenir en arrière). | Nos 13 parties perdues ou nulles en blanc de la vague du 1ᵉʳ octobre : le livre dévie au 2ᵉ coup dans les 13. Contre la maison (1,5 s, 34 parties) : 30-3-1, 0 faute. Pour la vague du 8 octobre. |
