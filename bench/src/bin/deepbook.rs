//! Livre profond : cherche longuement chaque position d'une liste, et écrit une
//! ligne au format du livre (notation, coup, profondeur, score), suivie de
//! « exact » quand la recherche a prouvé la valeur de la position.
//!
//!   deepbook --list ""                 # positions après chaque coup légal du plateau vide
//!   deepbook --list "4Lsr14"           # positions après chaque réponse à 4Lsr14
//!   deepbook --positions liste.txt --part 3/20 --minutes 90
//!
//! `--list` n'écrit qu'une position par paire de reflets gauche-droite : le
//! livre retrouve l'autre par son reflet. `--part k/n` ne garde que les
//! positions de rang k modulo n, pour répartir la liste entre machines. Les
//! fils de recherche se règlent par `LINKX_SEARCH_THREADS`, la table par
//! `LINKX_TT_MB`.

use engine::notation::{format_move, Game};
use engine::search::{hash, Search, WIN_THRESHOLD};
use std::collections::HashSet;
use std::io::Write;
use std::time::Duration;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// Positions après chaque coup légal depuis `record`, sans les reflets.
fn list(record: &str) {
    let game = Game::parse(record).expect("partie illisible");
    let mut seen = HashSet::new();
    for m in game.position.legal_moves(game.position.active) {
        let mut next = game.clone();
        next.apply(m).unwrap();
        if next.outcome.is_some() {
            continue;
        }
        let (key, mirror) = (hash(&next.position), hash(&next.position.mirrored()));
        if seen.contains(&mirror) {
            continue;
        }
        seen.insert(key);
        println!("{}", next.serialize());
    }
}

fn main() {
    if let Some(record) = arg("--list") {
        return list(&record);
    }
    let path = arg("--positions").expect("--positions fichier ou --list notation");
    let (k, n) = arg("--part")
        .and_then(|p| p.split_once('/').map(|(k, n)| (k.parse().unwrap(), n.parse().unwrap())))
        .unwrap_or((0usize, 1usize));
    let minutes: f64 = arg("--minutes").and_then(|m| m.parse().ok()).unwrap_or(30.0);
    let text = std::fs::read_to_string(&path).expect("liste illisible");
    let records: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    for (i, record) in records.iter().enumerate() {
        if i % n != k {
            continue;
        }
        let game = Game::parse(record).expect("position illisible");
        // Une recherche neuve par position : chaque ligne du livre ne dépend que d'elle.
        let mut search = Search::new();
        let r = search.best_move(&game.position, Duration::from_secs_f64(minutes * 60.0));
        let proved = r.exact || r.score.abs() >= WIN_THRESHOLD;
        println!("{record}\t{}\t{}\t{}{}", format_move(r.best), r.depth, r.score, if proved { "\texact" } else { "" });
        let _ = std::io::stdout().flush();
    }
}
