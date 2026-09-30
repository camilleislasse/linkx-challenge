//! Banc de duel : deux joueurs (processus parlant le protocole de `play`)
//! s'affrontent par paires de parties aux couleurs échangées, jusqu'à ce que le
//! test séquentiel (SPRT) tranche ou que le nombre maximal de parties soit atteint.
//!
//!   arena --a "LINKX_NODES=1500000 LINKX_LMR_INDEX=2 play" --b "LINKX_NODES=1500000 play" --sprt 0,10 --concurrency 10
//!   arena --a "LINKX_SOFT_PCT=60 play" --b play --budget 500 --sprt 0,10 --concurrency 4
//!
//! Avec `LINKX_NODES`, chaque coup s'arrête après ce nombre de positions : le
//! résultat ne dépend plus de la vitesse du cœur, et tous les cœurs servent. Au
//! temps (`--budget`, en ms), seuls des cœurs identiques donnent un banc juste.
//!
//! Le verdict se lit sur les paires (pentanomial) : les deux parties d'une paire
//! partent de la même ouverture, et compter la paire retire le bruit de
//! l'ouverture elle-même. `--trinomial` revient au décompte partie par partie.
//!
//! `--offset N` commence aux ouvertures de la paire N : plusieurs machines qui se
//! partagent un même test jouent ainsi des ouvertures différentes (à nombre de
//! positions fixe, une même ouverture redonne exactement la même partie).
//!
//! `--record fichier.tsv` y ajoute chaque position des parties jouées (hors
//! ouverture), étiquetée par le résultat pour le joueur au trait, au format de
//! `datagen`, pour le réglage automatique.
//!
//! Ouvertures : les 17 départs du tournoi, prolongés de 0 à 2 coups choisis
//! parmi les 4 meilleurs, pour que deux moteurs déterministes ne rejouent pas
//! la même partie.

use engine::board::Move;
use engine::notation::{format_move, parse_move, Game};
use engine::search::Search;
use engine::tt::Table;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const TOURNAMENT_STARTS: [&str; 17] = [
    "", "4Tr24", "4Lr38", "4Lsr15", "4Lr35", "4Lr36", "4Tr23", "4Tr21", "3Lr23", "3Lr28", "3Ir15", "4Tr22",
    "3Ir12", "3Ir17", "3Ir14", "2r15", "3Lr24",
];

struct Player {
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Player {
    fn spawn(cmd: &str) -> Player {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("lancement impossible de « {cmd} » : {e}"));
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Player { _child: child, input, output }
    }

