//! Recherche alpha-bêta (negamax, fenêtre nulle), approfondissement itératif,
//! table de transposition, coups tueurs et historique. Elle s'arrête au temps et
//! joue le meilleur coup de la dernière itération achevée.
//!
//! **Fin de partie exacte.** Chaque pose consomme une pièce : le nombre de poses
//! restantes borne la partie. Une itération qu'aucune feuille n'a coupée par la
//! profondeur rend la valeur de jeu vraie, et la recherche s'arrête là.

use crate::board::{After, Move, Outcome, Player, Position};
use crate::eval::{evaluate_detailed_with, path_cells, Scratch};
use crate::params::params;
use crate::tt::{prefetch, Bound, Table};
use crate::pieces::SHAPE_COUNT;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const WIN: i32 = 1_000_000;
pub const WIN_THRESHOLD: i32 = WIN - 1000;
const MAX_PLY: usize = 64;
/// Coût estimé d'une itération, en multiple de la précédente.
const GROWTH: u32 = 5;
/// Entrées du cache d'évaluation d'un fil (puissance de deux, ~1 Mo).
const EVAL_CACHE: usize = 1 << 16;
/// Coups choisis un à un avant de trier le reste.
const LAZY_PICKS: usize = 3;

struct Zobrist {
    cells: [[u64; 81]; 2],
    inventory: [[[u64; 3]; SHAPE_COUNT]; 2],
    white_to_move: u64,
}

fn zobrist() -> &'static Zobrist {
    static Z: OnceLock<Zobrist> = OnceLock::new();
    Z.get_or_init(|| {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            // splitmix64
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Zobrist {
            cells: std::array::from_fn(|_| std::array::from_fn(|_| next())),
            inventory: std::array::from_fn(|_| std::array::from_fn(|_| std::array::from_fn(|_| next()))),
            white_to_move: next(),
        }
    })
}

pub fn hash(pos: &Position) -> u64 {
    let z = zobrist();
    let mut h = 0;
    for p in 0..2 {
        let mut bits = pos.cells[p];
        while bits != 0 {
            h ^= z.cells[p][bits.trailing_zeros() as usize];
            bits &= bits - 1;
        }
        for s in 0..SHAPE_COUNT {
            h ^= z.inventory[p][s][pos.inventory[p][s] as usize];
        }
    }
    if pos.active == Player::White {
        h ^= z.white_to_move;
    }
    h
}

/// Empreinte après le coup `m`, calculée sans le jouer, en supposant que le
/// trait passe à l'adversaire (le seul cas où elle diffère : un tour passé).
fn key_after_move(h: u64, pos: &Position, m: Move) -> u64 {
    let z = zobrist();
    let p = pos.active as usize;
    let mut h = h ^ z.white_to_move;
    let mut placed = pos.legal_move_cells(m);
    while placed != 0 {
        h ^= z.cells[p][placed.trailing_zeros() as usize];
        placed &= placed - 1;
    }
    let s = m.shape as usize;
    let n = pos.inventory[p][s] as usize;
    h ^ z.inventory[p][s][n] ^ z.inventory[p][s][n - 1]
}

/// Empreinte de `after`, déduite de celle de `before` par la seule pièce posée.
fn hash_after(h: u64, before: &Position, after: &Position, m: Move) -> u64 {
    let z = zobrist();
    let p = before.active as usize;
    let mut h = h;
    let mut placed = after.cells[p] ^ before.cells[p];
    while placed != 0 {
        h ^= z.cells[p][placed.trailing_zeros() as usize];
        placed &= placed - 1;
    }
    let s = m.shape as usize;
    h ^= z.inventory[p][s][before.inventory[p][s] as usize] ^ z.inventory[p][s][after.inventory[p][s] as usize];
    if before.active != after.active {
        h ^= z.white_to_move;
    }
    h
}

