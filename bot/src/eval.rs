//! Évaluation d'une position, du point de vue du joueur au trait.
//!
//! **Distance à la victoire.** Pour chaque joueur et chaque paire de bords, le
//! nombre minimal de cases vides à conquérir pour relier les deux bords : ses
//! propres cases sont gratuites, celles de l'adversaire infranchissables,
//! voisinage à huit cases. L'axe le plus proche domine (un seul suffit pour
//! gagner), le second départage.
//!
//! **Largeur.** À distance égale, un chemin qu'on peut emprunter de plusieurs
//! façons vaut mieux qu'un chemin unique, que l'adversaire coupe d'une pièce. La
//! largeur compte les cases vides situées sur au moins un plus court chemin de
//! l'axe le plus proche, plafonnée.
//!
//! **Cases suspendues.** Une case vide ayant au moins `hang_wall` cases vides
//! sous elle ne s'atteint qu'une fois sa colonne comblée jusque-là, et celui qui
//! la comble l'offre à l'adversaire : pour l'axe vertical (et l'horizontal si
//! `hang_axes` vaut 2), elle compte comme un mur.
//!
//! Un chemin qui demande plus de cases que la réserve n'en contient encore est
//! mort : on ne le compte plus. Quand plus personne ne peut relier deux bords,
//! la partie se décide au blocage, par la plus grande zone ; son poids croît donc
//! avec le remplissage du plateau.

use crate::board::{expand, flood, Position, COL_LEFT, COL_RIGHT, FULL, ROW_BOTTOM, ROW_TOP};
use crate::params::params;
use std::mem::MaybeUninit;

/// Distance d'un axe infranchissable, en cases.
pub const UNREACHABLE: i32 = 20;

/// Pièces de quatre cases (S, T, grand L) encore en réserve.
fn big_pieces(pos: &Position, player: usize) -> i32 {
    (pos.inventory[player][4] + pos.inventory[player][5] + pos.inventory[player][6]) as i32
}

/// Cases encore disponibles dans la réserve d'un joueur.
pub fn reserve_cells(pos: &Position, player: usize) -> i32 {
    const SIZES: [i32; 7] = [1, 2, 3, 3, 4, 4, 4];
    pos.inventory[player].iter().zip(SIZES).map(|(&n, size)| n as i32 * size).sum()
}

/// Composantes connexes (voisinage à huit cases) des cases d'un joueur, et leur
/// voisinage. Un chemin qui touche une composante l'atteint en entier et sans
/// coût : le parcours saute de composante en composante au lieu de les remplir
/// case par case.
pub struct Groups {
    comps: [MaybeUninit<u128>; MAX_GROUPS],
    halo: [MaybeUninit<u128>; MAX_GROUPS],
    n: usize,
    /// Cases dont les composantes sont actuellement calculées.
    own: Option<u128>,
}

/// Un joueur a au plus 42 cases, donc au plus 42 composantes.
const MAX_GROUPS: usize = 42;

impl Groups {
    pub fn new(own: u128) -> Groups {
        let mut g = Groups::empty();
        g.fill(own);
        g
    }

    fn empty() -> Groups {
        Groups { comps: [MaybeUninit::uninit(); MAX_GROUPS], halo: [MaybeUninit::uninit(); MAX_GROUPS], n: 0, own: None }
    }

    /// Entre deux coups frères, les cases du joueur qui n'a pas posé ne changent
    /// pas : ses composantes sont alors gardées.
    fn fill(&mut self, own: u128) {
        if self.own == Some(own) {
            return;
        }
        self.own = Some(own);
        self.n = 0;
        let mut rest = own;
        while rest != 0 && self.n < MAX_GROUPS {
            let seed = rest & rest.wrapping_neg();
            let comp = flood(rest, seed);
            self.comps[self.n] = MaybeUninit::new(comp);
            self.halo[self.n] = MaybeUninit::new(expand(comp));
            self.n += 1;
            rest &= !comp;
        }
    }

