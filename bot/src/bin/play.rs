//! Joueur en ligne de commande, piloté par le banc de duel.
//!
//! Protocole, une ligne par coup sur l'entrée standard :
//!   <budget_ms>\t<notation>
//! Réponse sur la sortie standard :
//!   <coup>\t<commentaire>
//!
//! Un processus garde sa table de transposition d'un coup à l'autre, comme le
//! service HTTP. Avec `LINKX_BOOK=<fichier>`, il joue le coup du livre d'ouverture
//! quand la position y figure. Avec `LINKX_PONDER=1`, il réfléchit pendant le
//! tour adverse : après avoir répondu, il cherche la position où l'adversaire a
//! le trait, jusqu'à ce que la demande suivante arrive.

use engine::book::Book;
use engine::notation::{format_move, Game};
use engine::search::Search;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

/// Durée maximale d'une réflexion pendant le tour adverse.
const PONDER_LIMIT: Duration = Duration::from_secs(30);

fn main() {
    let book = Book::from_env();
    let ponder = std::env::var("LINKX_PONDER").as_deref() == Ok("1");

    // Les demandes arrivent par un fil de lecture, pour pouvoir réfléchir en attendant.
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut search = Some(Search::new());
    let mut pondering: Option<(JoinHandle<Search>, Arc<AtomicBool>)> = None;
    let stdout = io::stdout();
    for line in rx {
        // Une demande arrive : la réflexion en cours s'arrête, sa table reste.
        if let Some((handle, cancel)) = pondering.take() {
            cancel.store(true, Relaxed);
            search = Some(handle.join().expect("réflexion interrompue"));
        }
        let engine = search.as_mut().unwrap();
        engine.cancel_signal().store(false, Relaxed);

        let (budget, record) = line.split_once('\t').unwrap_or((&line, ""));
        let budget: u64 = budget.trim().parse().unwrap_or(1000);
        let mut out = stdout.lock();
        let mut played = None;
        match Game::parse(record) {
            Ok(game) if game.outcome.is_none() => {
                if let Some(m) = book.as_ref().and_then(|b| b.lookup(&game.position)) {
                    let _ = writeln!(out, "{}\tlivre", format_move(m));
                    played = Some((game, m));
                } else {
                    let r = engine.best_move(&game.position, Duration::from_millis(budget));
                    let exact = if r.exact { " exact" } else { "" };
                    let _ = writeln!(out, "{}\tscore {} prof {}{} {} nœuds", format_move(r.best), r.score, r.depth, exact, r.nodes);
                    played = Some((game, r.best));
                }
            }
            Ok(_) => {
                let _ = writeln!(out, "?\tpartie terminée");
            }
            Err(e) => {
                let _ = writeln!(out, "?\tnotation invalide : {} {}", e.index, e.reason);
            }
        }
        if out.flush().is_err() {
            return;
        }
        drop(out);

        // Réflexion pendant le tour adverse, sur la position après notre coup.
        if let (true, Some((mut game, m))) = (ponder, played) {
            if game.apply(m).is_ok() && game.outcome.is_none() {
                let mut engine = search.take().unwrap();
                let cancel = engine.cancel_signal();
                cancel.store(false, Relaxed);
                let pos = game.position.clone();
                let handle = std::thread::spawn(move || {
                    engine.ponder(&pos, PONDER_LIMIT);
                    engine
                });
                pondering = Some((handle, cancel));
            }
        }
    }
}
