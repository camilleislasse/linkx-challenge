//! Formes, orientations distinctes et notation des formes.
//!
//! Repère : `x` vers la droite, `y` vers le bas. Une orientation est ramenée à
//! l'origine (minimum des `x` et des `y` à zéro). La rotation d'un quart de
//! tour horaire envoie `(x, y)` sur `(-y, x)`, le miroir `(x, y)` sur `(-x, y)`,
//! appliqué avant la rotation.

use std::sync::OnceLock;

pub const SHAPE_COUNT: usize = 7;

/// Ordre d'énumération des formes : celui de la notation.
pub const SHAPE_TOKENS: [&str; SHAPE_COUNT] = ["1", "2", "3I", "3L", "4S", "4T", "4L"];

/// Matrices de référence, lignes de haut en bas.
const BASE_SHAPES: [&[&[u8]]; SHAPE_COUNT] = [
    &[&[1]],
    &[&[1, 1]],
    &[&[1, 1, 1]],
    &[&[1, 1], &[1, 0]],
    &[&[0, 1], &[1, 1], &[1, 0]],
    &[&[1, 1, 1], &[0, 1, 0]],
    &[&[1, 1, 1], &[1, 0, 0]],
];

#[derive(Clone, Debug)]
pub struct Orientation {
    pub shape: u8,
    pub rotation: u8,
    pub flipped: bool,
    /// Cases triées par `y` puis `x`.
    pub cells: Vec<(u8, u8)>,
    pub width: u8,
    pub height: u8,
    /// Pour chaque colonne `dx` de la pièce, l'ordonnée de sa case la plus basse.
    /// Les colonnes d'une pièce sont toutes pleines de haut en bas, si bien que
    /// seule cette case doit reposer sur quelque chose.
    pub bottom: [u8; 4],
    /// Masque des cases, pièce ancrée en `(0, 0)` (bit `y * 9 + x`).
    pub mask: u128,
}

fn normalize(points: &mut Vec<(i32, i32)>) {
    let min_x = points.iter().map(|p| p.0).min().unwrap();
    let min_y = points.iter().map(|p| p.1).min().unwrap();
    for p in points.iter_mut() {
        p.0 -= min_x;
        p.1 -= min_y;
    }
    points.sort_by_key(|&(x, y)| (y, x));
}

fn make(shape: usize, rotation: u8, flipped: bool) -> Orientation {
    let mut points: Vec<(i32, i32)> = Vec::new();
    for (y, row) in BASE_SHAPES[shape].iter().enumerate() {
        for (x, &v) in row.iter().enumerate() {
            if v == 1 {
                points.push((x as i32, y as i32));
            }
        }
    }
    normalize(&mut points);
    if flipped {
        for p in points.iter_mut() {
            p.0 = -p.0;
        }
        normalize(&mut points);
    }
    for _ in 0..rotation {
        for p in points.iter_mut() {
            *p = (-p.1, p.0);
        }
        normalize(&mut points);
    }
    let cells: Vec<(u8, u8)> = points.iter().map(|&(x, y)| (x as u8, y as u8)).collect();
    let width = cells.iter().map(|c| c.0).max().unwrap() + 1;
    let height = cells.iter().map(|c| c.1).max().unwrap() + 1;
    let mut bottom = [0u8; 4];
    let mut mask = 0u128;
    for &(x, y) in &cells {
        bottom[x as usize] = bottom[x as usize].max(y);
        mask |= 1u128 << (y as u32 * 9 + x as u32);
    }
    // Invariant dont dépend la légalité par le relief : colonnes sans trou.
    for dx in 0..width {
        let ys: Vec<u8> = cells.iter().filter(|c| c.0 == dx).map(|c| c.1).collect();
        let (lo, hi) = (*ys.iter().min().unwrap(), *ys.iter().max().unwrap());
        assert_eq!((hi - lo + 1) as usize, ys.len(), "colonne trouée");
    }
    Orientation { shape: shape as u8, rotation, flipped, cells, width, height, bottom, mask }
}

/// Orientations distinctes de chaque forme, dans l'ordre canonique : rotations
/// non retournées 0 à 3, puis rotations retournées.
pub fn orientations() -> &'static [Vec<Orientation>; SHAPE_COUNT] {
    static CACHE: OnceLock<[Vec<Orientation>; SHAPE_COUNT]> = OnceLock::new();
    CACHE.get_or_init(|| {
        std::array::from_fn(|shape| {
            let mut unique: Vec<Orientation> = Vec::new();
            for flipped in [false, true] {
                for rotation in 0..4 {
                    let o = make(shape, rotation, flipped);
                    if !unique.iter().any(|u| u.cells == o.cells) {
                        unique.push(o);
                    }
                }
            }
            unique
        })
    })
}

/// Indice, dans `orientations()[shape]`, de l'orientation obtenue par
/// `flipped` puis `rotation` quarts de tour horaires.
pub fn canonical_index(shape: usize, rotation: u8, flipped: bool) -> usize {
    let target = make(shape, rotation % 4, flipped);
    orientations()[shape]
        .iter()
        .position(|o| o.cells == target.cells)
        .expect("orientation inconnue")
}

/// Orientation reflet (miroir gauche-droite) de `orientations()[shape][orient]`.
pub fn mirror_orientation(shape: usize, orient: usize) -> usize {
    let o = &orientations()[shape][orient];
    let mut cells: Vec<(u8, u8)> = o.cells.iter().map(|&(x, y)| (o.width - 1 - x, y)).collect();
    cells.sort_by_key(|&(x, y)| (y, x));
    orientations()[shape].iter().position(|m| m.cells == cells).expect("reflet introuvable")
}

pub fn shape_from_token(token: &str) -> Option<usize> {
    SHAPE_TOKENS.iter().position(|&t| t == token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombre_d_orientations_distinctes() {
        let counts: Vec<usize> = orientations().iter().map(|o| o.len()).collect();
        assert_eq!(counts, vec![1, 2, 2, 4, 4, 4, 8]);
    }
}