    fn comp(&self, i: usize) -> u128 {
        assert!(i < self.n);
        // SAFETY : les `n` premières cases sont écrites par `new`.
        unsafe { self.comps[i].assume_init() }
    }

    fn halo(&self, i: usize) -> u128 {
        assert!(i < self.n);
        // SAFETY : les `n` premières cases sont écrites par `new`.
        unsafe { self.halo[i].assume_init() }
    }

    /// Taille de la plus grande composante et nombre de composantes.
    pub fn stats(&self) -> (i32, i32) {
        let largest = (0..self.n).map(|i| self.comp(i).count_ones() as i32).max().unwrap_or(0);
        (largest, self.n as i32)
    }
}

/// Couches d'un parcours : les `len` premières sont écrites, les suivantes valent zéro.
struct Layers {
    l: [MaybeUninit<u128>; UNREACHABLE as usize],
    len: usize,
}

impl Layers {
    #[inline(always)]
    fn new() -> Layers {
        Layers { l: [MaybeUninit::uninit(); UNREACHABLE as usize], len: 0 }
    }

    fn push(&mut self, layer: u128) {
        self.l[self.len] = MaybeUninit::new(layer);
        self.len += 1;
    }

    fn get(&self, k: usize) -> u128 {
        // SAFETY : les `len` premières couches sont écrites par `push`.
        if k < self.len { unsafe { self.l[k].assume_init() } } else { 0 }
    }
}

/// Couches d'un parcours en largeur : `layers[k]` est l'ensemble des cases
/// atteignables depuis `from` en ajoutant au plus `k` cases vides. Rend la
/// distance jusqu'à `to` (ou `UNREACHABLE`).
fn layers(groups: &Groups, empty: u128, from: u128, to: u128, out: &mut Layers) -> i32 {
    layers_within(groups, empty, from, to, UNREACHABLE - 1, out)
}

/// Comme `layers`, sans chercher au-delà de `limit` cases : un chemin plus long
/// est de toute façon mort (la réserve ne suffit pas à le remplir).
fn layers_within(groups: &Groups, empty: u128, from: u128, to: u128, limit: i32, out: &mut Layers) -> i32 {
    out.len = 0;
    let mut pending: u64 = if groups.n == 64 { u64::MAX } else { (1u64 << groups.n) - 1 };
    let mut reached = 0u128;
    let mut bits = pending;
    while bits != 0 {
        let i = bits.trailing_zeros() as usize;
        bits &= bits - 1;
        if groups.comp(i) & from != 0 {
            reached |= groups.comp(i);
            pending &= !(1 << i);
        }
    }
    out.push(reached);
    if reached & to != 0 {
        return 0;
    }
    for k in 1..=limit.min(UNREACHABLE - 1) {
        let new = ((expand(reached) & empty) | (from & empty)) & !reached;
        let mut next = reached | new;
        let mut bits = pending;
        while bits != 0 {
            let i = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            if groups.halo(i) & new != 0 {
                next |= groups.comp(i);
                pending &= !(1 << i);
            }
        }
        out.push(next);
        if next & to != 0 {
            return k;
        }
        if next == reached {
            return UNREACHABLE;
        }
        reached = next;
    }
    UNREACHABLE
}

/// Nombre minimal de cases vides à ajouter à `own` pour relier `from` à `to`.
pub fn distance(own: u128, empty: u128, from: u128, to: u128) -> i32 {
    layers(&Groups::new(own), empty, from, to, &mut Layers::new())
}

/// Cases vides situées sur au moins un plus court chemin de `from` à `to`,
/// sachant que ce plus court chemin en demande `d` et que `forward` en sont les
/// couches depuis `from`. Une case vide à `k` depuis `from` (elle comprise) et à
/// `d + 1 - k` depuis `to` (elle comprise) en fait partie.
fn width(groups: &Groups, empty: u128, from: u128, to: u128, d: i32, forward: &Layers) -> u32 {
    let mut backward = Layers::new();
    layers(groups, empty, to, from, &mut backward);
    let exactly = |l: &Layers, k: i32| l.get(k as usize) & !if k > 0 { l.get(k as usize - 1) } else { 0 };
    (1..=d).map(|k| (empty & exactly(forward, k) & exactly(&backward, d + 1 - k)).count_ones()).sum()
}