pub struct Search {
    table: Arc<Table>,
    killers: [[Option<Move>; 2]; MAX_PLY],
    history: Vec<i32>,
    deadline: Instant,
    pub nodes: u64,
    aborted: bool,
    /// Une feuille a été coupée par la profondeur pendant l'itération courante.
    depth_cut: bool,
    /// La position à ce niveau vient d'un coup nul (pas deux de suite).
    nulled: [bool; MAX_PLY],
    move_lists: Vec<Vec<Move>>,
    key_lists: Vec<Vec<(u64, Move)>>,
    scratch: Scratch,
    /// Budget en positions examinées (0 : aucun), pour des bancs dont le
    /// résultat ne dépend pas de la vitesse du cœur qui les exécute.
    pub node_limit: u64,
    /// Profondeur maximale, pour une recherche reproductible (banc de vitesse) :
    /// la recherche va jusque-là quel que soit le temps.
    pub depth_limit: Option<u32>,
    /// Fils de recherche (Lazy SMP) : les fils auxiliaires cherchent la même
    /// position en partageant la table, et s'arrêtent quand le principal a fini.
    pub threads: usize,
    /// Décalage de profondeur d'un fil auxiliaire, pour qu'il ne refasse pas
    /// exactement le travail du principal.
    depth_offset: u32,
    /// Signal d'arrêt partagé avec les fils auxiliaires.
    stop: Arc<AtomicBool>,
    /// Annulation venue de l'extérieur (fin de la réflexion pendant le tour
    /// adverse) : jamais remise à zéro par la recherche elle-même.
    cancel: Arc<AtomicBool>,
    /// Cases des plus courts chemins calculées au niveau parent, par ply.
    inherited: [u128; MAX_PLY],
    /// Coup exclu à chaque ply (recherche de singularité).
    excluded: [Option<Move>; MAX_PLY],
    /// Coup joué pour arriver à chaque ply (posé par le parent avant la descente).
    played: [Option<Move>; MAX_PLY],
    /// Coup réponse : pour chaque coup adverse, la réplique qui a le plus
    /// récemment provoqué une coupure.
    counter: Vec<Option<Move>>,
    /// Évaluations déjà calculées, à correspondance directe (clé, résultat) :
    /// les transpositions sont nombreuses dans ce jeu, où deux poses dans des
    /// colonnes différentes commutent.
    eval_cache: Vec<(u64, (i32, i32, i32))>,
    /// Fils auxiliaires, gardés d'une recherche à l'autre.
    helpers: Vec<Search>,
}

#[derive(Clone, Copy, Debug)]
pub struct Report {
    pub best: Move,
    pub score: i32,
    pub depth: u32,
    pub nodes: u64,
    /// La valeur est celle du jeu parfait, pas une estimation.
    pub exact: bool,
}

fn move_index(m: Move) -> usize {
    (m.shape as usize * 8 + m.orient as usize) * 9 + m.column as usize
}

fn remaining_plies(pos: &Position) -> u32 {
    pos.inventory.iter().flatten().map(|&c| c as u32).sum()
}

/// Score d'une partie finie, vu par le joueur qui vient de poser.
fn terminal(outcome: Outcome, mover: Player, ply: usize) -> i32 {
    match outcome.winner() {
        None => 0,
        Some(p) if p == mover => WIN - ply as i32,
        Some(_) => -(WIN - ply as i32),
    }
}

impl Search {
    pub fn new() -> Search {
        Search::with_table(Arc::new(Table::new(params().tt_mb)))
    }

    /// Recherche qui partage une table de transposition existante (serveur :
    /// une seule table pour toutes les requêtes et la réflexion).
    pub fn with_table(table: Arc<Table>) -> Search {
        Search {
            table,
            killers: [[None; 2]; MAX_PLY],
            history: vec![0; SHAPE_COUNT * 8 * 9],
            deadline: Instant::now(),
            nodes: 0,
            aborted: false,
            depth_cut: false,
            depth_limit: None,
            node_limit: params().node_limit,
            threads: params().search_threads,
            depth_offset: 0,
            stop: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            inherited: [0; MAX_PLY],
            played: [None; MAX_PLY],
            excluded: [None; MAX_PLY],
            nulled: [false; MAX_PLY],
            move_lists: (0..MAX_PLY).map(|_| Vec::with_capacity(128)).collect(),
            key_lists: (0..MAX_PLY).map(|_| Vec::with_capacity(128)).collect(),
            scratch: Scratch::new(),
            counter: vec![None; SHAPE_COUNT * 8 * 9],
            eval_cache: vec![(0, (0, 0, 0)); EVAL_CACHE],
            helpers: Vec::new(),
        }
    }

