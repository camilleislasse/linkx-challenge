#!/bin/sh
# Empreinte du moteur : pour chaque position de empreinte.tsv, le coup, le score,
# la profondeur et le nombre de positions à 1 500 000 positions par coup. Un
# changement qui ne doit toucher que la vitesse la laisse identique ; un
# changement qui modifie le jeu la met à jour dans le même commit.
#   bot/tests/empreinte.sh bot/target/release/play
set -eu
play=$1
dir=$(dirname "$0")
status=0
while IFS="$(printf '\t')" read -r record move rest; do
  got=$(printf '60000\t%s\n' "$record" | LINKX_NODES=1500000 "$play")
  if [ "$got" != "$(printf '%s\t%s' "$move" "$rest")" ]; then
    echo "différence sur « $record » : attendu « $move $rest », obtenu « $got »"
    status=1
  fi
done < "$dir/empreinte.tsv"
[ $status = 0 ] && echo "empreinte identique ($(wc -l < "$dir/empreinte.tsv" | tr -d ' ') positions)"
exit $status