/// Cases vides situées sur au moins un plus court chemin de `from` à `to`.
fn shortest_cells(groups: &Groups, empty: u128, from: u128, to: u128) -> u128 {
    let mut forward = Layers::new();
    let mut backward = Layers::new();
    let d = layers(groups, empty, from, to, &mut forward);
    if d == 0 || d >= UNREACHABLE {
        return 0;
    }
    layers(groups, empty, to, from, &mut backward);
    let exactly = |l: &Layers, k: i32| l.get(k as usize) & !if k > 0 { l.get(k as usize - 1) } else { 0 };
    (1..=d).fold(0, |acc, k| acc | (empty & exactly(&forward, k) & exactly(&backward, d + 1 - k)))
}

/// Cases vides des plus courts chemins d'un joueur, sur ses deux axes : là où
/// un coup fait avancer ce joueur, ou le freine s'il est adverse.
pub fn path_cells(pos: &Position, player: usize) -> u128 {
    let groups = Groups::new(pos.cells[player]);
    let empty = FULL & !pos.occupied();
    shortest_cells(&groups, empty, COL_LEFT, COL_RIGHT) | shortest_cells(&groups, empty, ROW_TOP, ROW_BOTTOM)
}

/// Cases vides ayant au moins `k` cases vides sous elles.
fn suspended(pos: &Position, k: i32) -> u128 {
    let mut mask = 0u128;
    for x in 0..9 {
        // Les cases vides de la colonne vont de la ligne 0 à `surface`.
        let last = 8 - pos.heights[x] as i32 - k;
        for y in 0..=last {
            mask |= 1u128 << (y as usize * 9 + x);
        }
    }
    mask
}

/// Distances de l'axe le plus proche et de l'autre, en cases, et largeur du
/// plus proche.
pub fn axes(pos: &Position, player: usize) -> (i32, i32, u32) {
    axes_with(pos, player, &Groups::new(pos.cells[player]))
}

/// `axes`, avec les composantes du joueur déjà calculées.
fn axes_with(pos: &Position, player: usize, groups: &Groups) -> (i32, i32, u32) {
    let p = params();
    let empty = FULL & !pos.occupied();
    let (empty_h, empty_v) = if p.hang_wall > 0 {
        let reachable = empty & !suspended(pos, p.hang_wall);
        (if p.hang_axes >= 2 { reachable } else { empty }, reachable)
    } else {
        (empty, empty)
    };
    let reserve = reserve_cells(pos, player);
    let alive = |d: i32| if d > reserve { UNREACHABLE } else { d };
    let mut forward_h = Layers::new();
    let mut forward_v = Layers::new();
    let h = alive(layers_within(groups, empty_h, COL_LEFT, COL_RIGHT, reserve, &mut forward_h));
    let v = alive(layers_within(groups, empty_v, ROW_TOP, ROW_BOTTOM, reserve, &mut forward_v));
    let near = h.min(v);
    let w = if p.width == 0 || near == 0 || near >= UNREACHABLE {
        0
    } else if h <= v {
        width(groups, empty_h, COL_LEFT, COL_RIGHT, near, &forward_h)
    } else {
        width(groups, empty_v, ROW_TOP, ROW_BOTTOM, near, &forward_v)
    };
    (near, h.max(v), w)
}

/// Nombre de critères de l'évaluation.
pub const F: usize = 13;

/// Noms des critères dont l'évaluation est la somme pondérée.
pub const FEATURE_NAMES: [&str; F] = [
    "primary", "secondary", "width", "zone_base", "zone_per_cell", "tempo",
    "reserve", "big_pieces", "mobility", "urgency", "imminent", "groups", "stranded",
];
/// Nombre de critères de base ; les suivants se sont ajoutés au fil des essais.
pub const ENGINE_FEATURES: usize = 6;