    /// Table de transposition de cette recherche, à partager.
    pub fn table(&self) -> Arc<Table> {
        Arc::clone(&self.table)
    }

    /// Signal d'annulation : le poser interrompt la recherche en cours au plus
    /// vite ; il faut le remettre à faux avant la recherche suivante.
    pub fn cancel_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Réflexion pendant le tour adverse : on cherche la position où
    /// l'adversaire a le trait, de son point de vue, jusqu'à être arrêté. Ses
    /// réponses, les plus dangereuses d'abord, et nos répliques remplissent la
    /// table : quel que soit son coup, la recherche suivante y trouve une partie
    /// du travail déjà fait.
    pub fn ponder(&mut self, pos: &Position, limit: Duration) {
        if pos.has_legal_move(pos.active) {
            self.best_move(pos, limit);
        }
    }

    /// Évaluation d'une position, relue dans le cache si elle y est déjà.
    fn cached_evaluation(&mut self, pos: &Position, key: u64) -> (i32, i32, i32) {
        let slot = &mut self.eval_cache[key as usize & (EVAL_CACHE - 1)];
        if slot.0 == key && key != 0 {
            return slot.1;
        }
        let result = evaluate_detailed_with(pos, &mut self.scratch);
        *slot = (key, result);
        result
    }

    /// Fil auxiliaire : même table et même signal d'arrêt, tueurs et historique propres.
    fn helper(&self, index: usize) -> Search {
        Search {
            table: Arc::clone(&self.table),
            killers: [[None; 2]; MAX_PLY],
            history: vec![0; SHAPE_COUNT * 8 * 9],
            deadline: self.deadline,
            nodes: 0,
            aborted: false,
            depth_cut: false,
            depth_limit: self.depth_limit,
            node_limit: 0,
            threads: 1,
            depth_offset: 1 + ((index - 1) % 2) as u32,
            stop: Arc::clone(&self.stop),
            cancel: Arc::clone(&self.cancel),
            inherited: [0; MAX_PLY],
            played: [None; MAX_PLY],
            excluded: [None; MAX_PLY],
            nulled: [false; MAX_PLY],
            move_lists: (0..MAX_PLY).map(|_| Vec::with_capacity(128)).collect(),
            key_lists: (0..MAX_PLY).map(|_| Vec::with_capacity(128)).collect(),
            scratch: Scratch::new(),
            counter: vec![None; SHAPE_COUNT * 8 * 9],
            eval_cache: vec![(0, (0, 0, 0)); EVAL_CACHE],
            helpers: Vec::new(),
        }
    }

    /// Ordre d'examen : coup de la table, tueurs, puis coups qui touchent les
    /// plus courts chemins (`paths`, s'il est fourni), puis historique.
    fn order(&self, pos: &Position, moves: &[Move], keys: &mut Vec<(u64, Move)>, tt_move: Option<Move>, ply: usize, paths: u128, counter: Option<Move>) {
        let killers = self.killers[ply.min(MAX_PLY - 1)];
        // Rang, puis cases de chemin et historique décroissants ; égalités : ordre d'origine.
        keys.clear();
        for (i, &m) in moves.iter().enumerate() {
            let (class, on_path, history) = if Some(m) == tt_move {
                (0, 0, 0)
            } else if Some(m) == killers[0] {
                (1, 0, 0)
            } else if Some(m) == killers[1] {
                (2, 0, 0)
            } else if Some(m) == counter {
                (3, 0, 0)
            } else {
                let on_path = if paths != 0 { (pos.legal_move_cells(m) & paths).count_ones() as u64 } else { 0 };
                (4, on_path, self.history[move_index(m)] as i64)
            };
            let key = (class << 50) | ((81 - on_path) << 40) | ((1i64 << 31) - history) as u64;
            keys.push(((key << 7) | i as u64, m));
        }
    }

