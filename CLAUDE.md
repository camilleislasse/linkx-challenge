# Faire tourner le bot sur le serveur

Ce dépôt est déployé sur le serveur de l'utilisateur, et ce fichier est ton
mode d'emploi. Répondre à l'utilisateur **en français**, simplement. Ici, on
fait seulement tourner le bot (dossier `bot/`) : pas de modification du code,
pas de tests. Le développement se fait ailleurs ; les nouvelles versions
arrivent par ce dépôt, sous forme d'étiquettes git.

## Le contexte

Tournoi Linkx de marmelab (https://marmelab.com/linkx/). Notre bot est un
serveur HTTP (moteur Rust, dossier `bot/`) ; la plateforme lui envoie un
POST à chaque coup, signé en HMAC avec un secret partagé.

**Vague décisive : nuit du mercredi 30 septembre au jeudi 1er octobre 2026,
0 h – 12 h, heure de Paris.** Environ 88 parties, 2 appels simultanés au plus,
6 s par coup réseau compris : un dépassement fait perdre la partie.

Adresse publique visée : **`https://oro.multimod.ovh`**, domaine réservé au
bot. La plateforme exige du https, sans port explicite et avec un nom de
domaine : le Traefik déjà en place sur la machine relaie ce domaine vers le
conteneur du bot, qui écoute sur le port 21345 (jamais ouvert à Internet).
Vérifier d'abord qu'aucun autre service n'utilise déjà ce domaine (`dig
oro.multimod.ovh` doit donner l'IP de la machine).

## Trois consignes

- **Le secret de la plateforme** : l'utilisateur crée lui-même `bot/.env` à
  partir de `bot/.env.example` et y met le secret (`LINKX_BOT_SECRET=...`,
  sans guillemets), droits 600. Ne jamais l'afficher, ni le committer, ni le mettre
  dans l'image.
- Faire tourner **une version précise** (étiquette `v11`, `v12`…).
- **Rien ne bascule sans l'accord de l'utilisateur**, et on ne touche pas
  aux autres services de la machine ni à la configuration de Traefik.

## Prérequis

Docker avec `docker compose` (version 2 : `docker compose version`) et git.
Toutes les commandes partent de la **racine du dépôt cloné**.

## 1. Construire et lancer le bot

```
git fetch --tags && git checkout v11
cd bot
VERSION=v11 docker compose -f compose.yaml up -d --build
curl -s http://127.0.0.1:21345/health
docker compose -f compose.yaml logs -f bot
```

À cette étape, le bot tourne sans relais (`-f compose.yaml` : Traefik n'est
pas encore renseigné).

Toujours construire sur ce serveur (le moteur est optimisé pour le processeur
qui le compile). Vérification du moteur, qui doit afficher
`3Ir14 … prof 9 296883 nœuds` (résultat identique au Mac) :

```
printf '60000\t4Tr24 3Ir15\n' | docker run --rm -i -e LINKX_NODES=300000 linkx:v11 play
```

- Le bot redémarre seul s'il plante (`restart: unless-stopped`).
- Une ligne de journal par coup : la durée doit rester sous ~4,3 s.
- 6 fils par recherche × 2 recherches simultanées = 12 cœurs : pendant la
  vague, rien d'autre de lourd ne doit tourner sur la machine.

## 2. Le relais par Traefik

Le domaine est servi par **Traefik**, qui découvre les conteneurs par leurs
étiquettes : on ne touche pas à sa configuration, on ajoute seulement des
étiquettes au bot (fichier `compose.traefik.yaml`). Il faut y reporter trois
valeurs propres à ce Traefik, à lire sans rien modifier :

- **son réseau Docker** : `docker inspect <conteneur traefik>` (rubrique
  `Networks`), ou les étiquettes `traefik.docker.network` des autres services ;
- **son point d'entrée https** (souvent `websecure`) et **son résolveur de
  certificats** (souvent `letsencrypt`) : dans sa configuration statique
  (`traefik.yml`, `traefik.toml` ou les arguments `--entrypoints…` et
  `--certificatesresolvers…` de son conteneur), ou dans les étiquettes des
  autres services déjà publiés.

Reporter ces valeurs dans `bot/.env` (lignes `TRAEFIK_…`, déjà prévues par
l'exemple ; la ligne `COMPOSE_FILE` fait utiliser les deux fichiers à toutes
les commandes `docker compose`). Ne modifier que ces lignes.

Puis relancer depuis `bot/` : `VERSION=v11 docker compose up -d`. Traefik
obtient seul le certificat du domaine (quelques secondes à la première
requête).

Contrôle depuis le serveur :

```
curl -s -X POST https://oro.multimod.ovh -d '{}' -o /dev/null -w '%{http_code}\n'
```

Réponse attendue : **401** (le bot répond, et refuse une requête non signée,
ce qui est normal). Un 404 veut dire que Traefik n'a pas pris la règle
(réseau, point d'entrée ou `traefik.enable`) ; un 502, qu'il n'atteint pas le
bot (réseau). Les journaux de Traefik (`docker logs <conteneur traefik>`)
disent pourquoi. Vérifier aussi que les autres services répondent toujours.
Le test complet (requête signée) est fait depuis le Mac de l'utilisateur.

## 3. Changer de version

Depuis la racine du dépôt :

```
git fetch --tags && git checkout v12
cd bot && VERSION=v12 docker compose up -d --build
```

Retour arrière : même chose avec l'étiquette précédente. Éviter pendant une
partie : la coupure de quelques secondes fait perdre le coup demandé à ce
moment-là.

## En cas de souci

- Le bot ne répond plus : `docker compose logs bot`, puis `docker compose restart bot`.
- Requêtes refusées (401) sur de vrais coups : secret absent ou faux dans `.env`.
- Doute sérieux : prévenir l'utilisateur. Son Mac reste prêt en secours
  (il suffit de remettre l'ancienne adresse sur la plateforme).