/// Avancement de la partie : 0 sur un plateau vide, 1 sur un plateau plein.
pub fn phase(pos: &Position) -> f64 {
    pos.occupied().count_ones() as f64 / 81.0
}

/// Poids du moteur en début de partie, dans l'ordre de `FEATURE_NAMES`.
pub fn engine_weights() -> [f64; F] {
    let p = params();
    [
        p.primary, p.secondary, p.width, p.zone_base, p.zone_per_cell, p.tempo, p.reserve, p.big_pieces, p.mobility,
        p.urgency, p.imminent, p.groups, p.stranded,
    ]
    .map(|w| w as f64)
}

/// Poids du moteur en fin de partie (égaux à ceux du début sauf réglage `_END`).
pub fn end_weights() -> [f64; F] {
    params().end.map(|w| w as f64)
}

/// Critères d'une position, du point de vue du joueur au trait, et distances de
/// l'axe le plus proche du joueur au trait et de son adversaire. `all` calcule
/// aussi les critères de poids nul, pour le réglage automatique.
/// Groupes des deux joueurs, remplis sur place à chaque évaluation.
pub struct Scratch {
    mine: Groups,
    theirs: Groups,
}

impl Scratch {
    pub fn new() -> Scratch {
        Scratch { mine: Groups::empty(), theirs: Groups::empty() }
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Scratch::new()
    }
}

fn feature_vector(pos: &Position, all: bool, scratch: &mut Scratch) -> ([f64; F], i32, i32) {
    let p = params();
    let me = pos.active as usize;
    let opp = 1 - me;
    scratch.mine.fill(pos.cells[me]);
    scratch.theirs.fill(pos.cells[opp]);
    let (my_groups, their_groups) = (&scratch.mine, &scratch.theirs);
    let (mn, mf, mw) = axes_with(pos, me, &my_groups);
    let (on, of, ow) = axes_with(pos, opp, &their_groups);
    let cap = |w: u32| w.min(p.width_cap as u32) as f64;
    let ((my_zone, my_count), (their_zone, their_count)) = (my_groups.stats(), their_groups.stats());
    let zones = (my_zone - their_zone) as f64;
    let occupied = pos.occupied().count_ones() as f64;
    let (dmn, don) = (mn.min(p.dead), on.min(p.dead));
    let stranded = if all || p.stranded != 0 || p.end[12] != 0 {
        (stranded_pieces(pos, opp) - stranded_pieces(pos, me)) as f64
    } else {
        0.0
    };
    let f = [
        (don - dmn) as f64,
        (of.min(p.dead) - mf.min(p.dead)) as f64,
        cap(mw) - cap(ow),
        zones,
        zones * occupied,
        1.0,
        (reserve_cells(pos, me) - reserve_cells(pos, opp)) as f64,
        (big_pieces(pos, me) - big_pieces(pos, opp)) as f64,
        {
            let placements = pos.placements_by_shape();
            pos.count_moves_from(&placements, pos.active) as f64 - pos.count_moves_from(&placements, pos.active.other()) as f64
        },
        (don * don - dmn * dmn) as f64,
        ((dmn <= 2) as i32 - (don <= 2) as i32) as f64,
        (my_count - their_count) as f64,
        stranded,
    ];
    (f, mn, on)
}

/// Critères d'une position (tous calculés), pour le réglage automatique.
pub fn features(pos: &Position) -> [f64; F] {
    feature_vector(pos, true, &mut Scratch::new()).0
}

/// Évaluation, et distance de l'axe le plus proche du joueur au trait et de son
/// adversaire (pour la détection des menaces).
///
/// Chaque critère a un poids de début et un poids de fin de partie, mélangés
/// selon l'avancement : l'importance d'un critère change au fil de la partie.
pub fn evaluate_detailed(pos: &Position) -> (i32, i32, i32) {
    evaluate_detailed_with(pos, &mut Scratch::new())
}