    fn negamax(&mut self, pos: &Position, key: u64, depth: u32, mut alpha: i32, beta: i32, ply: usize) -> i32 {
        self.nodes += 1;
        if self.nodes & 1023 == 0
            && (Instant::now() >= self.deadline
                || self.stop.load(Relaxed)
                || self.cancel.load(Relaxed)
                || (self.node_limit > 0 && self.nodes >= self.node_limit))
        {
            self.aborted = true;
        }
        if self.aborted {
            return 0;
        }

        let remaining = remaining_plies(pos);
        let excluded = self.excluded[ply.min(MAX_PLY - 1)];
        let entry = self.table.probe(key);
        let tt_move = entry.and_then(|e| e.best);
        if let (Some(e), None) = (entry, excluded) {
            if e.depth as u32 >= depth.min(remaining) {
                let usable = match e.bound {
                    Bound::Exact => true,
                    Bound::Lower => e.score >= beta,
                    Bound::Upper => e.score <= alpha,
                };
                if usable {
                    if (e.depth as u32) < remaining {
                        self.depth_cut = true;
                    }
                    return e.score;
                }
            }
        }
        if depth == 0 {
            self.depth_cut = true;
            let (score, my_near, their_near) = self.cached_evaluation(pos, key);
            let tuning = params();
            if tuning.threat_dist > 0 && ply < MAX_PLY - 2 {
                // Une pièce couvre au plus quatre cases : au-delà, pas de gain en un
                // coup. Le seuil réglable arbitre entre précision et coût.
                let near = tuning.threat_dist;
                if my_near <= near && pos.has_winning_move(pos.active) {
                    return WIN - (ply as i32 + 1);
                }
                // L'adversaire menace de gagner au coup suivant : on ne juge pas
                // la position avant d'avoir cherché une parade.
                if their_near <= near && pos.has_winning_move(pos.active.other()) {
                    return self.negamax(pos, key, 1, alpha, beta, ply);
                }
            }
            return score;
        }

        let tuning_np = params();
        let pruning = beta - alpha == 1 && excluded.is_none() && depth < remaining && beta.abs() < WIN_THRESHOLD && ply > 0;
        // Élagage par l'évaluation statique : tout près des feuilles, une
        // position nettement au-dessus de la fenêtre n'est pas cherchée.
        if pruning && depth <= tuning_np.rfp_depth {
            let (stat, _, _) = self.cached_evaluation(pos, key);
            if stat - tuning_np.rfp_margin * depth as i32 >= beta {
                self.depth_cut = true;
                return stat;
            }
        }
        // Coup nul : on laisse l'adversaire jouer deux fois ; si la position
        // tient encore au-dessus de la fenêtre, un vrai coup la tiendra aussi.
        if pruning
            && tuning_np.null_depth > 0
            && depth >= tuning_np.null_depth
            && !self.nulled[ply.min(MAX_PLY - 1)]
            && ply + 1 < MAX_PLY
            && pos.occupied().count_ones() < tuning_np.null_max_fill
            && pos.has_legal_move(pos.active.other())
            && self.cached_evaluation(pos, key).0 >= beta
        {
            let mut child = pos.clone();
            child.active = pos.active.other();
            let child_key = key ^ zobrist().white_to_move;
            self.nulled[ply + 1] = true;
            self.played[ply + 1] = None;
            let reduced = (depth - 1).saturating_sub(tuning_np.null_r);
            let score = -self.negamax(&child, child_key, reduced, -beta, -beta + 1, ply + 1);
            self.nulled[ply + 1] = false;
            if self.aborted {
                return 0;
            }
            if score >= beta {
                self.depth_cut = true;
                return beta;
            }
        }

        // ProbCut : hors de la variante principale, une recherche réduite qui
        // dépasse nettement la fenêtre prédit la coupure de la recherche complète.
        let tuning_pc = params();
        if tuning_pc.probcut_depth > 0
            && depth >= tuning_pc.probcut_depth
            && beta - alpha == 1
            && excluded.is_none()
            && depth < remaining
            && beta.abs() < WIN_THRESHOLD
        {
            let reduced = depth - tuning_pc.probcut_reduction.min(depth - 1);
            let margin = tuning_pc.probcut_margin;
            let high = beta + margin;
            if self.negamax(pos, key, reduced, high - 1, high, ply) >= high && !self.aborted {
                return beta;
            }
            let low = alpha - margin;
            if !self.aborted && self.negamax(pos, key, reduced, low, low + 1, ply) <= low && !self.aborted {
                return alpha;
            }
            if self.aborted {
                return 0;
            }
        }
        let alpha_start = alpha;
        let mut moves = std::mem::take(&mut self.move_lists[ply.min(MAX_PLY - 1)]);
        pos.legal_moves_into(pos.active, &mut moves);
        let tuning = params();
        let paths = if tuning.order_min_depth > 0 && depth >= tuning.order_min_depth {
            path_cells(pos, 0) | path_cells(pos, 1)
        } else if tuning.inherit_paths {
            // Plus près des feuilles, on réutilise les chemins du parent, moins
            // les cases qu'il a remplies depuis : un tri presque gratuit.
            self.inherited[ply.min(MAX_PLY - 1)] & !pos.occupied()
        } else {
            0
        };
        if ply + 1 < MAX_PLY {
            self.inherited[ply + 1] = paths;
        }
        // Réplique qui a réfuté le coup précédent ailleurs dans l'arbre.
        let previous = self.played[ply.min(MAX_PLY - 1)];
        let counter = if tuning.countermove { previous.and_then(|p| self.counter[move_index(p)]) } else { None };
        let mut keys = std::mem::take(&mut self.key_lists[ply.min(MAX_PLY - 1)]);
        self.order(pos, &moves, &mut keys, tt_move, ply, paths, counter);
        let mut best_score = -WIN - 1;
        let mut best_move = None;
        let tuning = params();
        let killers = self.killers[ply.min(MAX_PLY - 1)];
        // Extension des coups uniques : si la table propose un coup nettement
        // meilleur que tous les autres (recherche réduite qui l'exclut), ce coup
        // est cherché un niveau plus profond.
        let mut singular = false;
        if tuning.singular_depth > 0 && depth >= tuning.singular_depth && excluded.is_none() && depth < remaining {
            if let (Some(e), Some(best)) = (entry, tt_move) {
                if e.depth as u32 + 3 >= depth && e.bound != Bound::Upper && e.score.abs() < WIN_THRESHOLD {
                    let singular_beta = e.score - tuning.singular_margin;
                    self.excluded[ply] = Some(best);
                    let others = self.negamax(pos, key, (depth - 1) / 2, singular_beta - 1, singular_beta, ply);
                    self.excluded[ply] = None;
                    if self.aborted {
                        return 0;
                    }
                    singular = others < singular_beta;
                }
            }
        }
        if depth == 1 {
            self.scratch.set_parent(pos.cells[pos.active as usize]);
        }
        for i in 0..keys.len() {
            // Les premiers coups sont choisis un à un : une coupure arrive
            // souvent avant qu'il faille trier le reste.
            if i < LAZY_PICKS {
                let mut best = i;
                for j in i + 1..keys.len() {
                    if keys[j].0 < keys[best].0 {
                        best = j;
                    }
                }
                keys.swap(i, best);
            } else if i == LAZY_PICKS {
                keys[i..].sort_unstable_by_key(|k| k.0);
            }
            let m = keys[i].1;
            if Some(m) == excluded {
                continue;
            }
            // Profondeur de l'enfant : un cran de plus pour un coup unique.
            let depth = if singular && Some(m) == tt_move { depth + 1 } else { depth };
            let quiet = Some(m) != tt_move && !killers.contains(&Some(m)) && Some(m) != counter;
            // Élagage des coups tardifs : tout près des feuilles, les coups
            // classés loin ne sont pas examinés du tout. Jamais sur une position
            // que la recherche peut résoudre exactement.
            if tuning.lmp_base > 0
                && quiet
                && depth <= 2
                && depth < remaining
                && best_score > -WIN_THRESHOLD
                && i >= tuning.lmp_base * depth as usize
            {
                self.depth_cut = true;
                continue;
            }
            // La clé de l'enfant, en supposant que l'adversaire ait le trait :
            // sa table et son évaluation se chargent pendant qu'on joue le coup.
            let guess = key_after_move(key, pos, m);
            self.table.prefetch(guess);
            prefetch(&self.eval_cache[guess as usize & (EVAL_CACHE - 1)] as *const _ as *const u8);
            let mut child = pos.clone();
            let after = child.play(m);
            let child_key = hash_after(key, pos, &child, m);
            if ply + 1 < MAX_PLY {
                self.played[ply + 1] = Some(m);
            }
            let score = match after {
                After::Finished(o) => terminal(o, pos.active, ply + 1),
                After::Pass => self.negamax(&child, child_key, depth - 1, alpha, beta, ply + 1),
                After::Next if i == 0 => -self.negamax(&child, child_key, depth - 1, -beta, -alpha, ply + 1),
                After::Next => {
                    // Réduction des coups tardifs : un coup mal classé est d'abord
                    // cherché moins profond ; s'il surprend, on le reprend en entier.
                    let lmr = tuning;
                    let reduce = lmr.lmr_min_index > 0
                        && depth >= lmr.lmr_min_depth
                        && i >= lmr.lmr_min_index
                        && quiet
                        && !(lmr.lmr_trivial && paths != 0 && pos.move_cells(m) & paths != 0)
                        && depth < remaining;
                    let mut probe = alpha + 1;
                    if reduce {
                        let r = if lmr.lmr_log_div > 0 {
                            // Réduction logarithmique en profondeur et en rang, un
                            // cran de moins pour un coup à bon historique.
                            let base = (depth as f64).ln() * ((i + 1) as f64).ln() * 100.0 / lmr.lmr_log_div as f64;
                            let good_history = self.history[move_index(m)] > 0;
                            let r = base as u32 - (good_history && base >= 1.0) as u32;
                            r.clamp(1, depth.saturating_sub(2).max(1))
                        } else if i >= lmr.lmr_deep_index && depth > 3 {
                            2
                        } else {
                            1
                        };
                        probe = -self.negamax(&child, child_key, (depth - 1).saturating_sub(r), -alpha - 1, -alpha, ply + 1);
                    }
                    if probe > alpha {
                        probe = -self.negamax(&child, child_key, depth - 1, -alpha - 1, -alpha, ply + 1);
                    }
                    if probe > alpha && probe < beta {
                        -self.negamax(&child, child_key, depth - 1, -beta, -alpha, ply + 1)
                    } else {
                        probe
                    }
                }
            };
            if self.aborted {
                return 0;
            }
            if score > best_score {
                best_score = score;
                best_move = Some(m);
            }
            if score > alpha {
                alpha = score;
            }
            if alpha >= beta {
                let k = &mut self.killers[ply.min(MAX_PLY - 1)];
                if k[0] != Some(m) {
                    k[1] = k[0];
                    k[0] = Some(m);
                }
                self.history[move_index(m)] += (depth * depth) as i32;
                if let Some(p) = previous {
                    self.counter[move_index(p)] = Some(m);
                }
                if params().history_decay {
                    // Malus aux coups essayés avant sans provoquer de coupure.
                    for &(_, tried) in &keys[..i] {
                        self.history[move_index(tried)] -= (depth * depth) as i32;
                    }
                }
                break;
            }
        }

        let bound = if best_score <= alpha_start {
            Bound::Upper
        } else if best_score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        if excluded.is_none() {
            self.table.store(key, best_score, depth as u8, bound, best_move);
        }
        self.move_lists[ply.min(MAX_PLY - 1)] = moves;
        self.key_lists[ply.min(MAX_PLY - 1)] = keys;
        best_score
    }

