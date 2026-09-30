//! Paramètres de l'évaluation, lus une fois dans l'environnement.
//!
//! Ils permettent au banc de comparer des réglages sans recompiler :
//!   LINKX_TEMPO=600 target/release/play

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// Poids d'une case de distance sur l'axe le plus proche.
    pub primary: i32,
    /// Poids d'une case de distance sur l'autre axe.
    pub secondary: i32,
    /// Poids d'une case de plus grande zone : `zone_base + zone_per_cell × cases occupées`.
    pub zone_base: i32,
    pub zone_per_cell: i32,
    /// Valeur constante du trait.
    pub tempo: i32,
    /// Poids d'une case de largeur du chemin le plus proche, et plafond de largeur.
    pub width: i32,
    pub width_cap: i32,
    /// Seuil de case suspendue (cases vides dessous) au-delà duquel elle fait
    /// mur ; 0 désactive. `hang_axes` : 1 axe vertical seul, 2 les deux axes.
    pub hang_wall: i32,
    pub hang_axes: i32,
    /// Critères candidats : écart de cases en réserve, de pièces de quatre cases
    /// en réserve, et de coups légaux (mobilité). Poids nul par défaut.
    pub reserve: i32,
    pub big_pieces: i32,
    pub mobility: i32,
    /// Écart des carrés des distances proches, menace à deux cases ou moins,
    /// écart du nombre de groupes. Poids nul par défaut.
    pub urgency: i32,
    pub imminent: i32,
    pub groups: i32,
    /// Asphyxie : écart du nombre de pièces en réserve sans aucune place légale
    /// (celles de l'adversaire moins les siennes). Poids nul par défaut.
    pub stranded: i32,
    /// Distance plafond d'un axe dans l'évaluation (un axe mort vaut 20 sinon).
    pub dead: i32,
    /// Réduction des coups tardifs : rang à partir duquel un coup est réduit
    /// (0 désactive), profondeur minimale, rang à partir duquel il l'est de deux.
    pub lmr_min_index: usize,
    pub lmr_min_depth: u32,
    pub lmr_deep_index: usize,
    /// Réduction logarithmique : r = ln(profondeur) × ln(rang) × 100 / lmr_log_div
    /// (0 : réduction de 1 ou 2 selon le rang).
    pub lmr_log_div: i32,
    /// Élagage des coups tardifs aux profondeurs 1 et 2 : on n'examine que les
    /// `lmp_base × profondeur` premiers coups (0 désactive).
    pub lmp_base: usize,
    /// Extension des coups uniques à partir de cette profondeur (0 désactive),
    /// avec une marge en points d'évaluation.
    pub singular_depth: u32,
    pub singular_margin: i32,
    /// ProbCut à partir de cette profondeur (0 désactive) : recherche réduite de
    /// `probcut_reduction` niveaux, avec une marge en points d'évaluation.
    pub probcut_depth: u32,
    pub probcut_reduction: u32,
    pub probcut_margin: i32,
    /// Coup nul à partir de cette profondeur (0 désactive) : si passer son tour
    /// et chercher `null_r` niveaux moins profond tient encore au-dessus de la
    /// fenêtre, on coupe. Seulement tant que le plateau compte moins de
    /// `null_max_fill` cases (le zugzwang apparaît en fin de partie).
    pub null_depth: u32,
    pub null_r: u32,
    pub null_max_fill: u32,
    /// Élagage par l'évaluation statique jusqu'à cette profondeur (0 désactive) :
    /// si l'évaluation dépasse la fenêtre de `rfp_margin` par niveau, on coupe.
    pub rfp_depth: u32,
    pub rfp_margin: i32,
    /// Coup réponse : la réplique qui a réfuté le coup adverse précédent passe
    /// juste après les tueurs.
    pub countermove: bool,
    /// Tri des coups par cases des plus courts chemins (0 désactive), à partir
    /// de cette profondeur restante ; `lmr_trivial` ne réduit que les coups qui
    /// ne touchent aucun chemin.
    pub order_min_depth: u32,
    pub lmr_trivial: bool,
    /// Menaces en bout de recherche : gain en un coup reconnu, et prolongement
    /// d'un coup quand l'adversaire menace de gagner, dès qu'un joueur est à au
    /// plus `threat_dist` cases de relier deux bords (0 désactive).
    pub threat_dist: i32,
    /// Poids de fin de partie des critères, dans l'ordre de `eval::FEATURE_NAMES`
    /// (variables `LINKX_<NOM>_END`) ; égaux aux poids de début par défaut.
    pub end: [i32; 13],
    /// Budget de recherche en positions examinées (0 : au temps seulement).
    pub node_limit: u64,
    /// Taille de la table de transposition, en Mo.
    pub tt_mb: usize,
    /// Fils de recherche par coup (Lazy SMP).
    pub search_threads: usize,
    /// Gestion du temps : entamer une nouvelle itération tant que moins de
    /// `soft_pct` % du budget est consommé (0 : estimation par `GROWTH`).
    pub soft_pct: u32,
    /// Itération interrompue : 0 ne garde un coup meilleur que s'il bat aussi le
    /// score de l'itération précédente ; 1 le garde dès qu'il bat le coup précédent.
    pub partial_mode: i32,
    /// Demi-largeur de la fenêtre d'aspiration à la racine (0 désactive).
    pub aspiration: i32,
    /// Sous `order_min_depth`, trier les coups avec les chemins du parent.
    pub inherit_paths: bool,
    /// Table de transposition : remplacer en priorité les entrées anciennes.
    pub tt_aging: bool,
    /// Malus d'historique aux coups essayés sans coupure (l'historique est de
    /// toute façon divisé par deux à chaque recherche).
    pub history_decay: bool,
}