    /// Coup du joueur sur la partie `record`, dans le budget en ms.
    fn ask(&mut self, budget: u64, record: &str) -> String {
        writeln!(self.input, "{budget}\t{record}").unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        if self.output.read_line(&mut line).unwrap_or(0) == 0 {
            eprintln!("Un joueur s'est arrêté sans répondre : banc interrompu.");
            std::process::exit(2);
        }
        line.split('\t').next().unwrap_or("").trim().to_string()
    }
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

/// Générateur pseudo-aléatoire reproductible (splitmix64).
fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Ouverture de la paire `index` : un départ du tournoi, puis 0 à 2 coups tirés
/// parmi les 4 meilleurs selon une recherche courte. Des coups au hasard
/// donneraient des positions qu'aucune IA sérieuse n'atteint. Une petite table
/// neuve par ouverture : le choix ne dépend que de `index` et `seed`.
fn opening(index: usize, seed: u64) -> String {
    let mut next = rng(seed ^ (index as u64).wrapping_mul(0xA24B_AED4_963E_E407));
    let start = TOURNAMENT_STARTS[index % TOURNAMENT_STARTS.len()];
    let mut game = Game::parse(start).unwrap();
    let extra = if index < TOURNAMENT_STARTS.len() { 0 } else { 1 + next() % 2 };
    let mut judge = Search::with_table(Arc::new(Table::new(1)));
    judge.depth_limit = Some(2);
    for _ in 0..extra {
        let mut scored: Vec<(i32, Move)> = Vec::new();
        for m in game.position.legal_moves(game.position.active) {
            let mut trial = game.clone();
            trial.apply(m).unwrap();
            if trial.outcome.is_some() {
                continue;
            }
            // Score du point de vue de celui qui joue `m`.
            let r = judge.best_move(&trial.position, Duration::from_secs(10));
            let score = if trial.position.active == game.position.active { r.score } else { -r.score };
            scored.push((score, m));
        }
        if scored.is_empty() {
            break;
        }
        scored.sort_by_key(|&(score, _)| -score);
        let m = scored[(next() % scored.len().min(4) as u64) as usize].1;
        game.apply(m).unwrap();
    }
    game.serialize()
}

/// Résultat d'une partie de la paire `pair`, pour A : 1 victoire, 0,5 nul, 0 défaite.
struct GameResult {
    pair: usize,
    score_a: f64,
    record: String,
    detail: String,
}

fn play_game(pair: usize, start: &str, a_is_first_to_move: bool, a: &mut Player, b: &mut Player, budgets: (u64, u64)) -> GameResult {
    let mut game = Game::parse(start).unwrap();
    let a_color = if a_is_first_to_move { game.position.active } else { game.position.active.other() };
    loop {
        if let Some(outcome) = game.outcome {
            let score_a = match outcome.winner() {
                None => 0.5,
                Some(p) if p == a_color => 1.0,
                Some(_) => 0.0,
            };
            return GameResult { pair, score_a, record: game.serialize(), detail: format!("{outcome:?}") };
        }
        let a_to_move = game.position.active == a_color;
        let record = game.serialize();
        let token = if a_to_move { a.ask(budgets.0, &record) } else { b.ask(budgets.1, &record) };
        let faulty = |why: String| GameResult {
            pair,
            score_a: if a_to_move { 0.0 } else { 1.0 },
            record: record.clone(),
            detail: format!("faute de {} : {why}", if a_to_move { "A" } else { "B" }),
        };
        let Some(m) = parse_move(&token) else { return faulty(format!("coup illisible « {token} »")) };
        if let Err(reason) = game.apply(m) {
            return faulty(format!("{} refusé ({reason})", format_move(m)));
        }
    }
}

/// Écrit les positions d'une partie terminée, étiquetées par son résultat.
fn write_positions(file: &mut std::fs::File, record: &str) {
    let Ok(game) = Game::parse(record) else { return };
    let Some(outcome) = game.outcome else { return };
    let mut replay = Game::new(game.first);
    for (i, entry) in game.history.iter().enumerate() {
        let engine::notation::Entry::Move(m) = entry else { continue };
        if i >= 3 {
            let result = match outcome.winner() {
                None => "0.5",
                Some(w) if w == replay.position.active => "1",
                Some(_) => "0",
            };
            let _ = writeln!(file, "{}\t{result}", replay.serialize());
        }
        if replay.apply(*m).is_err() {
            return;
        }
    }
}

fn expected(elo: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-elo / 400.0))
}

/// Log-vraisemblance du SPRT, approximation normale comme fishtest : `n`
/// observations de score moyen `mean` et de variance `var`, à comparer à
/// `elo0` et `elo1`.
fn llr(n: f64, mean: f64, var: f64, elo0: f64, elo1: f64) -> Option<f64> {
    if n < 2.0 || var <= 0.0 {
        return None;
    }
    let (s0, s1) = (expected(elo0), expected(elo1));
    Some(n * (s1 - s0) * (2.0 * mean - s0 - s1) / (2.0 * var))
}

/// Moyenne, variance et nombre d'observations d'une distribution de scores
/// (`counts[k]` observations valant `k / (len - 1)`).
fn moments(counts: &[f64]) -> (f64, f64, f64) {
    let n: f64 = counts.iter().sum();
    let value = |k: usize| k as f64 / (counts.len() - 1) as f64;
    let mean = (0..counts.len()).map(|k| counts[k] * value(k)).sum::<f64>() / n.max(1.0);
    let var = (0..counts.len()).map(|k| counts[k] * (value(k) - mean).powi(2)).sum::<f64>() / n.max(1.0);
    (n, mean, var)
}

fn elo(score: f64) -> f64 {
    let s = score.clamp(1e-6, 1.0 - 1e-6);
    -400.0 * (1.0 / s - 1.0).log10()
}

