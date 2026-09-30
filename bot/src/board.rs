//! Position, légalité d'une pose, connexion et départage.
//!
//! Le support intégral interdit toute case vide sous une pièce : le plateau
//! n'a donc jamais de trou, chaque colonne est une pile pleine depuis le fond.
//! Une pose se décide sur les seules hauteurs de colonnes : la pièce descend
//! jusqu'au premier contact, et elle est soutenue si et seulement si le bas de
//! **chacune** de ses colonnes touche la pile de la colonne correspondante.

use crate::pieces::{orientations, Orientation, SHAPE_COUNT};
use std::sync::OnceLock;

pub const N: usize = 9;
pub const CELLS: u32 = 81;
pub const FULL: u128 = (1u128 << CELLS) - 1;

const fn column_mask(x: u32) -> u128 {
    let mut m = 0u128;
    let mut y = 0;
    while y < 9 {
        m |= 1u128 << (y * 9 + x);
        y += 1;
    }
    m
}

pub const COL_LEFT: u128 = column_mask(0);
pub const COL_RIGHT: u128 = column_mask(8);
pub const ROW_TOP: u128 = 0x1FF;
pub const ROW_BOTTOM: u128 = 0x1FF << 72;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Player {
    Blue = 0,
    White = 1,
}

impl Player {
    pub fn other(self) -> Player {
        match self {
            Player::Blue => Player::White,
            Player::White => Player::Blue,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Player::Blue => "blue",
            Player::White => "white",
        }
    }
}

/// Un coup : une forme, une de ses orientations distinctes, une colonne d'ancrage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub shape: u8,
    pub orient: u8,
    pub column: u8,
}

impl Move {
    pub fn orientation(&self) -> &'static Orientation {
        &orientations()[self.shape as usize][self.orient as usize]
    }

    /// Le même coup vu dans un miroir gauche-droite.
    pub fn mirrored(self) -> Move {
        let orient = crate::pieces::mirror_orientation(self.shape as usize, self.orient as usize) as u8;
        let width = orientations()[self.shape as usize][orient as usize].width;
        Move { shape: self.shape, orient, column: N as u8 - self.column - width }
    }
}

/// Reflet gauche-droite d'un ensemble de cases.
pub fn mirror_cells(b: u128) -> u128 {
    let mut out = 0u128;
    for y in 0..N {
        for x in 0..N {
            if b >> (y * N + x) & 1 == 1 {
                out |= 1u128 << (y * N + (N - 1 - x));
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropError {
    HorizontalBounds,
    Overflow,
    Unsupported,
}

impl DropError {
    pub fn reason(self) -> &'static str {
        match self {
            DropError::HorizontalBounds => "horizontal-bounds",
            DropError::Overflow => "overflow",
            DropError::Unsupported => "unsupported",
        }
    }
}

/// Voisinage à huit cases d'un ensemble de cases : d'abord les voisins de
/// gauche et de droite, puis ceux du dessus et du dessous de cette bande, ce qui
/// couvre les diagonales. Quatre décalages de 128 bits au lieu de huit.
#[inline]
pub fn expand(b: u128) -> u128 {
    let h = b | ((b & !COL_RIGHT) << 1) | ((b & !COL_LEFT) >> 1);
    (h | (h << 9) | (h >> 9)) & FULL
}

/// Cases de `own` reliées à `seed` par le voisinage à huit cases.
#[inline]
pub fn flood(own: u128, seed: u128) -> u128 {
    let mut fill = seed & own;
    loop {
        let next = expand(fill) & own;
        if next == fill {
            return fill;
        }
        fill = next;
    }
}

pub fn has_connection(own: u128) -> bool {
    (own & COL_LEFT != 0 && own & COL_RIGHT != 0 && flood(own, own & COL_LEFT) & COL_RIGHT != 0)
        || (own & ROW_TOP != 0 && own & ROW_BOTTOM != 0 && flood(own, own & ROW_TOP) & ROW_BOTTOM != 0)
}

pub fn largest_zone(own: u128) -> u32 {
    let mut rest = own;
    let mut best = 0;
    while rest != 0 {
        let seed = rest & rest.wrapping_neg();
        let zone = flood(rest, seed);
        best = best.max(zone.count_ones());
        rest &= !zone;
    }
    best
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    /// Cases de chaque joueur, bit `y * 9 + x`, ligne 0 en haut.
    pub cells: [u128; 2],
    /// Nombre de cases occupées dans chaque colonne, depuis le fond.
    pub heights: [u8; N],
    /// Exemplaires restants de chaque forme, par joueur.
    pub inventory: [[u8; SHAPE_COUNT]; 2],
    pub active: Player,
}

/// Bas d'une orientation, vu comme un relief : elle tient sur la colonne `c`
/// si la hauteur de `c` laisse la place à `base` et si chaque marche entre deux
/// colonnes voisines vaut celle de la pièce.
struct Profile {
    shape: u8,
    orient: u8,
    /// Colonnes d'ancrage possibles.
    range: u16,
    base: u8,
    /// Case de `Relief::steps` de chaque marche ; `ANY` au-delà de la largeur.
    steps: [u8; 3],
}

/// Case de `Relief::steps` qui laisse tout passer.
const ANY: u8 = 8;

fn profiles() -> &'static [Profile] {
    static CACHE: OnceLock<Vec<Profile>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut all = Vec::new();
        for (shape, list) in orientations().iter().enumerate() {
            for (orient, o) in list.iter().enumerate() {
                let mut steps = [ANY; 3];
                for i in 1..o.width as usize {
                    steps[i - 1] = (o.bottom[i - 1] as i8 - o.bottom[i] as i8 + 3) as u8;
                }
                let range = (1u16 << (10 - o.width)) - 1;
                all.push(Profile { shape: shape as u8, orient: orient as u8, range, base: o.bottom[0], steps });
            }
        }
        all
    })
}