    /// Meilleur coup pour le joueur au trait, dans le budget imparti, sur
    /// `threads` fils de recherche.
    pub fn best_move(&mut self, pos: &Position, budget: Duration) -> Report {
        self.stop.store(false, Relaxed);
        self.table.new_generation();
        // L'historique d'une position vaut moins pour la suivante, et un serveur
        // qui enchaîne des centaines de parties ne doit jamais le laisser croître
        // sans borne (débordement). Les tueurs d'une autre position ne valent rien.
        self.history.iter_mut().for_each(|h| *h /= 2);
        self.killers = [[None; 2]; MAX_PLY];
        if self.threads <= 1 {
            return self.iterate(pos, budget);
        }
        self.stop.store(false, Relaxed);
        self.deadline = Instant::now() + budget;
        // Les fils auxiliaires sont gardés d'un coup à l'autre : leur cache
        // d'évaluation et leur historique restent chauds.
        let mut helpers = std::mem::take(&mut self.helpers);
        while helpers.len() + 1 < self.threads {
            let index = helpers.len() + 1;
            helpers.push(self.helper(index));
        }
        helpers.truncate(self.threads - 1);
        for helper in helpers.iter_mut() {
            helper.deadline = self.deadline;
            helper.depth_limit = self.depth_limit;
            helper.history.iter_mut().for_each(|h| *h /= 2);
            helper.killers = [[None; 2]; MAX_PLY];
        }
        let report = std::thread::scope(|scope| {
            for helper in helpers.iter_mut() {
                let pos = pos.clone();
                scope.spawn(move || helper.iterate(&pos, budget));
            }
            let report = self.iterate(pos, budget);
            self.stop.store(true, Relaxed);
            report
        });
        self.helpers = helpers;
        report
    }

