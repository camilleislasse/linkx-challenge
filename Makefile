# Commandes du serveur de prod (voir DEPLOIEMENT.md), depuis la racine du dépôt.
#
#   make deploy VERSION=v14   récupère l'étiquette, reconstruit l'image et relance le bot
#   make moteur VERSION=v14   vérifie le moteur de l'image (même résultat que sur le Mac)
#   make status               version en place, état du conteneur, santé
#   make logs                 journal du bot en direct (un coup par ligne)
#   make restart              redémarre le bot sans changer de version
#   make check                le domaine public répond (401 attendu : requête non signée)
#
# Pas de changement de version pendant une vague (mercredi soir 0 h – jeudi 12 h) :
# la coupure de quelques secondes fait perdre le coup demandé à ce moment-là.

VERSION ?= $(error précise la version : make deploy VERSION=v14)
COMPOSE = cd bot && VERSION=$(VERSION) docker compose

.PHONY: deploy moteur status logs restart check

deploy:
	git fetch --tags
	git checkout $(VERSION)
	$(COMPOSE) up -d --build
	@sleep 3
	@curl -s http://127.0.0.1:21345/health && echo
	@$(COMPOSE) logs --tail 3 bot

moteur:
	@printf '60000\t4Tr24 3Ir15\n' | docker run --rm -i -e LINKX_NODES=300000 linkx:$(VERSION) play
	@echo "attendu : 3Ir14  score 3944 prof 9 296883 nœuds"

status:
	@echo "version extraite : $$(git describe --tags --always)"
	@cd bot && docker compose ps
	@curl -s http://127.0.0.1:21345/health && echo

logs:
	cd bot && docker compose logs -f bot

restart:
	cd bot && docker compose restart bot

check:
	@curl -s -X POST https://oro.multimod.ovh -d '{}' -o /dev/null -w '%{http_code}\n'