/// Colonnes par marche (`steps[d + 3]` : colonnes `c` où `h[c + 1] - h[c] = d`)
/// et par place disponible (`room[b]` : colonnes où `h[c] + b <= 8`).
struct Relief {
    /// Marches de −3 à +3, puis une case ignorée, puis `ANY`.
    steps: [u16; 9],
    room: [u16; 4],
}

impl Relief {
    fn columns(&self, p: &Profile) -> u16 {
        let s = &self.steps;
        self.room[p.base as usize] & p.range & s[p.steps[0] as usize] & (s[p.steps[1] as usize] >> 1) & (s[p.steps[2] as usize] >> 2)
    }
}

impl Position {
    pub fn new(first: Player) -> Position {
        Position { cells: [0, 0], heights: [0; N], inventory: [[2; SHAPE_COUNT]; 2], active: first }
    }

    /// La même position vue dans un miroir gauche-droite.
    pub fn mirrored(&self) -> Position {
        let mut heights = self.heights;
        heights.reverse();
        Position {
            cells: [mirror_cells(self.cells[0]), mirror_cells(self.cells[1])],
            heights,
            inventory: self.inventory,
            active: self.active,
        }
    }

    pub fn occupied(&self) -> u128 {
        self.cells[0] | self.cells[1]
    }

    /// Ordonnée de la ligne du haut de la pièce une fois posée, ou le refus.
    #[inline]
    pub fn drop_row(&self, o: &Orientation, column: u8) -> Result<i32, DropError> {
        let (col, width) = (column as usize, o.width as usize);
        if col + width > N {
            return Err(DropError::HorizontalBounds);
        }
        // Le bas de la colonne dx doit rester au-dessus de la pile : y <= 8 - h.
        let mut anchor = i32::MAX;
        for dx in 0..width {
            let limit = 8 - self.heights[col + dx] as i32 - o.bottom[dx] as i32;
            anchor = anchor.min(limit);
        }
        if anchor < 0 {
            return Err(DropError::Overflow);
        }
        for dx in 0..width {
            if anchor + o.bottom[dx] as i32 != 8 - self.heights[col + dx] as i32 {
                return Err(DropError::Unsupported);
            }
        }
        Ok(anchor)
    }

    /// Cases qu'occuperait un coup légal.
    /// Cases d'un coup légal : sans trou, la pièce s'arrête quand le bas de sa
    /// première colonne touche la pile.
    pub fn legal_move_cells(&self, m: Move) -> u128 {
        let o = m.orientation();
        let row = 8 - self.heights[m.column as usize] as u32 - o.bottom[0] as u32;
        o.mask << (row * 9 + m.column as u32)
    }

    pub fn move_cells(&self, m: Move) -> u128 {
        let o = m.orientation();
        let row = self.drop_row(o, m.column).unwrap_or(0);
        o.mask << (row as u32 * 9 + m.column as u32)
    }