/// Noms des variables d'environnement des critères, dans l'ordre de `eval::FEATURE_NAMES`.
pub const WEIGHT_NAMES: [&str; 13] = [
    "PRIMARY", "SECONDARY", "WIDTH", "ZONE_BASE", "ZONE_PER_CELL", "TEMPO", "RESERVE", "BIG_PIECES", "MOBILITY",
    "URGENCY", "IMMINENT", "GROUPS", "STRANDED",
];

fn env_i32(name: &str, default: i32) -> i32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub fn params() -> &'static Params {
    static P: OnceLock<Params> = OnceLock::new();
    P.get_or_init(|| {
        let mut p = Params {
        primary: env_i32("LINKX_PRIMARY", 2642),
        secondary: env_i32("LINKX_SECONDARY", 937),
        zone_base: env_i32("LINKX_ZONE_BASE", -241),
        zone_per_cell: env_i32("LINKX_ZONE_PER_CELL", 9),
        tempo: env_i32("LINKX_TEMPO", 1368),
        width: env_i32("LINKX_WIDTH", 583),
        width_cap: env_i32("LINKX_WIDTH_CAP", 8),
        hang_wall: env_i32("LINKX_HANG_WALL", 0),
        hang_axes: env_i32("LINKX_HANG_AXES", 1),
        dead: env_i32("LINKX_DEAD", 20),
        reserve: env_i32("LINKX_RESERVE", 117),
        big_pieces: env_i32("LINKX_BIG_PIECES", -1359),
        mobility: env_i32("LINKX_MOBILITY", 249),
        urgency: env_i32("LINKX_URGENCY", -166),
        imminent: env_i32("LINKX_IMMINENT", 13),
        groups: env_i32("LINKX_GROUPS", -657),
        stranded: env_i32("LINKX_STRANDED", 0),
        lmr_min_index: env_i32("LINKX_LMR_INDEX", 3) as usize,
        lmr_min_depth: env_i32("LINKX_LMR_DEPTH", 3) as u32,
        lmr_deep_index: env_i32("LINKX_LMR_DEEP", 12) as usize,
        lmr_log_div: env_i32("LINKX_LMR_LOG_DIV", 200),
        lmp_base: env_i32("LINKX_LMP", 0) as usize,
        countermove: env_i32("LINKX_COUNTER", 0) == 1,
        probcut_depth: env_i32("LINKX_PROBCUT", 5) as u32,
        probcut_reduction: env_i32("LINKX_PROBCUT_R", 4) as u32,
        probcut_margin: env_i32("LINKX_PROBCUT_MARGIN", 2000),
        null_depth: env_i32("LINKX_NULL", 0) as u32,
        null_r: env_i32("LINKX_NULL_R", 3) as u32,
        null_max_fill: env_i32("LINKX_NULL_FILL", 50) as u32,
        rfp_depth: env_i32("LINKX_RFP", 0) as u32,
        rfp_margin: env_i32("LINKX_RFP_MARGIN", 1500),
        singular_depth: env_i32("LINKX_SINGULAR", 0) as u32,
        singular_margin: env_i32("LINKX_SINGULAR_MARGIN", 1000),
        order_min_depth: env_i32("LINKX_ORDER_DEPTH", 2) as u32,
        lmr_trivial: env_i32("LINKX_LMR_TRIVIAL", 0) == 1,
        threat_dist: env_i32("LINKX_THREAT_DIST", 0),
        end: [0; 13],
        tt_mb: env_i32("LINKX_TT_MB", 32) as usize,
        node_limit: std::env::var("LINKX_NODES").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
        search_threads: env_i32("LINKX_SEARCH_THREADS", 1).max(1) as usize,
        soft_pct: env_i32("LINKX_SOFT_PCT", 70).max(0) as u32,
        partial_mode: env_i32("LINKX_PARTIAL", 1),
        aspiration: env_i32("LINKX_ASPIRATION", 0),
        inherit_paths: env_i32("LINKX_INHERIT", 0) == 1,
        tt_aging: env_i32("LINKX_TT_AGING", 1) == 1,
        history_decay: env_i32("LINKX_HISTORY_DECAY", 1) == 1,
        };
        let open = [
            p.primary, p.secondary, p.width, p.zone_base, p.zone_per_cell, p.tempo, p.reserve, p.big_pieces, p.mobility,
            p.urgency, p.imminent, p.groups, p.stranded,
        ];
        for (i, name) in WEIGHT_NAMES.iter().enumerate() {
            p.end[i] = env_i32(&format!("LINKX_{name}_END"), open[i]);
        }
        p
    })
}
