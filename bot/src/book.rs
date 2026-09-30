//! Livre d'ouverture : coups calculés hors ligne par une recherche longue.
//!
//! Format, une ligne par position : `<notation>\t<coup>\t<profondeur>\t<score>`.
//! Les positions sont reconnues par leur empreinte, si bien qu'un autre ordre de
//! coups menant au même plateau et aux mêmes réserves trouve la même entrée. Une
//! position absente est aussi cherchée par son reflet gauche-droite, dont le
//! coup est alors reflété.

use crate::board::{Move, Position};
use crate::notation::{parse_move, Game};
use crate::search::hash;
use std::collections::HashMap;

pub struct Book {
    entries: HashMap<u64, Move>,
}

impl Book {
    pub fn parse(text: &str) -> Book {
        let mut entries = HashMap::new();
        for line in text.lines() {
            let mut fields = line.split('\t');
            let (Some(record), Some(token)) = (fields.next(), fields.next()) else { continue };
            let (Ok(game), Some(m)) = (Game::parse(record), parse_move(token)) else { continue };
            // Un coup du livre doit être légal dans sa position.
            if game.outcome.is_none() && game.position.is_legal(game.position.active, m) {
                entries.insert(hash(&game.position), m);
            }
        }
        Book { entries }
    }

    /// Livre désigné par la variable d'environnement `LINKX_BOOK`, s'il y en a un.
    pub fn from_env() -> Option<Book> {
        let path = std::env::var("LINKX_BOOK").ok()?;
        Some(Book::parse(&std::fs::read_to_string(path).ok()?))
    }

    pub fn lookup(&self, pos: &Position) -> Option<Move> {
        self.entries
            .get(&hash(pos))
            .copied()
            .or_else(|| self.entries.get(&hash(&pos.mirrored())).map(|m| m.mirrored()))
    }

    /// La position, ou son reflet, figure-t-elle dans le livre ?
    pub fn covers(&self, pos: &Position) -> bool {
        self.lookup(pos).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