pub fn evaluate_detailed_with(pos: &Position, scratch: &mut Scratch) -> (i32, i32, i32) {
    let (f, my_near, their_near) = feature_vector(pos, false, scratch);
    let t = phase(pos);
    let (open, end) = (engine_weights(), end_weights());
    let score: f64 = (0..F).map(|i| f[i] * (open[i] * (1.0 - t) + end[i] * t)).sum();
    (score.round() as i32, my_near, their_near)
}

pub fn evaluate(pos: &Position) -> i32 {
    evaluate_detailed(pos).0
}

/// Pièces en réserve d'un joueur qui n'ont plus aucune place légale sur le
/// relief actuel : elles ne serviront que si le relief change. Quand toutes le
/// sont, le joueur passe son tour et l'adversaire rejoue.
fn stranded_pieces(pos: &Position, player: usize) -> i32 {
    let mut stranded = 0;
    for (shape, list) in crate::pieces::orientations().iter().enumerate() {
        let copies = pos.inventory[player][shape] as i32;
        if copies == 0 {
            continue;
        }
        let placeable = list
            .iter()
            .any(|o| (0..=(9 - o.width)).any(|column| pos.drop_row(o, column).is_ok()));
        if !placeable {
            stranded += copies;
        }
    }
    stranded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Player;
    use crate::notation::Game;

    /// Ancien parcours, case par case : la référence du parcours par composantes.
    fn layers_reference(own: u128, empty: u128, from: u128, to: u128, limit: i32, out: &mut [u128; UNREACHABLE as usize]) -> i32 {
        let mut reached = flood(own, own & from);
        out[0] = reached;
        if reached & to != 0 {
            return 0;
        }
        for k in 1..=limit.min(UNREACHABLE - 1) {
            let step = reached | (expand(reached) & empty) | (from & empty);
            let next = flood(own | step, step);
            out[k as usize] = next;
            if next & to != 0 {
                return k;
            }
            if next == reached {
                return UNREACHABLE;
            }
            reached = next;
        }
        UNREACHABLE
    }

    #[test]
    fn parcours_par_composantes_identique() {
        let mut state = 0x1234_5678_9ABC_DEFFu64;
        let mut rand = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut checked = 0;
        for _ in 0..400 {
            let mut pos = Position::new(Player::Blue);
            loop {
                let moves = pos.legal_moves(pos.active);
                if moves.is_empty() {
                    break;
                }
                let m = moves[(rand() % moves.len() as u64) as usize];
                if !matches!(pos.play(m), crate::board::After::Next | crate::board::After::Pass) {
                    break;
                }
                let empty = FULL & !pos.occupied();
                for player in 0..2 {
                    let own = pos.cells[player];
                    let groups = Groups::new(own);
                    for (from, to) in [(COL_LEFT, COL_RIGHT), (ROW_TOP, ROW_BOTTOM), (COL_RIGHT, COL_LEFT), (ROW_BOTTOM, ROW_TOP)] {
                        for limit in [3, 8, UNREACHABLE - 1] {
                            let (mut a, mut b) = ([0u128; UNREACHABLE as usize], Layers::new());
                            let da = layers_reference(own, empty, from, to, limit, &mut a);
                            let db = layers_within(&groups, empty, from, to, limit, &mut b);
                            assert_eq!(da, db);
                            let upto = if da >= UNREACHABLE { 0 } else { da as usize };
                            assert!((0..=upto).all(|k| a[k] == b.get(k)));
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 10_000);
    }

    #[test]
    fn les_criteres_redonnent_l_evaluation() {
        for record in ["", "b 15 3Ir13", "4Lr38 4Lr32 3Lr36 2r12 2r12 4Tr13 3Ir16 16 4Lr18 3Lr38"] {
            let pos = Game::parse(record).unwrap().position;
            let sum: f64 = features(&pos).iter().zip(engine_weights()).map(|(f, w)| f * w).sum();
            assert_eq!(sum as i32, evaluate(&pos), "{record}");
        }
    }
}
