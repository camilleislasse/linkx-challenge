//! Notation d'une partie : lecture, validation coup par coup, écriture canonique.

use crate::board::{After, Move, Outcome, Player, Position, N};
use crate::pieces::{canonical_index, orientations, shape_from_token, SHAPE_TOKENS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Move(Move),
    Pass,
}

#[derive(Clone, Debug)]
pub struct Game {
    pub first: Player,
    pub position: Position,
    pub history: Vec<Entry>,
    pub outcome: Option<Outcome>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotationError {
    /// Rang du jeton fautif, hors indication de premier joueur.
    pub index: usize,
    pub token: String,
    pub reason: &'static str,
}

/// Lit un jeton de coup, écritures redondantes comprises.
pub fn parse_move(token: &str) -> Option<Move> {
    let t = token.to_ascii_uppercase();
    let b = t.as_bytes();
    let shape_len = match b.first()? {
        b'1' | b'2' => 1,
        b'3' | b'4' => 2,
        _ => return None,
    };
    if b.len() < shape_len {
        return None;
    }
    let shape = shape_from_token(&t[..shape_len])?;
    let mut rest = &b[shape_len..];
    let flipped = rest.first() == Some(&b'S') && rest.len() >= 2;
    if flipped {
        rest = &rest[1..];
    }
    let mut rotation = 0u8;
    if rest.len() == 3 && (rest[0] == b'R' || rest[0] == b'L') && (b'1'..=b'3').contains(&rest[1]) {
        let turns = rest[1] - b'0';
        rotation = if rest[0] == b'R' { turns } else { 4 - turns };
        rest = &rest[2..];
    }
    if rest.len() != 1 || !(b'1'..=b'9').contains(&rest[0]) {
        return None;
    }
    let column = rest[0] - b'1';
    let orient = canonical_index(shape, rotation, flipped) as u8;
    Some(Move { shape: shape as u8, orient, column })
}

/// Écrit un coup dans sa seule forme canonique.
pub fn format_move(m: Move) -> String {
    let o = &orientations()[m.shape as usize][m.orient as usize];
    let mut s = SHAPE_TOKENS[m.shape as usize].to_string();
    if o.flipped {
        s.push('s');
    }
    if o.rotation != 0 {
        s.push('r');
        s.push((b'0' + o.rotation) as char);
    }
    s.push((b'1' + m.column) as char);
    s
}

fn first_player(token: &str) -> Option<Player> {
    match token.to_lowercase().as_str() {
        "b" | "blue" => Some(Player::Blue),
        "w" | "white" => Some(Player::White),
        _ => None,
    }
}

impl Game {
    pub fn new(first: Player) -> Game {
        Game { first, position: Position::new(first), history: Vec::new(), outcome: None }
    }

    /// Joue un coup du joueur au trait ; en cas de refus, rien ne change.
    pub fn apply(&mut self, m: Move) -> Result<(), &'static str> {
        if self.outcome.is_some() {
            return Err("game-over");
        }
        let player = self.position.active;
        if self.position.inventory[player as usize][m.shape as usize] == 0 {
            return Err("exhausted");
        }
        if (m.column as usize) >= N {
            return Err("horizontal-bounds");
        }
        self.position.drop_row(m.orientation(), m.column).map_err(|e| e.reason())?;
        self.history.push(Entry::Move(m));
        match self.position.play(m) {
            After::Next => {}
            After::Pass => self.history.push(Entry::Pass),
            After::Finished(outcome) => self.outcome = Some(outcome),
        }
        Ok(())
    }

    pub fn parse(source: &str) -> Result<Game, NotationError> {
        let tokens: Vec<&str> = source
            .split(|c: char| c.is_whitespace() || c == ',' || c == '+')
            .filter(|t| !t.is_empty())
            .collect();
        let declared = tokens.first().and_then(|t| first_player(t));
        let offset = declared.is_some() as usize;
        let mut game = Game::new(declared.unwrap_or(Player::Blue));
        let mut pending_pass = false;
        for (index, &token) in tokens.iter().enumerate().skip(offset) {
            let error = |reason| NotationError { index: index - offset, token: token.to_string(), reason };
            if token == "--" {
                if !pending_pass {
                    return Err(error("unexpected-pass"));
                }
                pending_pass = false;
                continue;
            }
            if game.outcome.is_some() {
                return Err(error("game-over"));
            }
            let m = parse_move(token).ok_or_else(|| error("syntax"))?;
            game.apply(m).map_err(error)?;
            pending_pass = game.history.last() == Some(&Entry::Pass);
        }
        Ok(game)
    }

    pub fn serialize(&self) -> String {
        let mut tokens: Vec<String> = Vec::new();
        if self.first == Player::White {
            tokens.push("w".into());
        }
        for e in &self.history {
            tokens.push(match e {
                Entry::Move(m) => format_move(*m),
                Entry::Pass => "--".into(),
            });
        }
        tokens.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(token: &str) -> String {
        format_move(parse_move(token).unwrap())
    }

    #[test]
    fn normalisation_du_document() {
        assert_eq!(canon("2r23"), "23");
        assert_eq!(canon("1r27"), "17");
        assert_eq!(canon("2l13"), "2r13");
        assert_eq!(canon("4Ls5"), "4Ls5");
        assert_eq!(canon("4Ssr21"), "4Ss1");
        assert_eq!(canon("4lsR27"), "4Lsr27");
    }

    #[test]
    fn jetons_invalides() {
        for t in ["", "5", "3X1", "111", "2r1", "3I0", "4Lr41", "--1"] {
            assert!(parse_move(t).is_none(), "{t}");
        }
    }

    #[test]
    fn relecture_identique() {
        let record = "w 4Lr34 4Lr36 4Lsr35 3Lr35 4Tr21 4Lr38 2r18 2r17 3Ir17 3Ir11 3Ir17";
        let game = Game::parse(record).unwrap();
        assert_eq!(game.serialize(), record);
        assert_eq!(game.outcome.unwrap().winner(), Some(Player::White));
    }
}