    /// Approfondissement itératif d'un seul fil.
    fn iterate(&mut self, pos: &Position, budget: Duration) -> Report {
        let start = Instant::now();
        self.deadline = start + budget;
        self.aborted = false;
        self.nodes = 0;
        let key = hash(pos);
        let mut moves = pos.legal_moves(pos.active);
        assert!(!moves.is_empty(), "aucun coup légal");
        let mut report = Report { best: moves[0], score: 0, depth: 0, nodes: 0, exact: false };
        let remaining = remaining_plies(pos);

        for depth in (1 + self.depth_offset)..=remaining.min(self.depth_limit.unwrap_or(u32::MAX)) {
            let iteration_start = Instant::now();
            self.depth_cut = false;
            // Fenêtre d'aspiration : on cherche d'abord autour du score de
            // l'itération précédente ; si le résultat en sort, on élargit.
            let aspiration = params().aspiration;
            let (mut low, mut high) = if aspiration > 0 && report.depth > 0 && report.score.abs() < WIN_THRESHOLD {
                (report.score - aspiration, report.score + aspiration)
            } else {
                (-WIN - 1, WIN + 1)
            };
            let best = loop {
                let best = self.root_pass(pos, key, &moves, depth, low, high);
                if self.aborted {
                    break best;
                }
                let score = best.map_or(-WIN - 1, |(_, s)| s);
                if score <= low && low > -WIN - 1 {
                    low = -WIN - 1;
                } else if score >= high && high < WIN + 1 {
                    high = WIN + 1;
                } else {
                    break best;
                }
            };
            if self.aborted {
                // Le coup précédent est examiné en premier : un coup qui l'a
                // battu dans l'itération interrompue est au moins aussi bon.
                if let Some((m, score)) = best {
                    let keep_score = params().partial_mode == 0;
                    if m != report.best && (!keep_score || score > report.score) {
                        report.best = m;
                    }
                }
                break;
            }
            let (m, score) = best.unwrap();
            report = Report { best: m, score, depth, nodes: self.nodes, exact: !self.depth_cut };
            // Le meilleur coup passe en tête de l'itération suivante.
            let i = moves.iter().position(|&x| x == m).unwrap();
            moves[..=i].rotate_right(1);
            if report.exact || score.abs() >= WIN_THRESHOLD {
                break;
            }
            // Une itération coûte plusieurs fois la précédente : ne pas entamer
            // celle qu'on ne finirait pas. Avec `soft_pct`, on l'entame tant que
            // moins de `soft_pct` % du budget est consommé : même interrompue,
            // elle peut trouver mieux que le coup de l'itération précédente.
            let last_iteration = iteration_start.elapsed();
            let soft = params().soft_pct;
            let stop = if self.node_limit > 0 {
                // Budget en positions (bancs indépendants de la vitesse du cœur).
                self.nodes * 100 > self.node_limit * soft.max(1) as u64
            } else if soft > 0 {
                start.elapsed() * 100 > budget * soft
            } else {
                start.elapsed() + last_iteration * GROWTH > budget
            };
            if self.depth_limit.is_none() && stop {
                break;
            }
        }
        report.nodes = self.nodes;
        report
    }
}

