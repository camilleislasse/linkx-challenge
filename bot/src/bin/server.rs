//! Service de tournoi Linkx, selon le protocole de la plateforme marmelab.
//!
//! Variables d'environnement :
//! - PORT                      port d'écoute (défaut 8787)
//! - LINKX_BOT_SECRET          secret remis à l'inscription
//! - LINKX_BOT_ALLOW_UNSIGNED  "1" pour accepter les appels non signés (dev)
//! - SEARCH_BUDGET_MS          plafond de recherche (défaut 4500)
//! - RESPONSE_MARGIN_MS        réserve pour réseau et contrôle (défaut 1200)
//! - THREADS                   requêtes traitées en parallèle (défaut 8)
//! - LINKX_BOOK                livre d'ouverture (fichier), facultatif
//! - LINKX_PONDER              "0" pour ne pas réfléchir pendant le tour adverse
//! - LINKX_PONDER_MAX_ACTIVE   réflexion seulement si au plus autant de requêtes
//!                             cherchent déjà (défaut 1 ; 0 : machine au repos)
//!
//! **Réflexion pendant le tour adverse.** Toutes les requêtes partagent une même
//! table de transposition. Après avoir répondu, et si au plus une autre requête
//! cherche (deux parties en même temps : une recherche et une réflexion se
//! partagent les cœurs), le serveur cherche la position où l'adversaire a le
//! trait : ses réponses, les plus dangereuses d'abord, et nos répliques
//! remplissent la table, quel que soit le coup qu'il jouera. Toute requête qui
//! arrive, pour n'importe quelle partie, arrête cette réflexion avant de chercher.

use engine::book::Book;
use engine::notation::{format_move, Game};
use engine::board::Position;
use engine::search::Search;
use engine::tt::Table;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Request, Response, Server};

const DEFAULT_DEADLINE_MS: u64 = 6000;
const MIN_BUDGET_MS: u64 = 100;
const MAX_TIMESTAMP_AGE_S: i64 = 300;
/// La plateforme n'appelle jamais plus de deux fois à la fois ; la marge absorbe
/// la sonde, un appel de santé ou un appel rejoué, pour qu'aucune requête
/// n'attende dans la file (ce temps-là n'est pas compté dans le budget).
const DEFAULT_THREADS: u64 = 8;

/// Réflexion pendant le tour adverse : le moteur au repos, ou la réflexion en cours.
struct Ponder {
    engine: Option<Search>,
    running: Option<(JoinHandle<Search>, Arc<AtomicBool>)>,
}

/// Durée maximale d'une réflexion pendant le tour adverse.
const PONDER_LIMIT: Duration = Duration::from_secs(30);

struct Config {
    table: Arc<Table>,
    ponder: Mutex<Ponder>,
    ponder_enabled: bool,
    /// Requêtes en cours : la réflexion ne démarre que si au plus
    /// `ponder_max_active` cherchent (une recherche et une réflexion se
    /// partagent les cœurs ; une seconde requête arrête la réflexion).
    active: AtomicUsize,
    ponder_max_active: usize,
    book: Option<Book>,
    secret: Option<String>,
    allow_unsigned: bool,
    max_budget_ms: u64,
    margin_ms: u64,
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

/// HMAC-SHA256 de `<timestamp>.<corps>`, comparé en temps constant.
fn verify(config: &Config, request: &Request, body: &str) -> Result<(), &'static str> {
    let Some(secret) = &config.secret else {
        return if config.allow_unsigned { Ok(()) } else { Err("aucun secret configuré") };
    };
    let timestamp = header(request, "X-Linkx-Timestamp").ok_or("horodatage absent")?;
    let signature = header(request, "X-Linkx-Signature").ok_or("signature absente")?;
    let sent: i64 = timestamp.parse().map_err(|_| "horodatage illisible")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    if (now - sent).abs() > MAX_TIMESTAMP_AGE_S {
        return Err("horodatage trop ancien");
    }
    let digest = signature.strip_prefix("sha256=").ok_or("signature mal formée")?;
    let digest = hex::decode(digest).map_err(|_| "signature mal formée")?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.{body}").as_bytes());
    mac.verify_slice(&digest).map_err(|_| "signature invalide")
}

/// Arrête la réflexion en cours, s'il y en a une, et remet son moteur au repos.
fn stop_ponder(config: &Config) {
    let mut ponder = config.ponder.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((handle, cancel)) = ponder.running.take() {
        cancel.store(true, SeqCst);
        if let Ok(engine) = handle.join() {
            ponder.engine = Some(engine);
        }
    }
}

/// Réfléchit sur la position où l'adversaire a le trait, s'il reste des cœurs.
fn start_ponder(config: &Config, pos: Position) {
    if !config.ponder_enabled {
        return;
    }
    let mut ponder = config.ponder.lock().unwrap_or_else(|e| e.into_inner());
    if ponder.running.is_some() || config.active.load(SeqCst) > config.ponder_max_active {
        return;
    }
    let Some(mut engine) = ponder.engine.take() else { return };
    let cancel = engine.cancel_signal();
    cancel.store(false, SeqCst);
    let handle = std::thread::spawn(move || {
        engine.ponder(&pos, PONDER_LIMIT);
        engine
    });
    ponder.running = Some((handle, cancel));
}

/// Compte une requête en cours tant qu'il vit.
struct Active<'a>(&'a AtomicUsize);

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, SeqCst);
    }
}

fn reply(request: Request, status: u16, body: Value) {
    let header = Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap();
    let _ = request.respond(Response::from_string(body.to_string()).with_status_code(status).with_header(header));
}

