//! Table de transposition compacte, partagée entre fils de recherche.
//!
//! Une entrée tient en 16 octets : un mot de vérification (64 bits) et un mot de
//! 64 bits qui empaquette le score, la profondeur, le type de borne et le
//! meilleur coup. Les entrées vont par paquets de huit, alignés sur 128 octets :
//! un paquet occupe exactement une ligne de cache de l'Apple M4 (deux lignes
//! voisines sur x86, chargées ensemble), si bien qu'une recherche dans la table
//! ne coûte qu'un accès mémoire.
//!
//! **Partage sans verrou.** Plusieurs fils lisent et écrivent en même temps. Le
//! mot de vérification vaut `clé XOR données` : une entrée à moitié réécrite par
//! un autre fil ne redonne pas la clé, et elle est simplement ignorée.

use crate::board::Move;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bound {
    Exact,
    Lower,
    Upper,
}

#[derive(Clone, Copy, Debug)]
pub struct Probe {
    pub score: i32,
    pub depth: u8,
    pub bound: Bound,
    pub best: Option<Move>,
}

const WAYS: usize = 8;

/// Huit entrées : une ligne de cache de 128 octets.
#[repr(C, align(128))]
#[derive(Default)]
struct Bucket {
    check: [AtomicU64; WAYS],
    data: [AtomicU64; WAYS],
}

// Mot de données : score sur les bits 0-31, profondeur 32-39, borne 40-41,
// présence d'un coup 42, forme 43-45, orientation 46-48, colonne 49-52,
// génération (recherche qui l'a écrite) 53-58.
fn pack(score: i32, depth: u8, bound: Bound, best: Option<Move>) -> u64 {
    let mut d = score as u32 as u64 | (depth as u64) << 32 | (bound as u64) << 40;
    if let Some(m) = best {
        d |= 1 << 42 | (m.shape as u64) << 43 | (m.orient as u64) << 46 | (m.column as u64) << 49;
    }
    d
}

fn unpack(d: u64) -> Probe {
    let bound = match (d >> 40) & 3 {
        0 => Bound::Exact,
        1 => Bound::Lower,
        _ => Bound::Upper,
    };
    let best = (d >> 42 & 1 == 1).then(|| Move {
        shape: (d >> 43 & 7) as u8,
        orient: (d >> 46 & 7) as u8,
        column: (d >> 49 & 15) as u8,
    });
    Probe { score: d as u32 as i32, depth: (d >> 32 & 0xFF) as u8, bound, best }
}

pub struct Table {
    buckets: Vec<Bucket>,
    bits: u32,
    /// Génération courante, avancée à chaque nouvelle recherche : les entrées
    /// anciennes sont remplacées en priorité quand `aging` est actif.
    generation: AtomicU64,
    aging: bool,
}

impl Table {
    /// Table d'environ `megabytes` Mo (arrondi à une puissance de deux de paquets).
    pub fn new(megabytes: usize) -> Table {
        let wanted = ((megabytes.max(1) << 20) / std::mem::size_of::<Bucket>()).max(1);
        let buckets = 1usize << wanted.ilog2();
        Table {
            buckets: (0..buckets).map(|_| Bucket::default()).collect(),
            bits: buckets.trailing_zeros(),
            generation: AtomicU64::new(0),
            aging: crate::params::params().tt_aging,
        }
    }

    /// Nouvelle recherche : les entrées écrites jusqu'ici vieillissent d'un cran.
    pub fn new_generation(&self) {
        self.generation.fetch_add(1, Relaxed);
    }

    fn bucket(&self, key: u64) -> &Bucket {
        let index = if self.bits == 0 { 0 } else { (key >> (64 - self.bits)) as usize };
        &self.buckets[index]
    }

    /// Entrée `i` du paquet, et la clé qu'elle porte.
    fn read(b: &Bucket, i: usize) -> (u64, u64) {
        let data = b.data[i].load(Relaxed);
        (b.check[i].load(Relaxed) ^ data, data)
    }

    /// Demande au processeur de charger le paquet de `key` en avance.
    pub fn prefetch(&self, key: u64) {
        prefetch(self.bucket(key) as *const Bucket as *const u8);
    }

