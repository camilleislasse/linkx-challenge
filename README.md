# LinkxMaster

Bot pour le [tournoi Linkx](https://marmelab.com/linkx/) de marmelab

Linkx se joue sur un plateau de 9 × 9 : chaque joueur fait tomber ses pièces
(des polyominos) dans les colonnes, et gagne en reliant deux bords opposés.
Chaque IA du tournoi est un service HTTP que la plateforme appelle à chaque
coup.

## Le moteur

Écrit en Rust, dans `bot/` :

- **Plateau en bitboards** : le plateau n'a jamais de trou, une position se
  résume à deux masques de 81 bits et à la hauteur des colonnes.
- **Recherche alpha-bêta** (PVS) à approfondissement itératif, avec table de
  transposition, coups tueurs, historique, réductions des coups tardifs et
  ProbCut ; recherche parallèle sur plusieurs cœurs (Lazy SMP) ; résolution
  exacte des fins de partie.
- **Évaluation** : distance de chaque joueur à la connexion (parcours en
  largeur sur les groupes de pièces), largeur des chemins, zones, mobilité,
  réserve de pièces ; poids réglés automatiquement sur des parties
  (méthode de Texel), différents en début et en fin de partie.
- **Livre d'ouverture** calculé hors ligne pour les départs imposés du tournoi.
- **Réflexion pendant le tour adverse**, et marge de sécurité sur le délai
  de réponse.

## Organisation

```
bot/
├── src/            le moteur (bibliothèque)
│   └── bin/
│       ├── server.rs   le service HTTP appelé par la plateforme
│       └── play.rs     le même moteur en ligne de commande
├── data/           le livre d'ouverture
├── Dockerfile
├── compose.yaml
└── compose.traefik.yaml
```

## Lancer

Avec Docker :

```
cd bot
echo "LINKX_BOT_SECRET=<secret de la plateforme>" > .env
docker compose up -d --build
curl http://127.0.0.1:21345/health
```

Ou directement avec Rust :

```
cd bot
cargo build --release
LINKX_BOT_SECRET=<secret> PORT=21345 target/release/server
```

Un coup en ligne de commande (budget en millisecondes, puis la partie en
notation Linkx) :

```
printf '3000\t4Tr24 3Ir15\n' | target/release/play
```

Le détail du déploiement (relais HTTPS, changement de version, en cas de
souci) est dans [`CLAUDE.md`](CLAUDE.md).

## Réglages

Tout se règle par variables d'environnement :

| Variable | Rôle | Défaut |
|---|---|---|
| `LINKX_BOT_SECRET` | secret partagé avec la plateforme (signature HMAC) | — |
| `PORT` | port d'écoute | 21345 dans Docker |
| `SEARCH_BUDGET_MS` | temps de réflexion par coup | 4500 |
| `RESPONSE_MARGIN_MS` | marge gardée sur le délai de la plateforme | 1200 |
| `LINKX_SEARCH_THREADS` | cœurs utilisés par recherche | 1 |
| `LINKX_BOOK` | fichier du livre d'ouverture | aucun |

Les versions sont étiquetées (`v11`, `v12`…) : le serveur fait toujours
tourner une étiquette précise.