fn handle(config: &Config, search: &mut Search, mut request: Request) {
    let started = Instant::now();
    if request.url() == "/health" {
        return reply(request, 200, json!({ "ok": true }));
    }
    if *request.method() != tiny_http::Method::Post {
        return reply(request, 405, json!({ "error": "Utiliser POST." }));
    }
    // Une requête arrive : elle est comptée avant d'arrêter la réflexion, pour
    // qu'aucune réflexion ne puisse redémarrer pendant qu'elle cherche.
    config.active.fetch_add(1, SeqCst);
    let active_guard = Active(&config.active);
    stop_ponder(config);
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        return reply(request, 400, json!({ "error": "Corps illisible." }));
    }
    if let Err(message) = verify(config, &request, &body) {
        return reply(request, 401, json!({ "error": message }));
    }
    let Ok(payload) = serde_json::from_str::<Value>(&body) else {
        return reply(request, 400, json!({ "error": "Corps JSON illisible." }));
    };
    let Some(record) = payload["record"].as_str() else {
        return reply(request, 400, json!({ "error": "Champ « record » manquant." }));
    };
    let game_id = payload["game"].as_str().unwrap_or("?").to_string();
    let deadline_ms = payload["deadline_ms"].as_u64().filter(|&d| d > 0).unwrap_or(DEFAULT_DEADLINE_MS).min(DEFAULT_DEADLINE_MS);

    let game = match Game::parse(record) {
        Ok(game) => game,
        Err(e) => return reply(request, 400, json!({ "error": "notation", "index": e.index, "reason": e.reason })),
    };
    if game.outcome.is_some() {
        return reply(request, 409, json!({ "error": "La partie est déjà terminée." }));
    }
    let active = game.position.active;
    if let Some(color) = payload["color"].as_str() {
        if color != active.name() {
            return reply(request, 409, json!({ "error": "Couleur annoncée différente du joueur au trait.", "expected": active.name() }));
        }
    }

    if let Some(m) = config.book.as_ref().and_then(|b| b.lookup(&game.position)) {
        // Le coup du livre passe par le même contrôle que celui de la recherche ;
        // s'il était refusé, on cherche normalement.
        let mut check = game.clone();
        if check.apply(m).is_ok() {
            let token = format_move(m);
            println!("{game_id} {} → {token} (livre, {} ms)", active.name(), started.elapsed().as_millis());
            reply(request, 200, json!({ "move": token }));
            drop(active_guard);
            if check.outcome.is_none() {
                start_ponder(config, check.position);
            }
            return;
        }
        eprintln!("{game_id} coup du livre refusé par le contrôle : {}", format_move(m));
    }

    let spent = started.elapsed().as_millis() as u64;
    let budget = config.max_budget_ms.min(deadline_ms.saturating_sub(spent + config.margin_ms)).max(MIN_BUDGET_MS);
    let position = game.position.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        search.best_move(&position, Duration::from_millis(budget))
    }));
    let (m, detail) = match outcome {
        Ok(r) => (r.best, format!("score {} prof {}{} {} nœuds", r.score, r.depth, if r.exact { " exact" } else { "" }, r.nodes)),
        Err(_) => {
            // Coup de secours : ne jamais perdre sur une panne du moteur.
            *search = Search::with_table(Arc::clone(&config.table));
            (position.legal_moves(active)[0], "SECOURS après panne".to_string())
        }
    };
    let token = format_move(m);
    let mut check = game.clone();
    if check.apply(m).is_err() {
        eprintln!("{game_id} coup refusé par notre propre arbitre : {token}");
        return reply(request, 500, json!({ "error": "coup refusé par le contrôle de légalité" }));
    }
    println!("{game_id} {} → {token} ({} ms, budget {budget} ms, {detail})", active.name(), started.elapsed().as_millis());
    reply(request, 200, json!({ "move": token }));
    drop(active_guard);
    if check.outcome.is_none() {
        start_ponder(config, check.position);
    }
}

fn main() {
    let port = env_u64("PORT", 8787);
    let table = Search::new().table();
    let config = Arc::new(Config {
        ponder: Mutex::new(Ponder { engine: Some(Search::with_table(Arc::clone(&table))), running: None }),
        ponder_enabled: std::env::var("LINKX_PONDER").as_deref() != Ok("0"),
        active: AtomicUsize::new(0),
        ponder_max_active: env_u64("LINKX_PONDER_MAX_ACTIVE", 1) as usize,
        table,
        book: Book::from_env(),
        secret: std::env::var("LINKX_BOT_SECRET").ok().filter(|s| !s.is_empty()),
        allow_unsigned: std::env::var("LINKX_BOT_ALLOW_UNSIGNED").as_deref() == Ok("1"),
        max_budget_ms: env_u64("SEARCH_BUDGET_MS", 4500),
        margin_ms: env_u64("RESPONSE_MARGIN_MS", 1200),
    });
    if config.secret.is_none() && !config.allow_unsigned {
        eprintln!("LINKX_BOT_SECRET absent : tous les appels seront refusés.");
    }
    let server = Arc::new(Server::http(("0.0.0.0", port as u16)).expect("port indisponible"));
    println!(
        "Linkx bot à l'écoute sur :{port} (budget {} ms, marge {} ms, réflexion {}, livre : {} positions)",
        config.max_budget_ms,
        config.margin_ms,
        if config.ponder_enabled { "oui" } else { "non" },
        config.book.as_ref().map_or(0, |b| b.len())
    );
    let workers: Vec<_> = (0..env_u64("THREADS", DEFAULT_THREADS))
        .map(|_| {
            let (server, config) = (Arc::clone(&server), Arc::clone(&config));
            std::thread::spawn(move || {
                let mut search = Search::with_table(Arc::clone(&config.table));
                for request in server.incoming_requests() {
                    handle(&config, &mut search, request);
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
}