fn main() {
    let cmd_a = arg("--a").expect("--a <commande>");
    let cmd_b = arg("--b").expect("--b <commande>");
    let budget: u64 = arg("--budget").and_then(|s| s.parse().ok()).unwrap_or(60_000);
    // Budgets propres à chaque camp, pour simuler le tournoi (l'IA de la maison y joue à 700 ms).
    let budgets = (
        arg("--budget-a").and_then(|s| s.parse().ok()).unwrap_or(budget),
        arg("--budget-b").and_then(|s| s.parse().ok()).unwrap_or(budget),
    );
    let concurrency: usize = arg("--concurrency").and_then(|s| s.parse().ok()).unwrap_or(4);
    let max_games: usize = arg("--games").and_then(|s| s.parse().ok()).unwrap_or(2000);
    let seed: u64 = arg("--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let offset: usize = arg("--offset").and_then(|s| s.parse().ok()).unwrap_or(0);
    let sprt: Option<(f64, f64)> = arg("--sprt").map(|s| {
        let (a, b) = s.split_once(',').expect("--sprt elo0,elo1");
        (a.parse().unwrap(), b.parse().unwrap())
    });
    let (verbose, trinomial) = (flag("--verbose"), flag("--trinomial"));
    let mut record_file = arg("--record").map(|path| {
        std::fs::OpenOptions::new().create(true).append(true).open(path).expect("fichier --record")
    });
    let bound = (0.95f64 / 0.05).ln(); // α = β = 0,05

    println!(
        "A = {cmd_a}\nB = {cmd_b}\nbudget A {} ms, B {} ms, {concurrency} en parallèle, {max_games} parties au plus{}",
        budgets.0,
        budgets.1,
        sprt.map_or(String::new(), |(e0, e1)| format!(
            ", SPRT [{e0}, {e1}] {}",
            if trinomial { "par partie" } else { "par paires" }
        ))
    );

    let next_pair = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<GameResult>();
    let started = Instant::now();
    for _ in 0..concurrency {
        let (cmd_a, cmd_b, next_pair, stop, tx) =
            (cmd_a.clone(), cmd_b.clone(), Arc::clone(&next_pair), Arc::clone(&stop), tx.clone());
        std::thread::spawn(move || {
            let mut a = Player::spawn(&cmd_a);
            let mut b = Player::spawn(&cmd_b);
            while !stop.load(Ordering::Relaxed) {
                let played = next_pair.fetch_add(1, Ordering::Relaxed);
                if played * 2 >= max_games {
                    break;
                }
                let pair = offset + played;
                let start = opening(pair, seed);
                for a_first in [true, false] {
                    if tx.send(play_game(pair, &start, a_first, &mut a, &mut b, budgets)).is_err() {
                        return;
                    }
                }
            }
        });
    }
    drop(tx);

    // Parties : défaites, nuls, victoires de A. Paires : 0 à 4 demi-points sur 2.
    let mut games = [0.0f64; 3];
    let mut pairs = [0.0f64; 5];
    let mut first_half: HashMap<usize, f64> = HashMap::new();
    let mut verdict = "nombre maximal de parties atteint";
    for r in rx {
        games[(r.score_a * 2.0).round() as usize] += 1.0;
        if let Some(first) = first_half.remove(&r.pair) {
            pairs[((first + r.score_a) * 2.0).round() as usize] += 1.0;
        } else {
            first_half.insert(r.pair, r.score_a);
        }
        if let Some(file) = record_file.as_mut() {
            write_positions(file, &r.record);
        }
        if verbose || r.detail.starts_with("faute") {
            println!("  {:<4} {}  {}", r.score_a, r.detail, r.record);
        }

        let (n, mean, var) = moments(if trinomial { &games } else { &pairs });
        let current = sprt.and_then(|(e0, e1)| llr(n, mean, var, e0, e1));
        let played = games.iter().sum::<f64>();
        if played as usize % 20 == 0 || current.is_some_and(|x| x.abs() >= bound) {
            let err = 1.96 * (var / n.max(1.0)).sqrt();
            let [l, d, w] = games;
            println!(
                "{played:>5}/{max_games} parties ({:>3.0} %)  A {w}-{d}-{l}  {:.1} %  Elo {:+.0} [{:+.0}, {:+.0}]{}  {:.0} s",
                played / max_games as f64 * 100.0,
                mean * 100.0,
                elo(mean),
                elo(mean - err),
                elo(mean + err),
                current.map_or(String::new(), |x| format!("  LLR {x:.2} / ±{bound:.2}")),
                started.elapsed().as_secs_f64()
            );
        }
        if let Some(x) = current {
            if x.abs() >= bound {
                verdict = if x > 0.0 { "A est plus fort (H1 acceptée)" } else { "pas de gain démontré (H0 acceptée)" };
                break;
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    let [l, d, w] = games;
    let n = w + d + l;
    let s = (w + d / 2.0) / n.max(1.0);
    println!("\nVerdict : {verdict}");
    println!("Bilan A {w}-{d}-{l} sur {n} parties, {:.1} %, Elo {:+.0}", s * 100.0, elo(s));
    println!("Paires (0 à 2 points pour A) : {:?}", pairs.map(|c| c as u32));
    // Les processus des joueurs s'arrêtent avec le banc.
    std::process::exit(0);
}