    /// La pièce peut-elle être posée à cette colonne ? Version rapide de
    /// `drop_row` quand seul le oui ou le non compte : sans trou dans le plateau,
    /// la pose est légale si le bas de chaque colonne de la pièce arrive au même
    /// niveau que la pile de sa colonne, et sans dépasser le haut.
    #[inline]
    pub fn fits(&self, o: &Orientation, column: u8) -> bool {
        let c = column as usize;
        let key = self.heights[c] + o.bottom[0];
        if key > 8 {
            return false;
        }
        (1..o.width as usize).all(|dx| self.heights[c + dx] + o.bottom[dx] == key)
    }

    pub fn is_legal(&self, player: Player, m: Move) -> bool {
        self.inventory[player as usize][m.shape as usize] > 0
            && self.drop_row(m.orientation(), m.column).is_ok()
    }

    pub fn legal_moves(&self, player: Player) -> Vec<Move> {
        let mut moves = Vec::with_capacity(128);
        self.legal_moves_into(player, &mut moves);
        moves
    }

    pub fn legal_moves_into(&self, player: Player, moves: &mut Vec<Move>) {
        moves.clear();
        let relief = self.relief();
        for p in profiles() {
            if self.inventory[player as usize][p.shape as usize] == 0 {
                continue;
            }
            let mut columns = relief.columns(p);
            while columns != 0 {
                let column = columns.trailing_zeros() as u8;
                columns &= columns - 1;
                moves.push(Move { shape: p.shape, orient: p.orient, column });
            }
        }
    }

    /// Nombre de places légales de chaque forme sur le relief actuel, toutes
    /// orientations et colonnes confondues. Il ne dépend que du relief : il sert
    /// aux deux joueurs.
    pub fn placements_by_shape(&self) -> [u32; SHAPE_COUNT] {
        let relief = self.relief();
        let mut counts = [0; SHAPE_COUNT];
        for p in profiles() {
            counts[p.shape as usize] += relief.columns(p).count_ones();
        }
        counts
    }

    fn relief(&self) -> Relief {
        let h = &self.heights;
        // Marches de −3 à +3 ; les autres vont dans une case ignorée.
        const SLOT: [usize; 19] = [7, 7, 7, 7, 7, 7, 0, 1, 2, 3, 4, 5, 6, 7, 7, 7, 7, 7, 7];
        let mut steps = [0u16; 9];
        for c in 0..N - 1 {
            steps[SLOT[(h[c + 1] as i32 - h[c] as i32 + 9) as usize]] |= 1 << c;
        }
        steps[ANY as usize] = 0xFFFF;
        // Sans trou, une colonne a au moins `b + 1` cases libres si sa case de la
        // ligne `b` (depuis le haut) est vide.
        let empty = FULL & !self.occupied();
        let room = std::array::from_fn(|b| ((empty >> (9 * b)) & 0x1FF) as u16);
        Relief { steps, room }
    }

    /// Nombre de coups légaux d'un joueur, à partir des places par forme.
    pub fn count_moves_from(&self, placements: &[u32; SHAPE_COUNT], player: Player) -> u32 {
        (0..SHAPE_COUNT).filter(|&s| self.inventory[player as usize][s] > 0).map(|s| placements[s]).sum()
    }

    /// Nombre de coups légaux, sans les construire.
    pub fn count_legal_moves(&self, player: Player) -> u32 {
        self.count_moves_from(&self.placements_by_shape(), player)
    }

    /// Le joueur peut-il relier deux bords en une seule pose ?
    pub fn has_winning_move(&self, player: Player) -> bool {
        let own = self.cells[player as usize];
        self.legal_moves(player).into_iter().any(|m| has_connection(own | self.move_cells(m)))
    }

    pub fn has_legal_move(&self, player: Player) -> bool {
        let relief = self.relief();
        profiles().iter().any(|p| self.inventory[player as usize][p.shape as usize] > 0 && relief.columns(p) != 0)
    }