impl Search {
    /// Un passage sur les coups de la racine, dans la fenêtre `[alpha, beta]` :
    /// le meilleur coup et son score (ou une borne, si la fenêtre est dépassée).
    fn root_pass(&mut self, pos: &Position, key: u64, moves: &[Move], depth: u32, mut alpha: i32, beta: i32) -> Option<(Move, i32)> {
        let mut best: Option<(Move, i32)> = None;
        for (i, &m) in moves.iter().enumerate() {
            let mut child = pos.clone();
            let after = child.play(m);
            let child_key = hash_after(key, pos, &child, m);
            self.played[1] = Some(m);
            let score = match after {
                After::Finished(o) => terminal(o, pos.active, 1),
                After::Pass => self.negamax(&child, child_key, depth - 1, alpha, beta, 1),
                After::Next if i == 0 => -self.negamax(&child, child_key, depth - 1, -beta, -alpha, 1),
                After::Next => {
                    let probe = -self.negamax(&child, child_key, depth - 1, -alpha - 1, -alpha, 1);
                    if probe > alpha && probe < beta {
                        -self.negamax(&child, child_key, depth - 1, -beta, -alpha, 1)
                    } else {
                        probe
                    }
                }
            };
            if self.aborted {
                break;
            }
            if best.map_or(true, |(_, s)| score > s) {
                best = Some((m, score));
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                break;
            }
        }
        best
    }

    /// Valeur de jeu du joueur au trait (+1 gagnée, 0 nulle, −1 perdue), si la
    /// recherche la prouve dans le budget : résolution complète, ou suite forcée
    /// jusqu'à une fin de partie.
    pub fn solve(&mut self, pos: &Position, budget: Duration) -> Option<i8> {
        let r = self.best_move(pos, budget);
        if r.score >= WIN_THRESHOLD {
            Some(1)
        } else if r.score <= -WIN_THRESHOLD {
            Some(-1)
        } else if r.exact {
            Some(r.score.signum() as i8)
        } else {
            None
        }
    }

    /// Valeur prouvée d'un coup, pour le joueur qui le joue.
    pub fn solve_move(&mut self, pos: &Position, m: Move, budget: Duration) -> Option<i8> {
        let mut child = pos.clone();
        match child.play(m) {
            After::Finished(o) => Some(match o.winner() {
                None => 0,
                Some(p) if p == pos.active => 1,
                Some(_) => -1,
            }),
            After::Pass => self.solve(&child, budget),
            After::Next => self.solve(&child, budget).map(|v| -v),
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Search::new()
    }
}
