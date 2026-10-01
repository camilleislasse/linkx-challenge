//! Livre profond : cherche longuement chaque position d'une liste, et écrit une
//! ligne au format du livre (notation, coup, profondeur, score), suivie de
//! « exact » quand la recherche a prouvé la valeur de la position.
//!
//!   deepbook --list ""                 # positions après chaque coup légal du plateau vide
//!   deepbook --list "4Lsr14"           # positions après chaque réponse à 4Lsr14
//!   deepbook --positions liste.txt --part 3/20 --budget 320
//!
//! `--list` n'écrit qu'une position par paire de reflets gauche-droite : le
//! livre retrouve l'autre par son reflet. `--part k/n` ne garde que les
//! positions de rang k modulo n, pour répartir la liste entre machines.
//!
//! `--budget` est le temps total de la machine, en minutes. Une première passe
//! courte règle les positions faciles (souvent prouvées), une position par fil ;
//! la seconde partage tout le temps restant entre les positions non prouvées,
//! tous les fils sur chacune, la part de chacune étant recalculée au fil de
//! l'eau. Toutes les recherches partagent la table de transposition : les
//! positions d'une même liste se ressemblent.
//!
//! Une victoire prouvée laisse dans la table notre coup gagnant contre chaque
//! réponse adverse : ces positions sont écrites aussi (le sous-livre).
//!
//! Réglages : `LINKX_SEARCH_THREADS` (fils), `LINKX_TT_MB` (table),
//! `LINKX_SOLVER=1` (mode preuve, conseillé). Le suivi va sur la sortie d'erreur.

use engine::notation::{format_move, Game};
use engine::search::{hash, Report, Search, WIN_THRESHOLD};
use engine::tt::{Bound, Table};
use std::collections::HashSet;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Durée maximale d'une recherche de la première passe.
const FIRST_PASS: Duration = Duration::from_secs(120);

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

fn proved(r: &Report) -> bool {
    r.exact || r.score.abs() >= WIN_THRESHOLD
}

/// Les lignes du livre pour `game` : la position elle-même, puis, si elle est
/// gagnée, chaque réponse adverse au coup gagnant dont la table connaît la
/// parade prouvée. Renvoie aussi le nombre de réponses sans parade trouvée.
fn lines(game: &Game, r: &Report, table: &Table) -> (Vec<String>, usize) {
    let record = game.serialize();
    let mut out = vec![format!(
        "{record}\t{}\t{}\t{}{}",
        format_move(r.best),
        r.depth,
        r.score,
        if proved(r) { "\texact" } else { "" }
    )];
    let mut missing = 0;
    if r.score < WIN_THRESHOLD {
        return (out, missing);
    }
    let me = game.position.active;
    let mut after = game.clone();
    after.apply(r.best).unwrap();
    if after.outcome.is_some() || after.position.active == me {
        return (out, missing);
    }
    for reply in after.position.legal_moves(after.position.active) {
        let mut next = after.clone();
        next.apply(reply).unwrap();
        if next.outcome.is_some() || next.position.active != me {
            continue;
        }
        match table.probe(hash(&next.position)) {
            Some(e) if e.score >= WIN_THRESHOLD && e.bound != Bound::Upper && e.best.is_some() => out.push(format!(
                "{}\t{}\t{}\t{}\texact",
                next.serialize(),
                format_move(e.best.unwrap()),
                e.depth,
                e.score
            )),
            _ => missing += 1,
        }
    }
    (out, missing)
}

struct Out {
    table: Arc<Table>,
    started: Instant,
    lock: Mutex<()>,
}

impl Out {
    fn write(&self, game: &Game, r: &Report, spent: Duration) {
        let (lines, missing) = lines(game, r, &self.table);
        let _guard = self.lock.lock().unwrap();
        let mut stdout = std::io::stdout().lock();
        for line in &lines {
            let _ = writeln!(stdout, "{line}");
        }
        let _ = stdout.flush();
        eprintln!(
            "[{:>6.0} s] {} : {} prof {} score {} en {:.0} s{}{}",
            self.started.elapsed().as_secs_f64(),
            game.serialize(),
            format_move(r.best),
            r.depth,
            r.score,
            spent.as_secs_f64(),
            if proved(r) { ", prouvée" } else { "" },
            if lines.len() > 1 || missing > 0 { format!(", sous-livre {} (manque {missing})", lines.len() - 1) } else { String::new() }
        );
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
    let budget = Duration::from_secs_f64(arg("--budget").and_then(|m| m.parse().ok()).unwrap_or(30.0) * 60.0);
    let text = std::fs::read_to_string(&path).expect("liste illisible");
    let mine: Vec<Game> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .enumerate()
        .filter(|(i, _)| i % n == k)
        .map(|(_, r)| Game::parse(r).expect("position illisible"))
        .collect();
    if mine.is_empty() {
        return;
    }
    let threads = engine::params::params().search_threads;
    let out = Out { table: Arc::new(Table::new(engine::params::params().tt_mb)), started: Instant::now(), lock: Mutex::new(()) };

    // Passe 1 : une recherche courte par position, une position par fil ; les
    // prouvées sont écrites aussitôt.
    let first = FIRST_PASS.min(budget * threads as u32 / (4 * mine.len() as u32));
    let next = AtomicUsize::new(0);
    let open = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..threads.min(mine.len()) {
            scope.spawn(|| {
                let mut search = Search::with_table(Arc::clone(&out.table));
                search.threads = 1;
                loop {
                    let i = next.fetch_add(1, Relaxed);
                    let Some(game) = mine.get(i) else { break };
                    let start = Instant::now();
                    let r = search.best_move(&game.position, first);
                    if proved(&r) {
                        out.write(game, &r, start.elapsed());
                    } else {
                        open.lock().unwrap().push((i, r));
                    }
                }
            });
        }
    });

    // Passe 2 : le temps restant, partagé au fil de l'eau entre les positions
    // ouvertes, tous les fils sur chacune.
    let mut open = open.into_inner().unwrap();
    open.sort_by_key(|&(i, _)| i);
    let mut search = Search::with_table(Arc::clone(&out.table));
    let count = open.len();
    for (j, (i, short)) in open.into_iter().enumerate() {
        let left = budget.saturating_sub(out.started.elapsed());
        let share = left / (count - j) as u32;
        let start = Instant::now();
        let r = if share > first { search.best_move(&mine[i].position, share) } else { short };
        out.write(&mine[i], &r, start.elapsed());
    }
}