    pub fn probe(&self, key: u64) -> Option<Probe> {
        let b = self.bucket(key);
        (0..WAYS).map(|i| Table::read(b, i)).find(|&(k, _)| k == key).map(|(_, d)| unpack(d))
    }

    /// Range une entrée : à la place de la même position, sinon d'une place
    /// vide, sinon de l'entrée la moins profonde du paquet.
    pub fn store(&self, key: u64, score: i32, depth: u8, bound: Bound, best: Option<Move>) {
        let b = self.bucket(key);
        let entries: [(u64, u64); WAYS] = std::array::from_fn(|i| Table::read(b, i));
        let slot = (0..WAYS)
            .find(|&i| entries[i].0 == key)
            .or_else(|| (0..WAYS).find(|&i| entries[i] == (0, 0)))
            .unwrap_or_else(|| {
                let now = self.generation.load(Relaxed) & 63;
                (0..WAYS)
                    .min_by_key(|&i| {
                        let depth = (entries[i].1 >> 32 & 0xFF) as i64;
                        let age = if self.aging { ((now + 64 - (entries[i].1 >> 53 & 63)) & 63) as i64 } else { 0 };
                        depth - 8 * age
                    })
                    .unwrap()
            });
        // Même position déjà rangée plus profond, dans cette recherche : on la garde,
        // sauf si la nouvelle valeur est exacte ou presque aussi profonde.
        if entries[slot].0 == key {
            let old_depth = (entries[slot].1 >> 32 & 0xFF) as u8;
            // Une preuve (mode preuve) n'est remplacée que par une autre preuve.
            if old_depth == crate::search::PROOF_DEPTH && depth < old_depth {
                return;
            }
            let same_generation = (entries[slot].1 >> 53 & 63) == (self.generation.load(Relaxed) & 63);
            if same_generation && bound != Bound::Exact && depth + 2 < old_depth {
                return;
            }
        }
        // Une entrée sans coup garde le coup déjà connu pour la position.
        let best = best.or_else(|| (entries[slot].0 == key).then(|| unpack(entries[slot].1).best).flatten());
        let data = pack(score, depth, bound, best) | (self.generation.load(Relaxed) & 63) << 53;
        b.data[slot].store(data, Relaxed);
        b.check[slot].store(key ^ data, Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empaquetage_reversible() {
        let m = Move { shape: 6, orient: 7, column: 8 };
        for (score, depth, bound, best) in
            [(-999_990, 12, Bound::Lower, Some(m)), (1234, 0, Bound::Exact, None), (-5, 255, Bound::Upper, Some(m))]
        {
            let p = unpack(pack(score, depth, bound, best));
            assert_eq!((p.score, p.depth, p.bound, p.best), (score, depth, bound, best));
        }
        assert_eq!(std::mem::size_of::<Bucket>(), 128);
    }

    #[test]
    fn rangement_et_lecture() {
        let t = Table::new(1);
        let m = Move { shape: 2, orient: 1, column: 4 };
        t.store(0xDEAD_BEEF, 42, 5, Bound::Exact, Some(m));
        let p = t.probe(0xDEAD_BEEF).unwrap();
        assert_eq!((p.score, p.depth, p.best), (42, 5, Some(m)));
        assert!(t.probe(0xBEEF_DEAD).is_none());
    }
}

/// Charge en avance les 128 octets à partir de `p` (un paquet, ou une entrée du
/// cache d'évaluation) : simple indication, sans effet sur le résultat.
#[inline(always)]
pub fn prefetch(p: *const u8) {
    #[cfg(target_arch = "aarch64")]
    // SAFETY : une indication de préchargement ne lit ni n'écrit la mémoire.
    unsafe {
        std::arch::asm!("prfm pldl1keep, [{0}]", "prfm pldl1keep, [{0}, #64]", in(reg) p, options(nostack, readonly, preserves_flags));
    }
    #[cfg(target_arch = "x86_64")]
    // SAFETY : une indication de préchargement ne lit ni n'écrit la mémoire.
    unsafe {
        use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
        _mm_prefetch(p as *const i8, _MM_HINT_T0);
        _mm_prefetch(p.wrapping_add(64) as *const i8, _MM_HINT_T0);
    }
}