    /// Pose un coup **légal** du joueur au trait, sans résoudre la suite du tour.
    pub fn place(&mut self, m: Move) {
        let o = m.orientation();
        let row = 8 - self.heights[m.column as usize] as i32 - o.bottom[0] as i32;
        debug_assert_eq!(self.drop_row(o, m.column), Ok(row), "coup illégal");
        let p = self.active as usize;
        self.cells[p] |= o.mask << (row as u32 * 9 + m.column as u32);
        for dx in 0..o.width as usize {
            self.heights[m.column as usize + dx] += o.column_cells[dx];
        }
        self.inventory[p][m.shape as usize] -= 1;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Connection(Player),
    /// Plus personne ne peut jouer : plus grandes zones bleue et blanche.
    Stalemate { blue: u32, white: u32 },
}

impl Outcome {
    pub fn winner(&self) -> Option<Player> {
        match *self {
            Outcome::Connection(p) => Some(p),
            Outcome::Stalemate { blue, white } if blue > white => Some(Player::Blue),
            Outcome::Stalemate { blue, white } if white > blue => Some(Player::White),
            Outcome::Stalemate { .. } => None,
        }
    }
}

/// Ce qui suit une pose légale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Next,
    /// L'adversaire n'a aucun coup : son tour est passé, le même joueur rejoue.
    Pass,
    Finished(Outcome),
}

impl Position {
    /// Pose un coup légal puis applique la victoire, la passe forcée ou le blocage.
    pub fn play(&mut self, m: Move) -> After {
        let mover = self.active;
        self.place(m);
        if has_connection(self.cells[mover as usize]) {
            return After::Finished(Outcome::Connection(mover));
        }
        let next = mover.other();
        if self.has_legal_move(next) {
            self.active = next;
            return After::Next;
        }
        if self.has_legal_move(mover) {
            return After::Pass;
        }
        After::Finished(Outcome::Stalemate {
            blue: largest_zone(self.cells[0]),
            white: largest_zone(self.cells[1]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miroir() {
        let mut pos = Position::new(Player::Blue);
        for m in [pos.legal_moves(Player::Blue)[7], ] {
            pos.play(m);
        }
        let moves = pos.legal_moves(pos.active);
        let mirrored = pos.mirrored();
        assert_eq!(mirrored.mirrored(), pos);
        for m in moves {
            // Le reflet d'un coup légal est légal dans la position reflétée, et
            // occupe les cases reflétées.
            let r = m.mirrored();
            assert!(mirrored.is_legal(mirrored.active, r));
            assert_eq!(mirror_cells(pos.move_cells(m)), mirrored.move_cells(r));
            assert_eq!(r.mirrored(), m);
        }
    }

    #[test]
    fn places_par_profils_identiques() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut rand = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut checked = 0;
        for _ in 0..3000 {
            let mut pos = Position::new(Player::Blue);
            loop {
                let mut expected = [0u32; SHAPE_COUNT];
                for (shape, list) in orientations().iter().enumerate() {
                    for o in list {
                        for column in 0..=(N as u8 - o.width) {
                            expected[shape] += pos.fits(o, column) as u32;
                        }
                    }
                }
                assert_eq!(pos.placements_by_shape(), expected);
                for player in [Player::Blue, Player::White] {
                    let mut listed = Vec::new();
                    for (shape, list) in orientations().iter().enumerate() {
                        if pos.inventory[player as usize][shape] == 0 {
                            continue;
                        }
                        for (orient, o) in list.iter().enumerate() {
                            for column in 0..=(N as u8 - o.width) {
                                if pos.fits(o, column) {
                                    listed.push(Move { shape: shape as u8, orient: orient as u8, column });
                                }
                            }
                        }
                    }
                    assert_eq!(pos.legal_moves(player), listed);
                    assert_eq!(pos.has_legal_move(player), !listed.is_empty());
                }
                checked += 1;
                let moves = pos.legal_moves(pos.active);
                if moves.is_empty() {
                    break;
                }
                let m = moves[(rand() % moves.len() as u64) as usize];
                if !matches!(pos.play(m), After::Next | After::Pass) {
                    break;
                }
            }
        }
        assert!(checked > 30_000);
    }

    #[test]
    fn quatre_vingt_quinze_coups_sur_plateau_vide() {
        assert_eq!(Position::new(Player::Blue).legal_moves(Player::Blue).len(), 95);
    }

    #[test]
    fn diagonale_gagne() {
        let diag: u128 = (0..9).map(|i| 1u128 << (i * 9 + i)).sum();
        assert!(has_connection(diag));
        assert!(!has_connection(diag & !(1u128 << 40)));
    }
}
